//! macOS audio engine using AudioUnit VoiceProcessingIO.
//!
//! A single AudioUnit handles both capture and playout, giving the OS
//! the reference signal needed for acoustic echo cancellation (AEC).

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use livekit::webrtc::audio_source::native::NativeAudioSource;
use visio_core::{AudioCaptureBuffer, AudioPlayoutBuffer, CapturedFrame};

use super::audio_engine::{self, LK_CHANNELS, LK_SAMPLE_RATE, VoiceAudioEngine};

// ---------------------------------------------------------------------------
// AudioToolbox FFI
// ---------------------------------------------------------------------------

type AudioUnit = *mut c_void;
type AudioComponent = *mut c_void;
type OSStatus = i32;

const K_AUDIO_UNIT_TYPE_OUTPUT: u32 = u32::from_be_bytes(*b"auou");
const K_AUDIO_UNIT_SUBTYPE_VOICE_PROCESSING_IO: u32 = u32::from_be_bytes(*b"vpio");
const K_AUDIO_UNIT_MANUFACTURER_APPLE: u32 = u32::from_be_bytes(*b"appl");

// Scope/element constants
const K_AUDIO_UNIT_SCOPE_GLOBAL: u32 = 0;
const K_AUDIO_UNIT_SCOPE_INPUT: u32 = 1;
const K_AUDIO_UNIT_SCOPE_OUTPUT: u32 = 2;
const K_INPUT_ELEMENT: u32 = 1; // Microphone
const K_OUTPUT_ELEMENT: u32 = 0; // Speaker

// Property IDs
const K_AUDIO_OUTPUT_UNIT_PROPERTY_ENABLE_IO: u32 = 2003;
const K_AUDIO_UNIT_PROPERTY_STREAM_FORMAT: u32 = 8;
const K_AUDIO_UNIT_PROPERTY_SET_RENDER_CALLBACK: u32 = 23;
const K_AUDIO_OUTPUT_UNIT_PROPERTY_SET_INPUT_CALLBACK: u32 = 2005;

// Format constants
const K_AUDIO_FORMAT_LINEAR_PCM: u32 = u32::from_be_bytes(*b"lpcm");
const K_LINEAR_PCM_FORMAT_FLAG_IS_SIGNED_INTEGER: u32 = 1 << 2;
const K_LINEAR_PCM_FORMAT_FLAG_IS_PACKED: u32 = 1 << 3;

