//! Local dictation support: project vocabulary, spoken-identifier
//! post-processing, the speech helper protocol and the Parakeet engine.

pub mod catalog;
mod engine;
mod parakeet;
mod postprocess;
pub mod protocol;
mod vocabulary;

pub use engine::{EngineError, SpeechEngine};
pub use parakeet::ParakeetEngine;
pub use postprocess::postprocess;
pub use vocabulary::{local_branches, pick_terms, VocabularySources, MAX_TERMS, MIN_TERM_LEN};
