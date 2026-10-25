/// Turns 16 kHz mono samples into text. `terms` are project vocabulary the
/// engine may use to bias recognition.
pub trait SpeechEngine: Send {
    fn transcribe(&mut self, samples_16k: &[f32], terms: &[String]) -> Result<String, EngineError>;
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("speech model failed to load: {0}")]
    Load(String),
    #[error("transcription failed: {0}")]
    Transcribe(String),
}
