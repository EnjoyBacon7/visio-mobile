use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use livekit::webrtc::audio_frame::AudioFrame;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use visio_core::AudioPlayoutBuffer;

/// Internal sample rate used by LiveKit (48kHz mono i16).
const LK_SAMPLE_RATE: u32 = 48_000;
const LK_CHANNELS: u32 = 1;

// cpal::Stream is !Send + !Sync due to platform internals, but it is safe
// to hold in Tauri state — we never move the stream across threads, we just
// keep it alive so the OS audio callback keeps firing.
struct SendSyncStream(#[allow(dead_code)] cpal::Stream);
unsafe impl Send for SendSyncStream {}
unsafe impl Sync for SendSyncStream {}

// ---------------------------------------------------------------------------
// Playout — remote audio → speakers
// ---------------------------------------------------------------------------

pub struct CpalAudioPlayout {
    _stream: SendSyncStream,
}

impl CpalAudioPlayout {
    pub fn start(playout_buffer: Arc<AudioPlayoutBuffer>) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("no output audio device available")?;

        let default_cfg = device
            .default_output_config()
            .map_err(|e| format!("default output config: {e}"))?;

        let device_sr = default_cfg.sample_rate().0;
        let device_ch = default_cfg.channels();

        tracing::info!(
            "audio playout: device={:?}, rate={device_sr}, channels={device_ch}, format={:?}",
            device.name(),
            default_cfg.sample_format(),
        );

        // Use the device's default config — CoreAudio works best with f32
        let config = cpal::StreamConfig {
            channels: device_ch,
            sample_rate: cpal::SampleRate(device_sr),
            buffer_size: cpal::BufferSize::Default,
        };

        // Pre-compute how many mono 48kHz samples to pull per device callback.
        // If device runs at a different rate we do naive nearest-neighbor resampling.
        let stream = device
            .build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    // Number of frames (one sample per channel) the device wants
                    let device_frames = data.len() / device_ch as usize;

                    // How many mono 48kHz samples correspond to these frames
                    let lk_samples =
                        (device_frames as u64 * LK_SAMPLE_RATE as u64 / device_sr as u64) as usize;
                    let lk_samples = lk_samples.max(1);

                    let mut buf = vec![0i16; lk_samples];
                    playout_buffer.pull_samples(&mut buf);

                    // Resample 48kHz → device rate using linear interpolation
                    let resampled = if device_sr == LK_SAMPLE_RATE {
                        buf
                    } else {
                        linear_resample(&buf, device_frames)
                    };

                    // Write to output: i16→f32 + mono→multichannel expansion
                    for (frame_idx, &sample) in resampled.iter().enumerate() {
                        let sample_f32 = sample as f32 / 32768.0;
                        for ch in 0..device_ch as usize {
                            data[frame_idx * device_ch as usize + ch] = sample_f32;
                        }
                    }
                },
                |err| {
                    tracing::error!("audio playout stream error: {err}");
                },
                None,
            )
            .map_err(|e| format!("build output stream: {e}"))?;

        stream.play().map_err(|e| format!("play output stream: {e}"))?;
        tracing::info!("cpal audio playout started");

        Ok(Self {
            _stream: SendSyncStream(stream),
        })
    }
}

// ---------------------------------------------------------------------------
// Capture — microphone → NativeAudioSource
// ---------------------------------------------------------------------------

pub struct CpalAudioCapture {
    _stream: SendSyncStream,
    running: Arc<AtomicBool>,
}

impl CpalAudioCapture {
    pub fn start(audio_source: NativeAudioSource) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or("no input audio device available")?;

        let default_cfg = device
            .default_input_config()
            .map_err(|e| format!("default input config: {e}"))?;

        let device_sr = default_cfg.sample_rate().0;
        let device_ch = default_cfg.channels();

        tracing::info!(
            "audio capture: device={:?}, rate={device_sr}, channels={device_ch}, format={:?}",
            device.name(),
            default_cfg.sample_format(),
        );

        let config = cpal::StreamConfig {
            channels: device_ch,
            sample_rate: cpal::SampleRate(device_sr),
            buffer_size: cpal::BufferSize::Default,
        };

        let running = Arc::new(AtomicBool::new(true));
        let running_flag = running.clone();