#[repr(C)]
#[derive(Clone, Copy)]
struct AudioComponentDescription {
    component_type: u32,
    component_sub_type: u32,
    component_manufacturer: u32,
    component_flags: u32,
    component_flags_mask: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AudioStreamBasicDescription {
    sample_rate: f64,
    format_id: u32,
    format_flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
    reserved: u32,
}

#[repr(C)]
struct AURenderCallbackStruct {
    input_proc: unsafe extern "C" fn(
        in_ref_con: *mut c_void,
        io_action_flags: *mut u32,
        in_time_stamp: *const c_void,
        in_bus_number: u32,
        in_number_frames: u32,
        io_data: *mut AudioBufferList,
    ) -> OSStatus,
    input_proc_ref_con: *mut c_void,
}

#[repr(C)]
struct AudioBufferList {
    number_buffers: u32,
    buffers: [AudioBuffer; 1],
}

#[repr(C)]
struct AudioBuffer {
    number_channels: u32,
    data_byte_size: u32,
    data: *mut c_void,
}

#[link(name = "AudioToolbox", kind = "framework")]
unsafe extern "C" {
    fn AudioComponentFindNext(
        component: AudioComponent,
        desc: *const AudioComponentDescription,
    ) -> AudioComponent;
    fn AudioComponentInstanceNew(component: AudioComponent, out: *mut AudioUnit) -> OSStatus;
    fn AudioComponentInstanceDispose(unit: AudioUnit) -> OSStatus;
    fn AudioUnitSetProperty(
        unit: AudioUnit,
        id: u32,
        scope: u32,
        element: u32,
        data: *const c_void,
        size: u32,
    ) -> OSStatus;
    fn AudioUnitGetProperty(
        unit: AudioUnit,
        id: u32,
        scope: u32,
        element: u32,
        data: *mut c_void,
        size: *mut u32,
    ) -> OSStatus;
    fn AudioUnitInitialize(unit: AudioUnit) -> OSStatus;
    fn AudioUnitUninitialize(unit: AudioUnit) -> OSStatus;
    fn AudioOutputUnitStart(unit: AudioUnit) -> OSStatus;
    fn AudioOutputUnitStop(unit: AudioUnit) -> OSStatus;
    fn AudioUnitRender(
        unit: AudioUnit,
        flags: *mut u32,
        timestamp: *const c_void,
        bus: u32,
        frames: u32,
        buf_list: *mut AudioBufferList,
    ) -> OSStatus;
}

// ---------------------------------------------------------------------------
// Shared state passed to callbacks via raw pointer
// ---------------------------------------------------------------------------

/// Max frames per callback (CoreAudio typically uses 512 or 1024 at 48kHz).
const MAX_CALLBACK_FRAMES: usize = 1024;

struct CallbackState {
    playout_buffer: Arc<AudioPlayoutBuffer>,
    capture_buffer: Arc<AudioCaptureBuffer>,
    /// Pre-allocated scratch buffer for playout callback (avoid alloc on RT thread).
    playout_scratch: std::cell::UnsafeCell<[i16; MAX_CALLBACK_FRAMES]>,
}

// UnsafeCell is !Sync but callbacks are single-threaded per AudioUnit
unsafe impl Sync for CallbackState {}

/// Playout callback: OS asks for audio to play through speakers.
unsafe extern "C" fn render_callback(
    in_ref_con: *mut c_void,
    _flags: *mut u32,
    _timestamp: *const c_void,
    _bus: u32,
    in_number_frames: u32,
    io_data: *mut AudioBufferList,
) -> OSStatus {
    let state = unsafe { &*(in_ref_con as *const CallbackState) };
    let buf_list = unsafe { &mut *io_data };
    let buf = &mut buf_list.buffers[0];

    let sample_count = in_number_frames as usize;
    // Use pre-allocated scratch buffer (no heap alloc on RT thread)
    let scratch = unsafe { &mut *state.playout_scratch.get() };
    let samples = &mut scratch[..sample_count];
    samples.fill(0);
    state.playout_buffer.pull_samples(samples);

    // Write i16 samples to the output buffer
    let dst = unsafe { std::slice::from_raw_parts_mut(buf.data as *mut i16, sample_count) };
    dst.copy_from_slice(samples);

    0 // noErr
}

/// Input callback: OS has captured audio from microphone.
/// We must call AudioUnitRender to get the data, then push to capture buffer.
unsafe extern "C" fn input_callback(
    in_ref_con: *mut c_void,
    io_action_flags: *mut u32,
    in_time_stamp: *const c_void,
    in_bus_number: u32,
    in_number_frames: u32,
    _io_data: *mut AudioBufferList,
) -> OSStatus {
    let wrapper = unsafe { &*(in_ref_con as *const InputCallbackWrapper) };

    let sample_count = in_number_frames as usize;
    let mut data = vec![0i16; sample_count];
    let buf = AudioBuffer {
        number_channels: 1,
        data_byte_size: (sample_count * 2) as u32,
        data: data.as_mut_ptr() as *mut c_void,
    };
    let mut buf_list = AudioBufferList {
        number_buffers: 1,
        buffers: [buf],
    };

    let status = unsafe {
        AudioUnitRender(
            wrapper.audio_unit,
            io_action_flags,
            in_time_stamp,
            in_bus_number,
            in_number_frames,
            &mut buf_list,
        )
    };
    if status != 0 {
        return status;
    }

    let frame = CapturedFrame {
        pcm: data,
        sample_rate: LK_SAMPLE_RATE,
        num_channels: LK_CHANNELS,
        samples_per_channel: in_number_frames,
    };
    wrapper.state.capture_buffer.push(frame);

    0 // noErr
}

struct InputCallbackWrapper {
    audio_unit: AudioUnit,
    state: Arc<CallbackState>,
}

// ---------------------------------------------------------------------------
// MacAudioEngine
// ---------------------------------------------------------------------------

pub struct MacAudioEngine {
    audio_unit: Option<AudioUnit>,
    callback_state: Option<Arc<CallbackState>>,
    // Box the wrapper so its address is stable for the callback pointer
    input_wrapper: Option<Box<InputCallbackWrapper>>,
    drain_running: Option<Arc<AtomicBool>>,
    _input_device: Option<String>,
    _output_device: Option<String>,
}

// AudioUnit is a raw pointer managed by AudioToolbox — safe to send across threads
unsafe impl Send for MacAudioEngine {}
unsafe impl Sync for MacAudioEngine {}

impl MacAudioEngine {
    pub fn new(input_device: Option<&str>, output_device: Option<&str>) -> Self {
        Self {
            audio_unit: None,
            callback_state: None,
            input_wrapper: None,
            drain_running: None,
            _input_device: input_device.map(String::from),
            _output_device: output_device.map(String::from),
        }
    }

