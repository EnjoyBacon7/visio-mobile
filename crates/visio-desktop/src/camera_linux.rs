//! Linux camera capture using V4L2.
//!
//! Opens the camera, captures MJPEG frames, decodes to RGB, converts to I420,
//! and feeds them into a LiveKit NativeVideoSource.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use livekit::webrtc::prelude::*;
use livekit::webrtc::video_source::native::NativeVideoSource;
use serde::Serialize;
use v4l::buffer::Type;
use v4l::io::mmap::Stream;
use v4l::io::traits::CaptureStream;
use v4l::video::Capture;
use v4l::{Device, FourCC};

// ---------------------------------------------------------------------------
// Video device enumeration
// ---------------------------------------------------------------------------

#[derive(Serialize, Clone)]
pub struct VideoDeviceInfo {
    pub name: String,
    pub unique_id: String,
    pub is_default: bool,
}

/// List available video capture devices.
pub fn list_cameras() -> Vec<VideoDeviceInfo> {
    let mut devices = Vec::new();

    // Check /dev/video0 through /dev/video9
    for i in 0..10 {
        let path = format!("/dev/video{}", i);
        if let Ok(dev) = Device::new(i) {
            if let Ok(caps) = dev.query_caps() {
                // Only include devices that support video capture
                if caps.capabilities.contains(v4l::capability::Flags::VIDEO_CAPTURE) {
                    devices.push(VideoDeviceInfo {
                        name: caps.card.clone(),
                        unique_id: path,
                        is_default: devices.is_empty(),
                    });
                }
            }
        }
    }

    devices
}

// ---------------------------------------------------------------------------
// Linux camera capture
// ---------------------------------------------------------------------------

/// Manages a V4L2 camera capture session on Linux.
pub struct LinuxCameraCapture {
    running: Arc<AtomicBool>,
    _capture_thread: JoinHandle<()>,
}

impl LinuxCameraCapture {
    /// Start capturing from the default camera and feeding frames into `source`.
    pub fn start(source: NativeVideoSource) -> Result<Self, String> {
        Self::start_with_index(0, source)
    }

    /// Start capturing from a camera identified by its unique ID (e.g., "/dev/video0").
    pub fn start_with_unique_id(
        unique_id: &str,
        source: NativeVideoSource,
    ) -> Result<Self, String> {
        // Parse unique_id to extract index (e.g., "/dev/video0" -> 0)
        let idx = if unique_id.starts_with("/dev/video") {
            unique_id
                .trim_start_matches("/dev/video")
                .parse::<usize>()
                .unwrap_or(0)
        } else {
            0
        };
        Self::start_with_index(idx, source)
    }

    fn start_with_index(device_index: usize, source: NativeVideoSource) -> Result<Self, String> {
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();

        let capture_thread = thread::Builder::new()
            .name("visio-camera-linux".into())
            .spawn(move || {
                if let Err(e) = capture_loop(device_index, source, running_clone) {
                    tracing::error!("Linux camera capture error: {e}");
                }
            })
            .map_err(|e| format!("Failed to spawn camera thread: {e}"))?;

        tracing::info!("Linux camera capture started");

        Ok(LinuxCameraCapture {
            running,
            _capture_thread: capture_thread,
        })
    }

    /// Stop camera capture.
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        tracing::info!("Linux camera capture stopped");
    }
}

impl Drop for LinuxCameraCapture {
    fn drop(&mut self) {
        if self.running.load(Ordering::SeqCst) {
            self.stop();
        }
    }
}

