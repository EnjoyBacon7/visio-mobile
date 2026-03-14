use std::sync::Arc;
use livekit::webrtc::audio_source::native::NativeAudioSource;
use visio_core::AudioPlayoutBuffer;
use super::audio_engine::VoiceAudioEngine;

pub struct WindowsAudioEngine;

impl WindowsAudioEngine {
    pub fn new(_input_device: Option<&str>, _output_device: Option<&str>) -> Self {
        Self
    }
}

impl VoiceAudioEngine for WindowsAudioEngine {
    fn start_playout(&mut self, _buffer: Arc<AudioPlayoutBuffer>) -> Result<(), String> {
        Err("Windows audio engine not yet implemented".into())
    }
    fn start_capture(&mut self, _source: NativeAudioSource) -> Result<(), String> {
        Err("Windows audio engine not yet implemented".into())
    }
    fn stop_capture(&mut self) {}
    fn stop_playout(&mut self) {}
}
