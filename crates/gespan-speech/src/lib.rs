//! Local dictation support: project vocabulary, spoken-identifier
//! post-processing, the speech helper protocol and the Parakeet engine.

pub mod capture;
pub mod catalog;
mod client;
mod engine;
mod helper;
mod parakeet;
mod postprocess;
pub mod protocol;
mod vocabulary;

pub use capture::{AudioSource, CaptureError, MicSource};
pub use client::{helper_command, RestartPolicy, SpeechClient, CRASHED, UNAVAILABLE};
pub use engine::{EngineError, SpeechEngine};
pub use helper::{
    helper_main, run_helper, run_helper_with, HelperLimits, MAX_RECORDING_SECS, PARTIAL_INTERVAL_MS,
};
pub use parakeet::ParakeetEngine;
pub use postprocess::postprocess;
pub use vocabulary::{local_branches, pick_terms, VocabularySources, MAX_TERMS, MIN_TERM_LEN};
