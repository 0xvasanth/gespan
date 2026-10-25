//! Local dictation support: project vocabulary, spoken-identifier
//! post-processing and the speech helper protocol. No audio or engine
//! dependencies live here yet.

mod postprocess;
pub mod protocol;
mod vocabulary;

pub use postprocess::postprocess;
pub use vocabulary::{local_branches, pick_terms, VocabularySources, MAX_TERMS, MIN_TERM_LEN};
