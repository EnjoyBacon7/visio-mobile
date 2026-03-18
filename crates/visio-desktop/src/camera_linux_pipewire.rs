//! PipeWire Camera portal backend.
//!
//! Uses ashpd to request camera access via XDG portal (consent dialog),
//! then opens a PipeWire stream to receive video frames. This enables
//! proper Flatpak sandboxing without --device=all.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use livekit::webrtc::prelude::*;
use livekit::webrtc::video_source::native::NativeVideoSource;

use super::VideoDeviceInfo;
use super::camera_linux_convert;

/// Return a single synthetic camera entry — the portal handles device selection.
pub fn list_cameras() -> Vec<VideoDeviceInfo> {
    vec![VideoDeviceInfo {
        name: "Camera (PipeWire)".to_string(),
        unique_id: "pipewire:camera".to_string(),
        is_default: true,
    }]
}

pub struct PipewireCameraCapture {
    quit_sender: pipewire::channel::Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl PipewireCameraCapture {
    /// Start camera capture via XDG Camera portal + PipeWire stream.
    /// Shows a consent dialog on first use (cached by the portal afterwards).
    pub fn start(source: NativeVideoSource) -> Result<Self, String> {
        // Request camera access via portal on a dedicated thread
        // (ashpd is async, and we may be called from a tokio context)
        let pw_fd = thread::spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| format!("tokio runtime: {e}"))?;
            rt.block_on(async {
                let camera = ashpd::desktop::camera::Camera::new()
                    .await
                    .map_err(|e| format!("camera portal: {e}"))?;
                camera
                    .access_camera()
                    .await
                    .map_err(|e| format!("camera access denied: {e}"))?;
                let fd = camera
                    .open_pipe_wire_remote()
                    .await
                    .map_err(|e| format!("pipewire remote: {e}"))?;
                Ok::<_, String>(fd)
            })
        })
        .join()
        .map_err(|_| "portal thread panicked".to_string())??;

        // Create PipeWire main loop on a dedicated thread
        let (quit_sender, quit_receiver) = pipewire::channel::channel::<()>();

        let capture_thread = thread::Builder::new()
            .name("visio-camera-pipewire".into())
            .spawn(move || {
                if let Err(e) = pipewire_capture_loop(pw_fd, source, quit_receiver) {
                    tracing::error!("PipeWire camera capture error: {e}");
                }
            })
            .map_err(|e| format!("Failed to spawn PipeWire thread: {e}"))?;

        tracing::info!("PipeWire camera capture started");

        Ok(Self {
            quit_sender,
            thread: Some(capture_thread),
        })
    }

    pub fn stop(&mut self) {
        let _ = self.quit_sender.send(());
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
        tracing::info!("PipeWire camera capture stopped");
    }
}

impl Drop for PipewireCameraCapture {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.stop();
        }
    }
}

