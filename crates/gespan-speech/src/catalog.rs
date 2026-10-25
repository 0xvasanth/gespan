//! Pinned speech models. Sources are fixed Hugging Face commits so a file can
//! never change under a recorded hash.

use gespan_models::{ModelFile, ModelSpec};

pub static PARAKEET_V2_EN: ModelSpec = ModelSpec {
    id: "parakeet-tdt-0.6b-v2-en",
    label: "English (Parakeet v2)",
    base_urls: &["https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8/resolve/1ab9323565ddb038682214b292f588070a538ce2/"],
    files: &[
        ModelFile {
            name: "encoder.int8.onnx",
            url_path: "encoder.int8.onnx",
            size: 652_184_296,
            sha256: "a32b12d17bbbc309d0686fbbcc2987b5e9b8333a7da83fa6b089f0a2acd651ab",
        },
        ModelFile {
            name: "decoder.int8.onnx",
            url_path: "decoder.int8.onnx",
            size: 7_257_753,
            sha256: "b6bb64963457237b900e496ee9994b59294526439fbcc1fecf705b31a15c6b4e",
        },
        ModelFile {
            name: "joiner.int8.onnx",
            url_path: "joiner.int8.onnx",
            size: 1_739_080,
            sha256: "7946164367946e7f9f29a122407c3252b680dbae9a51343eb2488d057c3c43d2",
        },
        ModelFile {
            name: "tokens.txt",
            url_path: "tokens.txt",
            size: 9_384,
            sha256: "ec182b70dd42113aff6c5372c75cac58c952443eb22322f57bbd7f53977d497d",
        },
    ],
};

pub static PARAKEET_V3_MULTI: ModelSpec = ModelSpec {
    id: "parakeet-tdt-0.6b-v3-multi",
    label: "Multilingual (Parakeet v3)",
    base_urls: &["https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/2bda32ec70b097a55adaa07d9a7173915b43cc78/"],
    files: &[
        ModelFile {
            name: "encoder.int8.onnx",
            url_path: "encoder.int8.onnx",
            size: 652_184_281,
            sha256: "acfc2b4456377e15d04f0243af540b7fe7c992f8d898d751cf134c3a55fd2247",
        },
        ModelFile {
            name: "decoder.int8.onnx",
            url_path: "decoder.int8.onnx",
            size: 11_845_275,
            sha256: "179e50c43d1a9de79c8a24149a2f9bac6eb5981823f2a2ed88d655b24248db4e",
        },
        ModelFile {
            name: "joiner.int8.onnx",
            url_path: "joiner.int8.onnx",
            size: 6_355_277,
            sha256: "3164c13fc2821009440d20fcb5fdc78bff28b4db2f8d0f0b329101719c0948b3",
        },
        ModelFile {
            name: "tokens.txt",
            url_path: "tokens.txt",
            size: 93_939,
            sha256: "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
        },
    ],
};

pub const ALL: &[&ModelSpec] = &[&PARAKEET_V2_EN, &PARAKEET_V3_MULTI];
