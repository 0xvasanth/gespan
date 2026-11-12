//! What turns an entry's text, or a search query, into a vector: the
//! [`Embedder`] trait, a deterministic [`HashEmbedder`] for tests, and (with
//! the `nomic` feature) [`Nomic`], nomic-embed-text-v1.5 run locally.
use super::entry::Error;

/// Every vector the log stores has this many dimensions, the size of
/// nomic-embed-text-v1.5's vectors. The `vec0` table is declared with it.
pub const DIMS: usize = 768;

/// Turns text into a vector of [`dims`](Embedder::dims) floats. Documents
/// (entries) and queries may be embedded differently, as Nomic's task
/// prefixes require. Implementations must be deterministic for a given
/// `model_id`: the log only re-embeds when the id changes.
pub trait Embedder: Send + Sync {
    /// Names the model and its settings; stored next to the vectors so a
    /// change of model is noticed and every entry is embedded again.
    fn model_id(&self) -> &str;
    fn dims(&self) -> usize;
    fn embed_document(&self, text: &str) -> Result<Vec<f32>, Error>;
    fn embed_query(&self, text: &str) -> Result<Vec<f32>, Error>;
}

/// A deterministic embedder for tests: each lower-cased word and each of its
/// character trigrams is hashed (seeded by the model id) into one of
/// [`DIMS`] buckets with a sign, and the sum is L2-normalized. Texts that
/// share words or word parts ("tab", "tabs") end up close; texts with
/// nothing in common end up nearly orthogonal. Two different model ids give
/// unrelated vectors for the same text.
pub struct HashEmbedder {
    model: String,
}

impl HashEmbedder {
    pub fn new() -> HashEmbedder {
        HashEmbedder::named("hash-v1")
    }

    pub fn named(model: &str) -> HashEmbedder {
        HashEmbedder {
            model: model.to_string(),
        }
    }

    fn embed(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0f32; DIMS];
        let lower = text.to_lowercase();
        for word in lower
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
        {
            self.add(&mut v, word, 1.0);
            let padded: Vec<char> = format!(" {word} ").chars().collect();
            for gram in padded.windows(3) {
                self.add(&mut v, &gram.iter().collect::<String>(), 0.5);
            }
        }
        if v.iter().all(|x| *x == 0.0) {
            // Text with no words still needs a direction cosine can measure.
            v[0] = 1.0;
        }
        normalize(&mut v);
        v
    }

    fn add(&self, v: &mut [f32], feature: &str, weight: f32) {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in self.model.bytes().chain([0]).chain(feature.bytes()) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        let bucket = (h % DIMS as u64) as usize;
        v[bucket] += if h >> 63 == 1 { -weight } else { weight };
    }
}

impl Default for HashEmbedder {
    fn default() -> Self {
        HashEmbedder::new()
    }
}

impl Embedder for HashEmbedder {
    fn model_id(&self) -> &str {
        &self.model
    }

    fn dims(&self) -> usize {
        DIMS
    }

    fn embed_document(&self, text: &str) -> Result<Vec<f32>, Error> {
        Ok(self.embed(text))
    }

    fn embed_query(&self, text: &str) -> Result<Vec<f32>, Error> {
        Ok(self.embed(text))
    }
}

/// Scales `v` to unit length (a zero vector is left as it is).
pub fn normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
}