    fn create_and_configure_unit(&self) -> Result<AudioUnit, String> {
        unsafe {
            let desc = AudioComponentDescription {
                component_type: K_AUDIO_UNIT_TYPE_OUTPUT,
                component_sub_type: K_AUDIO_UNIT_SUBTYPE_VOICE_PROCESSING_IO,
                component_manufacturer: K_AUDIO_UNIT_MANUFACTURER_APPLE,
                component_flags: 0,
                component_flags_mask: 0,
            };

            let component = AudioComponentFindNext(std::ptr::null_mut(), &desc);
            if component.is_null() {
                return Err("VoiceProcessingIO AudioComponent not found".into());
            }

            let mut unit: AudioUnit = std::ptr::null_mut();
            let status = AudioComponentInstanceNew(component, &mut unit);
            if status != 0 {
                return Err(format!("AudioComponentInstanceNew failed: {status}"));
            }

            // Enable input (microphone)
            let enable: u32 = 1;
            let status = AudioUnitSetProperty(
                unit,
                K_AUDIO_OUTPUT_UNIT_PROPERTY_ENABLE_IO,
                K_AUDIO_UNIT_SCOPE_INPUT,
                K_INPUT_ELEMENT,
                &enable as *const u32 as *const c_void,
                4,
            );
            if status != 0 {
                AudioComponentInstanceDispose(unit);
                return Err(format!("enable input failed: {status}"));
            }

            // Set stream format: 48kHz, mono, i16
            let format = AudioStreamBasicDescription {
                sample_rate: LK_SAMPLE_RATE as f64,
                format_id: K_AUDIO_FORMAT_LINEAR_PCM,
                format_flags: K_LINEAR_PCM_FORMAT_FLAG_IS_SIGNED_INTEGER
                    | K_LINEAR_PCM_FORMAT_FLAG_IS_PACKED,
                bytes_per_packet: 2,
                frames_per_packet: 1,
                bytes_per_frame: 2,
                channels_per_frame: LK_CHANNELS,
                bits_per_channel: 16,
                reserved: 0,
            };

            // Set format on output scope of input element (what we read from mic)
            let status = AudioUnitSetProperty(
                unit,
                K_AUDIO_UNIT_PROPERTY_STREAM_FORMAT,
                K_AUDIO_UNIT_SCOPE_OUTPUT,
                K_INPUT_ELEMENT,
                &format as *const _ as *const c_void,
                std::mem::size_of::<AudioStreamBasicDescription>() as u32,
            );
            if status != 0 {
                AudioComponentInstanceDispose(unit);
                return Err(format!("set input format failed: {status}"));
            }

            // Set format on input scope of output element (what we write to speaker)
            let status = AudioUnitSetProperty(
                unit,
                K_AUDIO_UNIT_PROPERTY_STREAM_FORMAT,
                K_AUDIO_UNIT_SCOPE_INPUT,
                K_OUTPUT_ELEMENT,
                &format as *const _ as *const c_void,
                std::mem::size_of::<AudioStreamBasicDescription>() as u32,
            );
            if status != 0 {
                AudioComponentInstanceDispose(unit);
                return Err(format!("set output format failed: {status}"));
            }

            Ok(unit)
        }
    }
}

impl VoiceAudioEngine for MacAudioEngine {
    fn start_playout(&mut self, buffer: Arc<AudioPlayoutBuffer>) -> Result<(), String> {
        let unit = self.create_and_configure_unit()?;

        let capture_buffer = Arc::new(AudioCaptureBuffer::new(50));
        let state = Arc::new(CallbackState {
            playout_buffer: buffer,
            capture_buffer,
            playout_scratch: std::cell::UnsafeCell::new([0i16; MAX_CALLBACK_FRAMES]),
        });

        // Set render callback (playout)
        let render_cb = AURenderCallbackStruct {
            input_proc: render_callback,
            input_proc_ref_con: Arc::as_ptr(&state) as *mut c_void,
        };
        let status = unsafe {
            AudioUnitSetProperty(
                unit,
                K_AUDIO_UNIT_PROPERTY_SET_RENDER_CALLBACK,
                K_AUDIO_UNIT_SCOPE_INPUT,
                K_OUTPUT_ELEMENT,
                &render_cb as *const _ as *const c_void,
                std::mem::size_of::<AURenderCallbackStruct>() as u32,
            )
        };
        if status != 0 {
            unsafe { AudioComponentInstanceDispose(unit) };
            return Err(format!("set render callback failed: {status}"));
        }

        // Initialize and start
        let status = unsafe { AudioUnitInitialize(unit) };
        if status != 0 {
            unsafe { AudioComponentInstanceDispose(unit) };
            return Err(format!("AudioUnitInitialize failed: {status}"));
        }

        let status = unsafe { AudioOutputUnitStart(unit) };
        if status != 0 {
            unsafe { AudioUnitUninitialize(unit) };
            unsafe { AudioComponentInstanceDispose(unit) };
            return Err(format!("AudioOutputUnitStart failed: {status}"));
        }

        self.audio_unit = Some(unit);
        self.callback_state = Some(state);
        tracing::info!("macOS VoiceProcessingIO playout started (48kHz mono i16, AEC enabled)");
        Ok(())
    }

