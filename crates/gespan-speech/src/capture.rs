//! Microphone capture. Audio handed to the sink is 16 kHz mono f32.

use std::sync::mpsc;
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample, Stream, StreamConfig};

pub const TARGET_RATE: u32 = 16_000;

pub type Sink = Box<dyn FnMut(&[f32]) + Send>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CaptureError {
    #[error("no input device")]
    NoInputDevice,
    #[error("microphone access denied or unavailable")]
    Denied,
}

/// A source of 16 kHz mono samples that only runs between `start` and `stop`.
pub trait AudioSource: Send {
    fn start(&mut self, sink: Sink) -> Result<(), CaptureError>;
    fn stop(&mut self);
}

/// Averages interleaved frames of `channels` channels into one mono channel.
pub fn downmix_to_mono(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Streaming linear-interpolation resampler for mono audio.
pub struct Resampler {
    step: f64,
    pos: f64,
    pending: Vec<f32>,
}

impl Resampler {
    pub fn new(from_rate: u32, to_rate: u32) -> Self {
        Self {
            step: f64::from(from_rate) / f64::from(to_rate),
            pos: 0.0,
            pending: Vec::new(),
        }
    }

    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        self.pending.extend_from_slice(input);
        let mut out = Vec::with_capacity((input.len() as f64 / self.step) as usize + 1);
        while self.pos + 1.0 < self.pending.len() as f64 {
            let i = self.pos as usize;
            let frac = (self.pos - i as f64) as f32;
            out.push(self.pending[i] * (1.0 - frac) + self.pending[i + 1] * frac);
            self.pos += self.step;
        }
        let consumed = (self.pos as usize).min(self.pending.len());
        self.pending.drain(..consumed);
        self.pos -= consumed as f64;
        out
    }
}

/// Converts device frames to 16 kHz mono and forwards them to the sink.
struct Pipeline {
    channels: usize,
    resampler: Resampler,
    sink: Sink,
}

impl Pipeline {
    fn push(&mut self, interleaved: &[f32]) {
        let mono = downmix_to_mono(interleaved, self.channels);
        let out = self.resampler.process(&mono);
        if !out.is_empty() {
            (self.sink)(&out);
        }
    }
}

pub fn i16_to_f32(s: i16) -> f32 {
    f32::from(s) / 32768.0
}

pub fn u16_to_f32(s: u16) -> f32 {
    (f32::from(s) - 32768.0) / 32768.0
}

/// The default input device through cpal. The stream lives on its own thread
/// (cpal streams are not `Send` everywhere) and exists only while recording,
/// so the system microphone indicator is off when idle.
#[derive(Default)]
pub struct MicSource {
    running: Option<Running>,
}

struct Running {
    stop: mpsc::Sender<()>,
    thread: JoinHandle<()>,
}

impl MicSource {
    pub fn new() -> Self {
        Self::default()
    }
}

impl AudioSource for MicSource {
    fn start(&mut self, sink: Sink) -> Result<(), CaptureError> {
        self.stop();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let thread = std::thread::spawn(move || match open_stream(sink) {
            Ok(stream) => {
                let _ = ready_tx.send(Ok(()));
                // Returns on stop or when the sender is dropped.
                let _ = stop_rx.recv();
                drop(stream);
            }
            Err(err) => {
                let _ = ready_tx.send(Err(err));
            }
        });
        match ready_rx.recv() {
            Ok(Ok(())) => {
                self.running = Some(Running {
                    stop: stop_tx,
                    thread,
                });
                Ok(())
            }
            Ok(Err(err)) => {
                let _ = thread.join();
                Err(err)
            }
            Err(_) => {
                let _ = thread.join();
                Err(CaptureError::Denied)
            }
        }
    }

    fn stop(&mut self) {
        if let Some(running) = self.running.take() {
            let _ = running.stop.send(());
            let _ = running.thread.join();
        }
    }
}

impl Drop for MicSource {
    fn drop(&mut self) {
        self.stop();
    }
}

