# Speech models

Attribution text (shown in the app):

> Speech recognition uses NVIDIA Parakeet TDT 0.6B (CC-BY-4.0), run with sherpa-onnx (Apache-2.0).

## Shipped (downloaded on explicit request, never bundled)

| Id | Model | Licence | Source (pinned commit) |
| --- | --- | --- | --- |
| `parakeet-tdt-0.6b-v2-en` | NVIDIA Parakeet TDT 0.6B v2, int8, English | CC-BY-4.0 | `csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8` @ `1ab9323565ddb038682214b292f588070a538ce2` |
| `parakeet-tdt-0.6b-v3-multi` | NVIDIA Parakeet TDT 0.6B v3, int8, multilingual | CC-BY-4.0 | `csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8` @ `2bda32ec70b097a55adaa07d9a7173915b43cc78` |

The runtime is sherpa-onnx (Apache-2.0), linked statically.

## Not shipped

- Parakeet EOU: not used.
- Nemotron speech models: not used.
- fluidaudio: not used.

## Hotwords (spike result)

Measured on an Apple Silicon Mac, release build, 4 threads, v2 int8:

- Speed: about 10 to 12 times real time (7.4 s clip decoded in 0.6 to 0.7 s) with the default greedy decoder; recognizer creation takes about 0.9 s.
- Hotwords are not usable. Hotwords need `decoding_method = "modified_beam_search"` and `create_stream_with_hotwords`, but they must be passed as pre-encoded BPE pieces and the model repo ships no BPE vocabulary. Plain words (`PHEBE`) and sentencepiece-style pieces (`▁Ph o e be`) are both rejected by sherpa ("Cannot find ID for token", "Encode hotwords failed, skipping"). `modified_beam_search` also decoded slower (6 to 7 times real time) and changed the output punctuation for the worse.
- Decision: `ParakeetEngine::transcribe` ignores `terms`. Project vocabulary is applied afterwards by `postprocess`.

## Owner sign-off (#118)

- [ ] Owner has reviewed the licences and attribution above.
