use crate::engine::{EngineError, SpeechEngine};
use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig};
use std::path::Path;

const SAMPLE_RATE: i32 = 16_000;

/// Parakeet TDT (NeMo transducer) running on sherpa-onnx.
pub struct ParakeetEngine {
    recognizer: OfflineRecognizer,
}

impl ParakeetEngine {
    pub fn load(model_dir: &Path) -> Result<Self, EngineError> {
        let path = |name: &str| -> Result<String, EngineError> {
            let p = model_dir.join(name);
            if !p.is_file() {
                return Err(EngineError::Load(format!("missing {name}")));
            }
            Ok(p.to_string_lossy().into_owned())
        };
        let mut config = OfflineRecognizerConfig::default();
        config.model_config.transducer = OfflineTransducerModelConfig {
            encoder: Some(path("encoder.int8.onnx")?),
            decoder: Some(path("decoder.int8.onnx")?),
            joiner: Some(path("joiner.int8.onnx")?),
        };
        config.model_config.tokens = Some(path("tokens.txt")?);
        config.model_config.model_type = Some("nemo_transducer".to_string());
        config.model_config.num_threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get().min(4))
            .try_into()
            .unwrap_or(1);
        let recognizer = OfflineRecognizer::create(&config)
            .ok_or_else(|| EngineError::Load("recognizer could not be created".to_string()))?;
        Ok(Self { recognizer })
    }
}

impl SpeechEngine for ParakeetEngine {
    fn transcribe(
        &mut self,
        samples_16k: &[f32],
        _terms: &[String],
    ) -> Result<String, EngineError> {
        if samples_16k.is_empty() {
            return Ok(String::new());
        }
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(SAMPLE_RATE, samples_16k);
        self.recognizer.decode(&stream);
        let result = stream
            .get_result()
            .ok_or_else(|| EngineError::Transcribe("no result".to_string()))?;
        Ok(result.text.trim().to_string())
    }
}
