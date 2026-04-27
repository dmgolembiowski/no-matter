//! File upload + serve.
//!
//! Storage layout: every uploaded file is written verbatim to
//! `./uploads/<file_id>` (no extension — the row carries `name` and
//! `mime`). Two reasons to keep the on-disk file unnamed:
//!   - Sidesteps filesystem quirks around exotic / colliding filenames.
//!   - Decouples the storage key from the user-visible name, so a rename
//!     in the future is a metadata-only change.
//!
//! `UPLOAD_DIR` is overridable via env var for ops; default `./uploads`
//! relative to CWD. Path traversal is impossible because the filename
//! is always `<file_id>` (a ULID we generate, not user-controlled).

use std::path::PathBuf;

use axum::{
    body::Bytes,
    extract::{Multipart, Path, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::auth::{now_ms, CurrentUser};
use crate::entities::file::{self, FileStatus};
use crate::entities::prelude::*;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct UploadResp {
    pub file_id: String,
}

#[derive(Debug, Deserialize)]
pub struct GetFileMetaRequest {
    pub file_id: String,
}

#[derive(Debug, Serialize)]
pub struct FileMetaResp {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub thumb_url: Option<String>,
    /// Server-relative URL. The client prepends its known `base_url` to
    /// build a fetchable href — this lets the same row work whether
    /// the desktop app talks to localhost or a remote server.
    pub url: String,
}

fn upload_dir() -> PathBuf {
    std::env::var("UPLOAD_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("./uploads"))
}

pub async fn upload(
    State(state): State<AppState>,
    user: CurrentUser,
    mut multipart: Multipart,
) -> AppResult<Json<UploadResp>> {
    let dir = upload_dir();
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("mkdir uploads: {e}")))?;

    let mut channel_id: Option<String> = None;
    let mut filename: Option<String> = None;
    let mut mime: String = "application/octet-stream".into();
    let mut bytes_buf: Vec<Bytes> = Vec::new();
    let mut total: u64 = 0;

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("multipart: {e}")))?
    {
        let name = field.name().map(|s| s.to_string()).unwrap_or_default();
        match name.as_str() {
            "channel_id" => {
                channel_id = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("channel_id: {e}")))?,
                );
            }
            "file" => {
                if let Some(n) = field.file_name() {
                    filename = Some(n.to_string());
                }
                if let Some(m) = field.content_type() {
                    mime = m.to_string();
                }
                while let Some(chunk) = field
                    .chunk()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("chunk: {e}")))?
                {
                    total += chunk.len() as u64;
                    bytes_buf.push(chunk);
                }
            }
            _ => {
                // Drain unknown parts so the iterator keeps moving.
                let _ = field.bytes().await;
            }
        }
    }

    let _channel_id =
        channel_id.ok_or_else(|| AppError::BadRequest("missing channel_id".into()))?;
    let filename = filename.unwrap_or_else(|| "file".into());

    if bytes_buf.is_empty() {
        return Err(AppError::BadRequest("empty file".into()));
    }

    let file_id = ulid::Ulid::new().to_string();
    let storage_path = dir.join(&file_id);
    let mut f = tokio::fs::File::create(&storage_path)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("create file: {e}")))?;
    for chunk in &bytes_buf {
        f.write_all(chunk)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("write: {e}")))?;
    }
    f.flush()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("flush: {e}")))?;

    let row = file::ActiveModel {
        id: Set(file_id.clone()),
        name: Set(filename),
        mime: Set(mime),
        size: Set(total as i64),
        storage_key: Set(storage_path.to_string_lossy().to_string()),
        thumb_url: Set(None),
        // Thumb worker is future work — mark Ready immediately so
        // MediaPreview can render the original as the preview.
        status: Set(FileStatus::Ready),
        uploader_id: Set(user.id),
        created_at: Set(now_ms()),
    };
    row.insert(&state.db).await?;

    Ok(Json(UploadResp { file_id }))
}

pub async fn get_file_meta(
    State(state): State<AppState>,
    _user: CurrentUser,
    Json(req): Json<GetFileMetaRequest>,
) -> AppResult<Json<FileMetaResp>> {
    let row = Files::find_by_id(&req.file_id)
        .one(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;

    Ok(Json(FileMetaResp {
        id: row.id.clone(),
        name: row.name,
        mime: row.mime,
        size: row.size as u64,
        thumb_url: row.thumb_url,
        url: format!("/files/{}", row.id),
    }))
}

pub async fn get_file(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> AppResult<Response> {
    let row = Files::find_by_id(&id)
        .one(&state.db)
        .await?
        .ok_or(AppError::NotFound)?;

    let bytes = tokio::fs::read(&row.storage_key)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("read: {e}")))?;

    let mut resp = (StatusCode::OK, bytes).into_response();
    let headers = resp.headers_mut();
    if let Ok(v) = HeaderValue::from_str(&row.mime) {
        headers.insert(header::CONTENT_TYPE, v);
    }
    if let Ok(v) = HeaderValue::from_str(&format!("inline; filename=\"{}\"", row.name)) {
        headers.insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(resp)
}
