import Foundation
import visioFFI

/// Generates a 440Hz sine wave and pushes PCM frames to Rust via visio_push_ios_audio_frame().
/// Used for E2E testing on simulators that have no real microphone.
final class SyntheticAudioCapture {
    private let queue = DispatchQueue(label: "io.visio.synthetic-audio", qos: .userInitiated)
    private var running = false
    private var workItem: DispatchWorkItem?

    private let sampleRate: Int = 48000
    private let channels: Int = 1
    private let frameDurationMs: Int = 20
    private let frequency: Double = 440.0
    private let amplitude: Double = 3000.0

    func start() {
        queue.async { [self] in
            guard !running else { return }
            running = true

            let samplesPerFrame = sampleRate * frameDurationMs / 1000  // 960
            var sampleOffset: UInt64 = 0

            NSLog("SyntheticAudioCapture: started (%.0fHz sine, %dHz, %dms frames)", frequency, sampleRate, frameDurationMs)

            let item = DispatchWorkItem { [weak self] in
                guard let self else { return }
                var buffer = [Int16](repeating: 0, count: samplesPerFrame)

                while self.running {
                    for i in 0..<samplesPerFrame {
                        let t = Double(sampleOffset + UInt64(i)) / Double(self.sampleRate)
                        let val = sin(t * self.frequency * 2.0 * .pi) * self.amplitude
                        buffer[i] = Int16(clamping: Int(val))
                    }
                    sampleOffset += UInt64(samplesPerFrame)

                    buffer.withUnsafeBufferPointer { ptr in
                        guard let base = ptr.baseAddress else { return }
                        visio_push_ios_audio_frame(
                            base,
                            UInt32(samplesPerFrame),
                            UInt32(self.sampleRate),
                            UInt32(self.channels)
                        )
                    }

                    Thread.sleep(forTimeInterval: Double(self.frameDurationMs) / 1000.0)
                }

                NSLog("SyntheticAudioCapture: stopped")
            }
            self.workItem = item
            queue.async(execute: item)
        }
    }

    func stop() {
        queue.async { [self] in
            running = false
            workItem?.cancel()
            workItem = nil
        }
    }
}
