//! Thread-safe bounded queue for captured audio frames.
//!
//! The cpal audio callback pushes frames without blocking on async I/O.
//! A separate tokio task pops frames and calls `capture_frame()`.

use std::collections::VecDeque;
use std::sync::Mutex;

/// A single captured audio frame ready for submission to LiveKit.
#[derive(Clone)]
pub struct CapturedFrame {
    pub pcm: Vec<i16>,
    pub sample_rate: u32,
    pub num_channels: u32,
    pub samples_per_channel: u32,
}

/// Bounded FIFO queue for captured audio frames.
///
/// When the queue is full, the oldest frame is dropped to make room.
/// All operations hold a mutex only briefly (no async, no I/O).
pub struct AudioCaptureBuffer {
    queue: Mutex<VecDeque<CapturedFrame>>,
    max_frames: usize,
}

impl AudioCaptureBuffer {
    pub fn new(max_frames: usize) -> Self {
        Self {
            queue: Mutex::new(VecDeque::with_capacity(max_frames)),
            max_frames,
        }
    }

    /// Push a frame. Non-blocking (mutex only).
    /// Returns `false` if the queue was full (oldest frame dropped).
    pub fn push(&self, frame: CapturedFrame) -> bool {
        let mut q = self.queue.lock().unwrap();
        if q.len() >= self.max_frames {
            q.pop_front();
            q.push_back(frame);
            false
        } else {
            q.push_back(frame);
            true
        }
    }

    /// Pop the oldest frame, if any.
    pub fn pop(&self) -> Option<CapturedFrame> {
        self.queue.lock().unwrap().pop_front()
    }

    /// Number of frames currently queued.
    pub fn len(&self) -> usize {
        self.queue.lock().unwrap().len()
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_frame(value: i16) -> CapturedFrame {
        CapturedFrame {
            pcm: vec![value; 480],
            sample_rate: 48_000,
            num_channels: 1,
            samples_per_channel: 480,
        }
    }

    #[test]
    fn push_and_pop() {
        let buf = AudioCaptureBuffer::new(4);
        assert!(buf.push(make_frame(1)));
        assert!(buf.push(make_frame(2)));
        assert_eq!(buf.len(), 2);

        let f1 = buf.pop().unwrap();
        assert_eq!(f1.pcm[0], 1);
        let f2 = buf.pop().unwrap();
        assert_eq!(f2.pcm[0], 2);
        assert!(buf.is_empty());
    }

    #[test]
    fn overflow_drops_oldest() {
        let buf = AudioCaptureBuffer::new(2);
        assert!(buf.push(make_frame(10)));
        assert!(buf.push(make_frame(20)));
        // Queue is full — push returns false and drops oldest (10)
        assert!(!buf.push(make_frame(30)));
        assert_eq!(buf.len(), 2);

        let f1 = buf.pop().unwrap();
        assert_eq!(f1.pcm[0], 20);
        let f2 = buf.pop().unwrap();
        assert_eq!(f2.pcm[0], 30);
    }

    #[test]
    fn pop_empty_returns_none() {
        let buf = AudioCaptureBuffer::new(4);
        assert!(buf.pop().is_none());
        assert!(buf.is_empty());
    }
}
