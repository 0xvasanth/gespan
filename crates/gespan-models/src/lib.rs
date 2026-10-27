//! Shared model downloads: resumable, SHA-256 verified, with mirror fallback.
//! Used by dictation (speech models) and, later, the memory embedding models.

mod download;

pub use download::{
    default_root, CancelToken, DownloadError, ModelFile, ModelSpec, ModelStatus, ModelStore,
    Progress,
};
