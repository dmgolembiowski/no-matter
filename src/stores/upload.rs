//! In-flight upload tracking.
//!
//! Originally `FileUploader` owned its own `RwSignal<UploadStatus>`,
//! which meant only that component could observe progress. That breaks
//! the moment you want:
//!   - the message composer to show staged attachments before posting,
//!   - a global "uploads" indicator in the title bar,
//!   - an upload to survive the user navigating away from the channel
//!     where they started it.
//!
//! Lifting upload state here makes all three trivial. The `FileUploader`
//! component now just kicks off uploads and reads from this store; the
//! Tauri progress events feed the store directly via the realtime bridge.

use std::collections::HashMap;

use leptos::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadPhase {
    Starting,
    InProgress,
    Complete,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UploadStatus {
    pub upload_id: String,
    pub channel_id: String,
    pub filename: String,
    pub bytes_sent: u64,
    pub total: u64,
    pub phase: UploadPhase,
    pub file_id: Option<String>,
    pub error: Option<String>,
}

impl UploadStatus {
    pub fn percent(&self) -> u8 {
        if self.total == 0 {
            0
        } else {
            ((self.bytes_sent.saturating_mul(100)) / self.total).min(100) as u8
        }
    }
}

#[derive(Copy, Clone)]
pub struct UploadStore(pub RwSignal<HashMap<String, UploadStatus>>);

impl UploadStore {
    pub fn new() -> Self {
        Self(RwSignal::new(HashMap::new()))
    }

    pub fn start(&self, upload_id: String, channel_id: String, filename: String) {
        self.0.update(|m| {
            m.insert(
                upload_id.clone(),
                UploadStatus {
                    upload_id,
                    channel_id,
                    filename,
                    bytes_sent: 0,
                    total: 0,
                    phase: UploadPhase::Starting,
                    file_id: None,
                    error: None,
                },
            );
        });
    }

    pub fn progress(&self, upload_id: &str, bytes_sent: u64, total: u64) {
        self.0.update(|m| {
            if let Some(s) = m.get_mut(upload_id) {
                s.bytes_sent = bytes_sent;
                s.total = total;
                s.phase = UploadPhase::InProgress;
            }
        });
    }

    pub fn complete(&self, upload_id: &str, file_id: String) {
        self.0.update(|m| {
            if let Some(s) = m.get_mut(upload_id) {
                s.phase = UploadPhase::Complete;
                s.file_id = Some(file_id);
                s.bytes_sent = s.total;
            }
        });
    }

    pub fn fail(&self, upload_id: &str, error: String) {
        self.0.update(|m| {
            if let Some(s) = m.get_mut(upload_id) {
                s.phase = UploadPhase::Failed;
                s.error = Some(error);
            }
        });
    }

    /// Drop a finished upload — call after the user posts the message
    /// containing it, or when they cancel a staged attachment.
    pub fn clear(&self, upload_id: &str) {
        self.0.update(|m| {
            m.remove(upload_id);
        });
    }

    pub fn get(&self, upload_id: String) -> Memo<Option<UploadStatus>> {
        let store = self.0;
        Memo::new(move |_| store.with(|m| m.get(&upload_id).cloned()))
    }

    /// All uploads currently being staged for a given channel — drives
    /// the composer's attachment strip.
    pub fn for_channel(&self, channel_id: String) -> Memo<Vec<UploadStatus>> {
        let store = self.0;
        Memo::new(move |_| {
            store.with(|m| {
                let mut v: Vec<_> = m
                    .values()
                    .filter(|s| s.channel_id == channel_id)
                    .cloned()
                    .collect();
                v.sort_by(|a, b| a.upload_id.cmp(&b.upload_id));
                v
            })
        })
    }

    /// Count of in-flight uploads across the app — for a global indicator.
    pub fn in_flight_count(&self) -> Memo<usize> {
        let store = self.0;
        Memo::new(move |_| {
            store.with(|m| {
                m.values()
                    .filter(|s| matches!(s.phase, UploadPhase::Starting | UploadPhase::InProgress))
                    .count()
            })
        })
    }
}

impl Default for UploadStore {
    fn default() -> Self {
        Self::new()
    }
}