/// The Nomic model's files, pinned to one Hugging Face commit so a file can
/// never change under its recorded hash. Downloaded through `gespan-models`
/// when the user first turns "Find related decisions by meaning" on.
pub static NOMIC_MODEL: gespan_models::ModelSpec = gespan_models::ModelSpec {
    id: "nomic-embed-text-v1.5-int8",
    label: "Nomic Embed Text v1.5",
    base_urls: &["https://huggingface.co/nomic-ai/nomic-embed-text-v1.5/resolve/e9b6763023c676ca8431644204f50c2b100d9aab/"],
    files: &[
        gespan_models::ModelFile {
            name: "model_quantized.onnx",
            url_path: "onnx/model_quantized.onnx",
            size: 137_296_292,
            sha256: "b4342336debaea79de872370664b0aaeb67dea4605513d00ee236ea871a81f27",
        },
        gespan_models::ModelFile {
            name: "tokenizer.json",
            url_path: "tokenizer.json",
            size: 711_396,
            sha256: "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66",
        },
        gespan_models::ModelFile {
            name: "config.json",
            url_path: "config.json",
            size: 2_538,
            sha256: "9ab00bd92cee80a569f708140b7b6c1661a65891ff3765b1519e181ba2f2c92b",
        },
        gespan_models::ModelFile {
            name: "special_tokens_map.json",
            url_path: "special_tokens_map.json",
            size: 695,
            sha256: "5d5b662e421ea9fac075174bb0688ee0d9431699900b90662acd44b2a350503a",
        },
        gespan_models::ModelFile {
            name: "tokenizer_config.json",
            url_path: "tokenizer_config.json",
            size: 1_191,
            sha256: "d7e0000bcc80134debd2222220427e6bf5fa20a669f40a0d0d1409cc18e0a9bc",
        },
    ],
};

#[cfg(feature = "nomic")]
pub use nomic::Nomic;

#[cfg(feature = "nomic")]
mod nomic {
    use super::{normalize, Embedder, Error, DIMS, NOMIC_MODEL};
    use fastembed::{
        InitOptionsUserDefined, Pooling, QuantizationMode, TextEmbedding, TokenizerFiles,
        UserDefinedEmbeddingModel,
    };
    use std::path::Path;
    use std::sync::Mutex;

    /// Tokens embedded per text. Entries are capped at 1,000 characters a
    /// field, so three fields fit; longer queries are cut.
    const MAX_TOKENS: usize = 512;

    /// nomic-embed-text-v1.5 (int8 ONNX) through fastembed on the CPU, with
    /// the model's task prefixes (`search_document: `, `search_query: `) and
    /// L2-normalized output. The int8 model quantizes activations per call,
    /// so it embeds one text per call, never a batch.
    pub struct Nomic {
        model: Mutex<TextEmbedding>,
    }

    impl Nomic {
        /// Loads the model from `dir`, where `gespan-models` downloaded
        /// [`NOMIC_MODEL`]'s files.
        pub fn load(dir: &Path) -> Result<Nomic, Error> {
            let read = |name: &str| {
                std::fs::read(dir.join(name))
                    .map_err(|e| Error::Embed(format!("cannot read {name}: {e}")))
            };
            let model = UserDefinedEmbeddingModel::new(
                read(NOMIC_MODEL.files[0].name)?,
                TokenizerFiles {
                    tokenizer_file: read("tokenizer.json")?,
                    config_file: read("config.json")?,
                    special_tokens_map_file: read("special_tokens_map.json")?,
                    tokenizer_config_file: read("tokenizer_config.json")?,
                },
            )
            .with_pooling(Pooling::Mean)
            .with_quantization(QuantizationMode::Dynamic);
            let options = InitOptionsUserDefined::new().with_max_length(MAX_TOKENS);
            let model = TextEmbedding::try_new_from_user_defined(model, options)
                .map_err(|e| Error::Embed(e.to_string()))?;
            Ok(Nomic {
                model: Mutex::new(model),
            })
        }

        fn embed(&self, prefix: &str, text: &str) -> Result<Vec<f32>, Error> {
            let mut model = self.model.lock().unwrap_or_else(|e| e.into_inner());
            let mut out = model
                .embed(vec![format!("{prefix}{text}")], None)
                .map_err(|e| Error::Embed(e.to_string()))?;
            let mut v = out
                .pop()
                .ok_or_else(|| Error::Embed("the model returned no vector".into()))?;
            normalize(&mut v);
            Ok(v)
        }
    }

    impl Embedder for Nomic {
        fn model_id(&self) -> &str {
            NOMIC_MODEL.id
        }

        fn dims(&self) -> usize {
            DIMS
        }

        fn embed_document(&self, text: &str) -> Result<Vec<f32>, Error> {
            self.embed("search_document: ", text)
        }

        fn embed_query(&self, text: &str) -> Result<Vec<f32>, Error> {
            self.embed("search_query: ", text)
        }
    }
}
