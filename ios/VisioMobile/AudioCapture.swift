import AVFoundation
import visioFFI

/// Captures real microphone audio via AVAudioEngine and pushes 48 kHz mono
/// Int16 PCM frames to Rust via visio_push_ios_audio_frame().
final class AudioCapture {
    private let engine = AVAudioEngine()
    private var isRunning = false

    private let sampleRate: Double = 48_000
    private let channels: UInt32 = 1
    /// 10ms frames = 480 samples at 48kHz
    private let samplesPerFrame: Int = 480

    func start() {
        guard !isRunning else { return }

        let inputNode = engine.inputNode
        let hwFormat = inputNode.inputFormat(forBus: 0)
        NSLog("AudioCapture: hardware format: %@", hwFormat.description)

        // Request 48kHz mono — AVAudioEngine will resample from hardware format.
        guard let desiredFormat = AVAudioFormat(
            commonFormat: .pcmFormatFloat32,
            sampleRate: sampleRate,
            channels: AVAudioChannelCount(channels),
            interleaved: false
        ) else {
            NSLog("AudioCapture: failed to create desired format")
            return
        }

        // Install a tap on the input node to receive mic audio.
        let samplesPerFrame = self.samplesPerFrame
        let sampleRate = UInt32(self.sampleRate)
        let channels = self.channels

        inputNode.installTap(onBus: 0, bufferSize: AVAudioFrameCount(samplesPerFrame), format: desiredFormat) { buffer, _ in
            guard let floatData = buffer.floatChannelData?[0] else { return }
            let frameCount = Int(buffer.frameLength)

            // Convert Float32 → Int16 and push in chunks of samplesPerFrame
            var i16Buf = [Int16](repeating: 0, count: frameCount)
            for i in 0..<frameCount {
                i16Buf[i] = Int16(clamping: Int(floatData[i] * 32767.0))
            }

            // Push complete frames
            var offset = 0
            while offset + samplesPerFrame <= frameCount {
                i16Buf.withUnsafeBufferPointer { ptr in
                    guard let base = ptr.baseAddress else { return }
                    visio_push_ios_audio_frame(
                        base.advanced(by: offset),
                        UInt32(samplesPerFrame),
                        sampleRate,
                        channels
                    )
                }
                offset += samplesPerFrame
            }
        }

        do {
            try engine.start()
            isRunning = true
            NSLog("AudioCapture: started (48kHz mono, %d samples/frame)", samplesPerFrame)
        } catch {
            NSLog("AudioCapture: failed to start engine: %@", error.localizedDescription)
        }
    }

    func stop() {
        guard isRunning else { return }
        engine.inputNode.removeTap(onBus: 0)
        engine.stop()
        isRunning = false
        NSLog("AudioCapture: stopped")
    }
}