fn pipewire_capture_loop(
    pw_fd: std::os::fd::OwnedFd,
    source: NativeVideoSource,
    quit_receiver: pipewire::channel::Receiver<()>,
) -> Result<(), String> {
    let main_loop = pipewire::main_loop::MainLoop::new(None)
        .map_err(|e| format!("MainLoop::new: {e}"))?;
    let context = pipewire::context::Context::new(&main_loop)
        .map_err(|e| format!("Context::new: {e}"))?;

    use std::os::fd::AsRawFd;
    let core = context
        .connect_fd(pw_fd.as_raw_fd(), None)
        .map_err(|e| format!("connect_fd: {e}"))?;

    let stream = pipewire::stream::Stream::new(
        &core,
        "visio-camera",
        pipewire::properties::properties! {
            *pipewire::keys::MEDIA_TYPE => "Video",
            *pipewire::keys::MEDIA_CATEGORY => "Capture",
            *pipewire::keys::MEDIA_ROLE => "Camera",
        },
    )
    .map_err(|e| format!("Stream::new: {e}"))?;

    let frame_count = Arc::new(AtomicU64::new(0));
    let frame_count_cb = frame_count.clone();
    let source_cb = source;

    // Shared state for negotiated video format
    let video_width = Arc::new(AtomicU64::new(0));
    let video_height = Arc::new(AtomicU64::new(0));
    let video_format = Arc::new(AtomicU64::new(0));
    let vw_cb = video_width.clone();
    let vh_cb = video_height.clone();
    let vf_cb = video_format.clone();

    // Register quit signal
    let main_loop_weak = main_loop.loop_().downgrade();
    let _quit_listener = quit_receiver.attach(&main_loop.loop_(), move |_| {
        if let Some(main_loop) = main_loop_weak.upgrade() {
            main_loop.quit();
        }
    });

    // Process callback — called for each video frame
    let _listener = stream
        .add_local_listener()
        .param_changed(move |_stream, id, _user_data, param| {
            // Extract video format from SPA param when negotiated
            if param.is_none() {
                return;
            }
            let param = param.unwrap();
            if id == pipewire::spa::param::ParamType::Format.as_raw() {
                if let Ok(info) = pipewire::spa::param::video::VideoInfoRaw::parse(param) {
                    vw_cb.store(info.size().width as u64, Ordering::SeqCst);
                    vh_cb.store(info.size().height as u64, Ordering::SeqCst);
                    vf_cb.store(info.format().as_raw() as u64, Ordering::SeqCst);
                    tracing::info!(
                        "PipeWire format negotiated: {}x{} format={}",
                        info.size().width,
                        info.size().height,
                        info.format().as_raw()
                    );
                }
            }
        })
        .process(move |stream, _| {
            let width = video_width.load(Ordering::SeqCst) as u32;
            let height = video_height.load(Ordering::SeqCst) as u32;
            let format = video_format.load(Ordering::SeqCst) as u32;

            if width == 0 || height == 0 {
                return; // Format not yet negotiated
            }

            if let Some(mut buffer) = stream.dequeue_buffer() {
                if let Some(buf) = buffer.datas_mut().first_mut() {
                    if let Some(data) = buf.data() {
                        let mut i420 = I420Buffer::new(width, height);
                        let converted = convert_spa_frame(
                            data, format, width as usize, height as usize, &mut i420,
                        );

                        if converted {
                            // Apply background blur
                            {
                                let strides = i420.strides();
                                let (y, u, v) = i420.data_mut();
                                visio_ffi::blur::BlurProcessor::process_i420(
                                    y, u, v,
                                    width as usize, height as usize,
                                    strides.0 as usize, strides.1 as usize, strides.2 as usize,
                                    0,
                                );
                            }

                            let video_frame = VideoFrame {
                                rotation: VideoRotation::VideoRotation0,
                                timestamp_us: 0,
                                buffer: i420,
                            };
                            source_cb.capture_frame(&video_frame);

                            let count = frame_count_cb.fetch_add(1, Ordering::Relaxed);
                            if count % 3 == 0 {
                                visio_video::render_local_i420(
                                    &video_frame.buffer,
                                    "local-camera",
                                );
                            }
                            if count == 0 {
                                tracing::info!("First PipeWire camera frame captured");
                            }
                        }
                    }
                }
            }
        })
        .register()
        .map_err(|e| format!("stream listener: {e}"))?;

    // Connect stream — accept any video format the camera offers
    stream
        .connect(
            pipewire::spa::utils::Direction::Input,
            None,
            pipewire::stream::StreamFlags::AUTOCONNECT
                | pipewire::stream::StreamFlags::MAP_BUFFERS,
            &mut [].iter(),
        )
        .map_err(|e| format!("stream connect: {e}"))?;

    tracing::info!("PipeWire stream connected, entering main loop");
    main_loop.run();
    tracing::info!("PipeWire main loop exited");

    Ok(())
}

/// Convert a SPA video buffer to I420 based on the negotiated format.
fn convert_spa_frame(
    data: &[u8],
    format: u32,
    width: usize,
    height: usize,
    i420: &mut I420Buffer,
) -> bool {
    use pipewire::spa::param::video::VideoFormat;

    let strides = i420.strides();
    let (y, u, v) = i420.data_mut();

    // Match SPA video format raw values
    if format == VideoFormat::NV12.as_raw() {
        camera_linux_convert::nv12_to_i420(
            data, width, height,
            y, strides.0 as usize, u, strides.1 as usize, v, strides.2 as usize,
        );
        true
    } else if format == VideoFormat::YUY2.as_raw() {
        camera_linux_convert::yuyv_to_i420(
            data, width, height,
            y, strides.0 as usize, u, strides.1 as usize, v, strides.2 as usize,
        );
        true
    } else if format == VideoFormat::MJPG.as_raw() {
        match camera_linux_convert::decode_mjpeg(data) {
            Ok(rgb) => {
                camera_linux_convert::rgb_to_i420(
                    &rgb, width, height,
                    y, strides.0 as usize, u, strides.1 as usize, v, strides.2 as usize,
                );
                true
            }
            Err(e) => {
                tracing::warn!("PipeWire MJPEG decode: {e}");
                false
            }
        }
    } else if format == VideoFormat::RGB.as_raw() {
        camera_linux_convert::rgb_to_i420(
            data, width, height,
            y, strides.0 as usize, u, strides.1 as usize, v, strides.2 as usize,
        );
        true
    } else {
        tracing::warn!("Unsupported PipeWire format: {format}");
        false
    }
}
