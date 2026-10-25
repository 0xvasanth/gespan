# Test clips

`v2_0.wav` is `test_wavs/0.wav` from
[csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8](https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8)
(CC-BY-4.0), 16 kHz mono. The model repo ships no transcript; `v2_0.txt` is the
reference printed in the sherpa-onnx documentation for this file
(<https://k2-fsa.github.io/sherpa/onnx/pretrained_models/offline-transducer/nemo-transducer-models.html>).

Clips must be 16 kHz mono 16-bit WAV with a `.txt` reference beside them.

## Follow-up (owner)

This is a single general-English clip, so the WER test is a smoke check of the
engine plus `postprocess`, not a measure of code dictation. The owner adds at
least 8 self-recorded code-term clips (identifiers, flags, paths) with
references, which tighten the 0.10 threshold where it matters.