        // capture_frame is async — use a dedicated single-thread runtime
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("audio capture runtime: {e}"))?;

        let stream = device
            .build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if !running_flag.load(Ordering::Relaxed) {
                        return;
                    }

                    let device_frames = data.len() / device_ch as usize;

                    // Resample to 48kHz mono i16
                    let lk_frames = if device_sr == LK_SAMPLE_RATE {
                        device_frames
                    } else {
                        (device_frames as u64 * LK_SAMPLE_RATE as u64 / device_sr as u64) as usize
                    };
                    let lk_frames = lk_frames.max(1);

                    // Mix multichannel to mono
                    let mono = if device_ch == 1 {
                        data.to_vec()
                    } else {
                        mix_to_mono(data, device_ch as usize)
                    };

                    // Convert f32 mono to i16
                    let mono_i16: Vec<i16> = mono.iter()
                        .map(|&s| (s * 32767.0).clamp(-32768.0, 32767.0) as i16)
                        .collect();

                    // Resample device rate → 48kHz using linear interpolation
                    let pcm = if device_sr == LK_SAMPLE_RATE {
                        mono_i16
                    } else {
                        linear_resample(&mono_i16, lk_frames)
                    };

                    let frame = AudioFrame {
                        data: pcm.into(),
                        sample_rate: LK_SAMPLE_RATE,
                        num_channels: LK_CHANNELS,
                        samples_per_channel: lk_frames as u32,
                    };

                    let _ = rt.block_on(audio_source.capture_frame(&frame));
                },
                |err| {
                    tracing::error!("audio capture stream error: {err}");
                },
                None,
            )
            .map_err(|e| format!("build input stream: {e}"))?;

        stream.play().map_err(|e| format!("play input stream: {e}"))?;
        tracing::info!("cpal audio capture started");

        Ok(Self {
            _stream: SendSyncStream(stream),
            running,
        })
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
        tracing::info!("cpal audio capture stopped");
    }
}

// ---------------------------------------------------------------------------
// Pure helper functions for audio processing
// ---------------------------------------------------------------------------

/// Linear interpolation resampling from `input` to a buffer of `output_len` samples.
fn linear_resample(input: &[i16], output_len: usize) -> Vec<i16> {
    if input.is_empty() || output_len == 0 {
        return vec![0i16; output_len];
    }
    if input.len() == output_len {
        return input.to_vec();
    }
    let mut output = Vec::with_capacity(output_len);
    let ratio = (input.len() - 1) as f64 / (output_len - 1).max(1) as f64;
    for i in 0..output_len {
        let pos = i as f64 * ratio;
        let idx = pos as usize;
        let frac = pos - idx as f64;
        let sample = if idx + 1 < input.len() {
            input[idx] as f64 * (1.0 - frac) + input[idx + 1] as f64 * frac
        } else {
            input[idx] as f64
        };
        output.push(sample.round() as i16);
    }
    output
}

/// Mix multi-channel f32 interleaved audio to mono, averaging all channels.
fn mix_to_mono(data: &[f32], channels: usize) -> Vec<f32> {
    if channels == 0 {
        return Vec::new();
    }
    let frames = data.len() / channels;
    let mut mono = Vec::with_capacity(frames);
    for f in 0..frames {
        let mut sum = 0.0f32;
        for ch in 0..channels {
            sum += data[f * channels + ch];
        }
        mono.push(sum / channels as f32);
    }
    mono
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_same_length() {
        let input: Vec<i16> = vec![0, 100, 200, 300, 400];
        let output = linear_resample(&input, 5);
        assert_eq!(output, input);
    }

    #[test]
    fn resample_upsample_2x() {
        let input: Vec<i16> = vec![0, 100];
        let output = linear_resample(&input, 3);
        assert_eq!(output, vec![0, 50, 100]);
    }

    #[test]
    fn resample_downsample() {
        let input: Vec<i16> = vec![0, 50, 100];
        let output = linear_resample(&input, 2);
        assert_eq!(output[0], 0);
        assert_eq!(output[1], 100);
    }

    #[test]
    fn resample_empty_input() {
        let output = linear_resample(&[], 0);
        assert!(output.is_empty());
    }

    #[test]
    fn resample_single_sample() {
        let output = linear_resample(&[42], 5);
        assert_eq!(output, vec![42, 42, 42, 42, 42]);
    }

    #[test]
    fn mix_to_mono_stereo() {
        let stereo = vec![100.0f32, 200.0, 300.0, 400.0];
        let mono = mix_to_mono(&stereo, 2);
        assert_eq!(mono.len(), 2);
        assert!((mono[0] - 150.0).abs() < f32::EPSILON);
        assert!((mono[1] - 350.0).abs() < f32::EPSILON);
    }

    #[test]
    fn mix_to_mono_single_channel() {
        let data = vec![1.0f32, 2.0, 3.0];
        let mono = mix_to_mono(&data, 1);
        assert_eq!(mono, data);
    }

    #[test]
    fn mix_to_mono_empty() {
        let mono = mix_to_mono(&[], 2);
        assert!(mono.is_empty());
    }
}