fn capture_loop(
    device_index: usize,
    source: NativeVideoSource,
    running: Arc<AtomicBool>,
) -> Result<(), String> {
    // Open camera device
    let dev = Device::new(device_index).map_err(|e| format!("Failed to open camera: {e}"))?;

    // Get current format - use whatever the camera is already configured for
    let format = dev.format().map_err(|e| format!("Failed to get format: {e}"))?;

    tracing::info!(
        "Camera current format: {}x{} {:?}",
        format.width,
        format.height,
        format.fourcc
    );

    // Only try to change format if resolution is too low
    let format = if format.width < 320 || format.height < 240 {
        let mut new_format = format.clone();
        new_format.width = 640;
        new_format.height = 480;
        match dev.set_format(&new_format) {
            Ok(f) => {
                tracing::info!("Camera format updated to: {}x{}", f.width, f.height);
                f
            }
            Err(e) => {
                tracing::warn!("Could not update format: {e}, using current");
                format
            }
        }
    } else {
        format
    };

    let width = format.width;
    let height = format.height;
    let fourcc = format.fourcc;

    tracing::info!(
        "Linux camera opened: {}x{} {:?}",
        width,
        height,
        fourcc
    );

    // Create memory-mapped stream
    let mut stream =
        Stream::with_buffers(&dev, Type::VideoCapture, 4)
            .map_err(|e| format!("Failed to create stream: {e}"))?;

    let mut frame_count: u64 = 0;

    while running.load(Ordering::SeqCst) {
        // Capture frame
        let (buf, _meta) = match stream.next() {
            Ok(frame) => frame,
            Err(e) => {
                tracing::warn!("Frame capture error: {e}");
                continue;
            }
        };

        // Decode frame based on format
        let rgb_data = if fourcc == FourCC::new(b"MJPG") {
            // Decode MJPEG to RGB
            match decode_mjpeg(buf, width, height) {
                Ok(rgb) => rgb,
                Err(e) => {
                    tracing::warn!("MJPEG decode error: {e}");
                    continue;
                }
            }
        } else if fourcc == FourCC::new(b"YUYV") {
            // Convert YUYV to RGB
            yuyv_to_rgb(buf, width as usize, height as usize)
        } else {
            tracing::warn!("Unsupported format: {:?}", fourcc);
            continue;
        };

        // Convert RGB to I420
        let mut i420 = I420Buffer::new(width, height);

        {
            let strides = i420.strides();
            let (y_data, u_data, v_data) = i420.data_mut();

            rgb_to_i420(
                &rgb_data,
                width as usize,
                height as usize,
                y_data,
                strides.0 as usize,
                u_data,
                strides.1 as usize,
                v_data,
                strides.2 as usize,
            );
        }

        // Apply background processing (blur/replacement) if enabled
        {
            let strides = i420.strides();
            let (y_data, u_data, v_data) = i420.data_mut();
            visio_ffi::blur::BlurProcessor::process_i420(
                y_data,
                u_data,
                v_data,
                width as usize,
                height as usize,
                strides.0 as usize,
                strides.1 as usize,
                strides.2 as usize,
                0, // No rotation
            );
        }

        // Feed frame into LiveKit
        let video_frame = VideoFrame {
            rotation: VideoRotation::VideoRotation0,
            timestamp_us: 0,
            buffer: i420,
        };
        source.capture_frame(&video_frame);

        frame_count += 1;

        // Self-view: render every 3rd frame (~10 fps at 30fps capture)
        if frame_count % 3 == 0 {
            visio_video::render_local_i420(&video_frame.buffer, "local-camera");
        }

        if frame_count == 1 {
            tracing::info!("First camera frame captured and processed");
        }
    }

    Ok(())
}

/// Decode MJPEG frame to RGB using the image crate.
fn decode_mjpeg(data: &[u8], _width: u32, _height: u32) -> Result<Vec<u8>, String> {
    use image::io::Reader as ImageReader;
    use std::io::Cursor;

    let reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|e| format!("Failed to guess format: {e}"))?;

    let img = reader
        .decode()
        .map_err(|e| format!("Failed to decode JPEG: {e}"))?;

    Ok(img.to_rgb8().into_raw())
}

/// Convert YUYV (YUV 4:2:2) to RGB.
fn yuyv_to_rgb(data: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut rgb = vec![0u8; width * height * 3];

    for i in 0..(width * height / 2) {
        let y0 = data[i * 4] as f32;
        let u = data[i * 4 + 1] as f32 - 128.0;
        let y1 = data[i * 4 + 2] as f32;
        let v = data[i * 4 + 3] as f32 - 128.0;

        // First pixel
        let r0 = (y0 + 1.402 * v).clamp(0.0, 255.0) as u8;
        let g0 = (y0 - 0.344136 * u - 0.714136 * v).clamp(0.0, 255.0) as u8;
        let b0 = (y0 + 1.772 * u).clamp(0.0, 255.0) as u8;

        // Second pixel
        let r1 = (y1 + 1.402 * v).clamp(0.0, 255.0) as u8;
        let g1 = (y1 - 0.344136 * u - 0.714136 * v).clamp(0.0, 255.0) as u8;
        let b1 = (y1 + 1.772 * u).clamp(0.0, 255.0) as u8;

        rgb[i * 6] = r0;
        rgb[i * 6 + 1] = g0;
        rgb[i * 6 + 2] = b0;
        rgb[i * 6 + 3] = r1;
        rgb[i * 6 + 4] = g1;
        rgb[i * 6 + 5] = b1;
    }

    rgb
}

/// Convert RGB24 to I420 (BT.601 full range).
fn rgb_to_i420(
    rgb: &[u8],
    width: usize,
    height: usize,
    y_dst: &mut [u8],
    y_stride: usize,
    u_dst: &mut [u8],
    u_stride: usize,
    v_dst: &mut [u8],
    v_stride: usize,
) {
    for row in 0..height {
        for col in 0..width {
            let rgb_idx = (row * width + col) * 3;
            let r = rgb[rgb_idx] as f32;
            let g = rgb[rgb_idx + 1] as f32;
            let b = rgb[rgb_idx + 2] as f32;

            // BT.601 full-range RGB to YUV
            let y = (0.299 * r + 0.587 * g + 0.114 * b).clamp(0.0, 255.0) as u8;

            let y_idx = row * y_stride + col;
            y_dst[y_idx] = y;

            // Subsample chroma (every 2x2 block shares one U and V)
            if row % 2 == 0 && col % 2 == 0 {
                let u = ((-0.169 * r - 0.331 * g + 0.5 * b) + 128.0).clamp(0.0, 255.0) as u8;
                let v = ((0.5 * r - 0.419 * g - 0.081 * b) + 128.0).clamp(0.0, 255.0) as u8;

                let chroma_row = row / 2;
                let chroma_col = col / 2;
                let u_idx = chroma_row * u_stride + chroma_col;
                let v_idx = chroma_row * v_stride + chroma_col;

                u_dst[u_idx] = u;
                v_dst[v_idx] = v;
            }
        }
    }
}
