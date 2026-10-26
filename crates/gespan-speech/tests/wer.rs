use gespan_models::ModelSpec;
use gespan_speech::{catalog, postprocess, ParakeetEngine, SpeechEngine};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Word-level Levenshtein distance over the number of reference words.
fn wer(reference: &str, hypothesis: &str) -> f64 {
    let r = words(reference);
    let h = words(hypothesis);
    let mut prev: Vec<usize> = (0..=h.len()).collect();
    for i in 1..=r.len() {
        let mut cur = vec![i; h.len() + 1];
        for j in 1..=h.len() {
            let sub = prev[j - 1] + usize::from(r[i - 1] != h[j - 1]);
            cur[j] = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
        }
        prev = cur;
    }
    prev[h.len()] as f64 / r.len().max(1) as f64
}

#[test]
fn wer_counts_substitutions_insertions_deletions() {
    assert_eq!(wer("the cat sat", "the cat sat"), 0.0);
    assert!((wer("the cat sat", "the dog sat") - 1.0 / 3.0).abs() < 1e-9);
    assert!((wer("the cat sat", "the cat sat down") - 1.0 / 3.0).abs() < 1e-9);
    assert!((wer("the cat sat", "the sat") - 1.0 / 3.0).abs() < 1e-9);
    // Case and punctuation are ignored.
    assert_eq!(wer("Hello, World.", "hello world"), 0.0);
}

fn assert_spec_consistent(spec: &ModelSpec) {
    assert!(!spec.base_urls.is_empty(), "{}", spec.id);
    for base in spec.base_urls {
        assert!(
            base.starts_with("https://") && base.ends_with('/'),
            "{base}"
        );
    }
    let names: HashSet<_> = spec.files.iter().map(|f| f.name).collect();
    assert_eq!(
        names.len(),
        spec.files.len(),
        "duplicate file in {}",
        spec.id
    );
    for required in [
        "encoder.int8.onnx",
        "decoder.int8.onnx",
        "joiner.int8.onnx",
        "tokens.txt",
    ] {
        assert!(names.contains(required), "{} lacks {required}", spec.id);
    }
    for f in spec.files {
        assert_eq!(f.sha256.len(), 64, "{}", f.name);
        assert!(
            f.sha256.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')),
            "{}",
            f.name
        );
        assert!(f.size > 0, "{}", f.name);
    }
}

#[test]
fn catalog_specs_are_consistent() {
    let ids: HashSet<_> = catalog::ALL.iter().map(|s| s.id).collect();
    assert_eq!(ids.len(), catalog::ALL.len(), "model ids must be unique");
    for spec in catalog::ALL {
        assert_spec_consistent(spec);
    }
    assert_eq!(catalog::PARAKEET_V2_EN.id, "parakeet-tdt-0.6b-v2-en");
    assert_eq!(catalog::PARAKEET_V3_MULTI.id, "parakeet-tdt-0.6b-v3-multi");
}

#[test]
fn load_reports_a_missing_model_file() {
    let dir = tempfile::tempdir().unwrap();
    let err = ParakeetEngine::load(dir.path())
        .err()
        .expect("empty dir must not load");
    assert!(err.to_string().contains("encoder.int8.onnx"), "{err}");
}

fn read_wav(path: &Path) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
    let rate = u32::from_le_bytes(bytes[24..28].try_into().unwrap());
    assert_eq!(rate, 16_000, "{} must be 16 kHz", path.display());
    let data = bytes
        .windows(4)
        .position(|w| w == b"data")
        .expect("data chunk")
        + 8;
    bytes[data..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| f32::from(i16::from_le_bytes(*b)) / 32768.0)
        .collect()
}

#[test]
#[ignore = "needs the speech model; set GESPAN_SPEECH_MODEL_DIR"]
fn parakeet_clips_meet_wer() {
    let model_dir = PathBuf::from(
        std::env::var("GESPAN_SPEECH_MODEL_DIR").expect("set GESPAN_SPEECH_MODEL_DIR"),
    );
    let mut engine = ParakeetEngine::load(&model_dir).unwrap();
    let clips = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/clips");
    let mut total = 0.0;
    let mut count = 0;
    for entry in std::fs::read_dir(&clips).unwrap() {
        let wav = entry.unwrap().path();
        if wav.extension().is_none_or(|e| e != "wav") {
            continue;
        }
        let reference = std::fs::read_to_string(wav.with_extension("txt")).unwrap();
        let text = postprocess(&engine.transcribe(&read_wav(&wav), &[]).unwrap(), &[]);
        let score = wer(&reference, &text);
        println!("{}: wer {score:.3}: {text}", wav.display());
        total += score;
        count += 1;
    }
    assert!(count > 0, "no clips found");
    let mean = total / f64::from(count);
    println!("mean wer {mean:.3}");
    assert!(mean <= 0.10, "mean WER {mean:.3} exceeds 0.10");
}