fn open_stream(sink: Sink) -> Result<Stream, CaptureError> {
    let device = cpal::default_host()
        .default_input_device()
        .ok_or(CaptureError::NoInputDevice)?;
    let supported = device
        .default_input_config()
        .map_err(|_| CaptureError::Denied)?;
    let config: StreamConfig = supported.config();
    let pipeline = Pipeline {
        channels: usize::from(config.channels),
        resampler: Resampler::new(config.sample_rate, TARGET_RATE),
        sink,
    };
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, config, pipeline, |s| s),
        SampleFormat::I16 => build::<i16>(&device, config, pipeline, i16_to_f32),
        SampleFormat::U16 => build::<u16>(&device, config, pipeline, u16_to_f32),
        _ => return Err(CaptureError::Denied),
    }?;
    stream.play().map_err(|_| CaptureError::Denied)?;
    Ok(stream)
}

fn build<T: SizedSample + Send + 'static>(
    device: &cpal::Device,
    config: StreamConfig,
    mut pipeline: Pipeline,
    convert: fn(T) -> f32,
) -> Result<Stream, CaptureError> {
    let mut scratch: Vec<f32> = Vec::new();
    device
        .build_input_stream::<T, _, _>(
            config,
            move |data: &[T], _| {
                scratch.clear();
                scratch.extend(data.iter().map(|&s| convert(s)));
                pipeline.push(&scratch);
            },
            |_err| {},
            None,
        )
        .map_err(|_| CaptureError::Denied)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_averages_channels() {
        let stereo = [1.0, 0.0, 0.5, 0.5, -1.0, 1.0];
        assert_eq!(downmix_to_mono(&stereo, 2), vec![0.5, 0.5, 0.0]);
        assert_eq!(downmix_to_mono(&[0.25, 0.75], 1), vec![0.25, 0.75]);
    }

    #[test]
    fn downmix_drops_a_trailing_partial_frame() {
        assert_eq!(downmix_to_mono(&[1.0, 1.0, 9.0], 2), vec![1.0]);
    }

    #[test]
    fn stereo_48k_becomes_mono_16k_and_keeps_dc() {
        let frames = 48_000;
        let stereo: Vec<f32> = std::iter::repeat_n([0.5f32, 0.5], frames)
            .flatten()
            .collect();
        let mono = downmix_to_mono(&stereo, 2);
        let out = Resampler::new(48_000, 16_000).process(&mono);
        assert!(out.len().abs_diff(16_000) <= 2, "len {}", out.len());
        assert!(out.iter().all(|s| (s - 0.5).abs() < 1e-6));
    }

    #[test]
    fn chunked_resampling_matches_one_shot_length_and_dc() {
        let signal = vec![0.25f32; 4_410];
        let one_shot = Resampler::new(44_100, 16_000).process(&signal);
        let mut r = Resampler::new(44_100, 16_000);
        let chunked: Vec<f32> = signal.chunks(333).flat_map(|c| r.process(c)).collect();
        assert_eq!(chunked.len(), one_shot.len());
        assert!(chunked.iter().all(|s| (s - 0.25).abs() < 1e-6));
    }

    #[test]
    fn resampler_interpolates_a_ramp() {
        let ramp: Vec<f32> = (0..96).map(|i| i as f32).collect();
        let out = Resampler::new(48_000, 16_000).process(&ramp);
        // Every third input sample, exactly.
        assert_eq!(&out[..4], &[0.0, 3.0, 6.0, 9.0]);
    }

    #[test]
    fn empty_input_yields_nothing() {
        assert!(Resampler::new(48_000, 16_000).process(&[]).is_empty());
    }

    #[test]
    fn integer_formats_map_to_unit_range() {
        assert_eq!(i16_to_f32(0), 0.0);
        assert_eq!(i16_to_f32(i16::MIN), -1.0);
        assert!(i16_to_f32(i16::MAX) < 1.0);
        assert_eq!(u16_to_f32(32768), 0.0);
        assert_eq!(u16_to_f32(0), -1.0);
    }
}