    fn start_capture(&mut self, source: NativeAudioSource) -> Result<(), String> {
        let unit = self.audio_unit.ok_or("playout must be started before capture")?;
        let state = self.callback_state.clone().ok_or("no callback state")?;

        // Set input callback (capture)
        let wrapper = Box::new(InputCallbackWrapper {
            audio_unit: unit,
            state: state.clone(),
        });
        let wrapper_ptr = &*wrapper as *const InputCallbackWrapper as *mut c_void;

        let input_cb = AURenderCallbackStruct {
            input_proc: input_callback,
            input_proc_ref_con: wrapper_ptr,
        };
        let status = unsafe {
            AudioUnitSetProperty(
                unit,
                K_AUDIO_OUTPUT_UNIT_PROPERTY_SET_INPUT_CALLBACK,
                K_AUDIO_UNIT_SCOPE_GLOBAL,
                K_INPUT_ELEMENT,
                &input_cb as *const _ as *const c_void,
                std::mem::size_of::<AURenderCallbackStruct>() as u32,
            )
        };
        if status != 0 {
            return Err(format!("set input callback failed: {status}"));
        }

        // Start drain thread
        let drain_running = audio_engine::start_drain_thread(state.capture_buffer.clone(), source);

        self.input_wrapper = Some(wrapper);
        self.drain_running = Some(drain_running);
        tracing::info!("macOS VoiceProcessingIO capture started");
        Ok(())
    }

    fn stop_capture(&mut self) {
        if let Some(running) = self.drain_running.take() {
            running.store(false, Ordering::Relaxed);
        }
        self.input_wrapper = None;
        tracing::info!("macOS audio capture stopped");
    }

    fn stop_playout(&mut self) {
        self.stop_capture();
        if let Some(unit) = self.audio_unit.take() {
            unsafe {
                AudioOutputUnitStop(unit);
                AudioUnitUninitialize(unit);
                AudioComponentInstanceDispose(unit);
            }
        }
        self.callback_state = None;
        tracing::info!("macOS VoiceProcessingIO stopped");
    }
}

impl Drop for MacAudioEngine {
    fn drop(&mut self) {
        self.stop_playout();
    }
}
