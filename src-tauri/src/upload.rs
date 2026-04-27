//! Streaming file upload, with progress events emitted as the body is sent.
//!
//! Why streaming: large media (video, big images) shouldn't sit in WASM
//! memory. The frontend hands us a path; we open the file, wrap it in a
//! `ReaderStream`, and pipe it into `reqwest::Body::wrap_stream`. Each
//! chunk fires an `upload://progress` event keyed by a client-supplied
//! `upload_id` so multiple uploads can be in flight simultaneously.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

#[derive(Clone, Serialize)]
pub struct UploadProgress {
    pub upload_id: String,
    pub bytes_sent: u64,
    pub total: u64,
}

#[derive(Clone, Serialize)]
pub struct UploadComplete {
    pub upload_id: String,
    pub file_id: String,
}

#[derive(Deserialize)]
struct UploadResp {
    file_id: String,
}

#[tauri::command]
pub async fn upload_file(
    app: AppHandle,
    upload_id: String,
    channel_id: String,
    path: String,
    api_base: String,
    token: String,
) -> Result<String, String> {
    let metadata = tokio::fs::metadata(&path).await.map_err(|e| e.to_string())?;
    let total = metadata.len();
    let file = tokio::fs::File::open(&path).await.map_err(|e| e.to_string())?;
    let filename = std::path::Path::new(&path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();

    // Atomic running counter so the closure can be FnMut-free.
    let sent = Arc::new(AtomicU64::new(0));

    let app_for_progress = app.clone();
    let id_for_progress = upload_id.clone();
    let sent_for_progress = Arc::clone(&sent);

    let stream = tokio_util::io::ReaderStream::new(file).map(move |chunk| {
        if let Ok(ref bytes) = chunk {
            let new_sent = sent_for_progress.fetch_add(bytes.len() as u64, Ordering::Relaxed)
                + bytes.len() as u64;
            let _ = app_for_progress.emit(
                "upload://progress",
                UploadProgress {
                    upload_id: id_for_progress.clone(),
                    bytes_sent: new_sent,
                    total,
                },
            );
        }
        chunk
    });

    let body = reqwest::Body::wrap_stream(stream);
    let part = reqwest::multipart::Part::stream_with_length(body, total).file_name(filename);
    let form = reqwest::multipart::Form::new()
        .text("channel_id", channel_id)
        .part("file", part);

    let resp: UploadResp = reqwest::Client::new()
        .post(format!("{api_base}/files"))
        .bearer_auth(&token)
        .multipart(form)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;

    let _ = app.emit(
        "upload://complete",
        UploadComplete {
            upload_id,
            file_id: resp.file_id.clone(),
        },
    );

    Ok(resp.file_id)
}
