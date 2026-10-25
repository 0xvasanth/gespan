//! Local dictation support: project vocabulary and spoken-identifier
//! post-processing. No audio or engine dependencies live here yet.

mod postprocess;
mod vocabulary;

pub use postprocess::postprocess;
pub use vocabulary::{local_branches, pick_terms, VocabularySources, MAX_TERMS, MIN_TERM_LEN};
