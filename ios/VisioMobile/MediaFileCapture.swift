import Foundation
import AVFoundation
import CoreMedia
import visioFFI

/// Decodes audio and video from an MP4 file and pushes frames through the C FFI,
/// allowing E2E testing without real camera/microphone hardware.
final class MediaFileCapture {
    private let filePath: String
    private let audioQueue = DispatchQueue(label: "io.visio.media-file.audio", qos: .userInitiated)
    private let videoQueue = DispatchQueue(label: "io.visio.media-file.video", qos: .userInitiated)
    private var audioRunning = false
    private var videoRunning = false

    private let sampleRate: Int = 48000
    private let numChannels: Int = 1
    private let frameDurationMs: Int = 20
    private let videoFps: Double = 15.0

    init(filePath: String) {
        self.filePath = filePath
    }

    // MARK: - Audio

    func startAudio() {
        audioQueue.async { [self] in
            guard !audioRunning else { return }
            audioRunning = true
            NSLog("MediaFileCapture: audio start, file=%@", filePath)

            let samplesPerFrame = sampleRate * frameDurationMs / 1000  // 960

            while audioRunning {
                guard let (asset, reader, output) = createAudioReader() else {
                    NSLog("MediaFileCapture: failed to create audio reader, retrying in 1s")
                    Thread.sleep(forTimeInterval: 1.0)
                    continue
                }
                _ = asset  // keep asset alive

                reader.startReading()

                var buffer = [Int16](repeating: 0, count: samplesPerFrame)
                var residual = [Int16]()

                while audioRunning && reader.status == .reading {
                    guard let sampleBuffer = output.copyNextSampleBuffer() else { break }

                    guard let blockBuffer = CMSampleBufferGetDataBuffer(sampleBuffer) else { continue }

                    var lengthAtOffset: Int = 0
                    var totalLength: Int = 0
                    var dataPointer: UnsafeMutablePointer<Int8>?
                    let status = CMBlockBufferGetDataPointer(
                        blockBuffer,
                        atOffset: 0,
                        lengthAtOffsetOut: &lengthAtOffset,
                        totalLengthOut: &totalLength,
                        dataPointerOut: &dataPointer
                    )
                    guard status == kCMBlockBufferNoErr, let dataPointer else { continue }

                    let sampleCount = totalLength / MemoryLayout<Int16>.size
                    let samplesPtr = UnsafeRawPointer(dataPointer).bindMemory(to: Int16.self, capacity: sampleCount)

                    // Append decoded samples to residual buffer.
                    residual.append(contentsOf: UnsafeBufferPointer(start: samplesPtr, count: sampleCount))

                    // Push complete 960-sample frames.
                    while audioRunning && residual.count >= samplesPerFrame {
                        for i in 0..<samplesPerFrame {
                            buffer[i] = residual[i]
                        }
                        residual.removeFirst(samplesPerFrame)

                        buffer.withUnsafeBufferPointer { ptr in
                            guard let base = ptr.baseAddress else { return }
                            visio_push_ios_audio_frame(
                                base,
                                UInt32(samplesPerFrame),
                                UInt32(sampleRate),
                                UInt32(numChannels)
                            )
                        }

                        Thread.sleep(forTimeInterval: Double(frameDurationMs) / 1000.0)
                    }
                }

                reader.cancelReading()
                NSLog("MediaFileCapture: audio reached end of file, looping")
            }

            NSLog("MediaFileCapture: audio stopped")
        }
    }

    func stopAudio() {
        audioQueue.async { [self] in
            audioRunning = false
        }
    }

    // MARK: - Video

    func startVideo() {
        videoQueue.async { [self] in
            guard !videoRunning else { return }
            videoRunning = true
            NSLog("MediaFileCapture: video start, file=%@", filePath)

            let frameDuration = 1.0 / videoFps

            while videoRunning {
                guard let (asset, reader, output) = createVideoReader() else {
                    NSLog("MediaFileCapture: failed to create video reader, retrying in 1s")
                    Thread.sleep(forTimeInterval: 1.0)
                    continue
                }
                _ = asset  // keep asset alive

                reader.startReading()
                var frameCount: UInt64 = 0

                while videoRunning && reader.status == .reading {
                    let frameStart = CFAbsoluteTimeGetCurrent()

                    guard let sampleBuffer = output.copyNextSampleBuffer() else { break }
                    guard let pixelBuffer = CMSampleBufferGetImageBuffer(sampleBuffer) else { continue }

                    pushVideoFrame(pixelBuffer)
                    frameCount += 1

                    if frameCount % 30 == 1 {
                        let w = CVPixelBufferGetWidth(pixelBuffer)
                        let h = CVPixelBufferGetHeight(pixelBuffer)
                        NSLog("MediaFileCapture: video frame #%llu, %dx%d", frameCount, w, h)
                    }

                    // Pace to target fps.
                    let elapsed = CFAbsoluteTimeGetCurrent() - frameStart
                    let sleepTime = frameDuration - elapsed
                    if sleepTime > 0 {
                        Thread.sleep(forTimeInterval: sleepTime)
                    }
                }

                reader.cancelReading()
                NSLog("MediaFileCapture: video reached end of file (%llu frames), looping", frameCount)
            }

            NSLog("MediaFileCapture: video stopped")
        }
    }

    func stopVideo() {
        videoQueue.async { [self] in
            videoRunning = false
        }
    }

    // MARK: - Private helpers

    private func createAudioReader() -> (AVAsset, AVAssetReader, AVAssetReaderTrackOutput)? {
        let url = URL(fileURLWithPath: filePath)
        let asset = AVAsset(url: url)

        guard let audioTrack = asset.tracks(withMediaType: .audio).first else {
            NSLog("MediaFileCapture: no audio track in %@", filePath)
            return nil
        }

        let outputSettings: [String: Any] = [
            AVFormatIDKey: kAudioFormatLinearPCM,
            AVSampleRateKey: sampleRate,
            AVNumberOfChannelsKey: numChannels,
            AVLinearPCMBitDepthKey: 16,
            AVLinearPCMIsFloatKey: false,
            AVLinearPCMIsBigEndianKey: false,
        ]

        let output = AVAssetReaderTrackOutput(track: audioTrack, outputSettings: outputSettings)

        guard let reader = try? AVAssetReader(asset: asset) else {
            NSLog("MediaFileCapture: failed to create AVAssetReader for audio")
            return nil
        }

        if reader.canAdd(output) {
            reader.add(output)
        } else {
            NSLog("MediaFileCapture: cannot add audio output to reader")
            return nil
        }

        return (asset, reader, output)
    }

    private func createVideoReader() -> (AVAsset, AVAssetReader, AVAssetReaderTrackOutput)? {
        let url = URL(fileURLWithPath: filePath)
        let asset = AVAsset(url: url)

        guard let videoTrack = asset.tracks(withMediaType: .video).first else {
            NSLog("MediaFileCapture: no video track in %@", filePath)
            return nil
        }

        let outputSettings: [String: Any] = [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
        ]

        let output = AVAssetReaderTrackOutput(track: videoTrack, outputSettings: outputSettings)

        guard let reader = try? AVAssetReader(asset: asset) else {
            NSLog("MediaFileCapture: failed to create AVAssetReader for video")
            return nil
        }

        if reader.canAdd(output) {
            reader.add(output)
        } else {
            NSLog("MediaFileCapture: cannot add video output to reader")
            return nil
        }

        return (asset, reader, output)
    }

    /// Convert NV12 pixel buffer to I420 and push via FFI.
    private func pushVideoFrame(_ pixelBuffer: CVPixelBuffer) {
        CVPixelBufferLockBaseAddress(pixelBuffer, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(pixelBuffer, .readOnly) }

        let width = CVPixelBufferGetWidth(pixelBuffer)
        let height = CVPixelBufferGetHeight(pixelBuffer)
        let chromaW = width / 2
        let chromaH = height / 2

        guard let yBase = CVPixelBufferGetBaseAddressOfPlane(pixelBuffer, 0),
              let uvBase = CVPixelBufferGetBaseAddressOfPlane(pixelBuffer, 1) else { return }

        let yStride = CVPixelBufferGetBytesPerRowOfPlane(pixelBuffer, 0)
        let uvStride = CVPixelBufferGetBytesPerRowOfPlane(pixelBuffer, 1)

        let yPtr = yBase.assumingMemoryBound(to: UInt8.self)
        let uvPtr = uvBase.assumingMemoryBound(to: UInt8.self)

        // De-interleave NV12 UV plane into separate U and V planes (I420).
        var uPlane = [UInt8](repeating: 0, count: chromaW * chromaH)
        var vPlane = [UInt8](repeating: 0, count: chromaW * chromaH)

        for row in 0..<chromaH {
            let uvRow = uvPtr.advanced(by: row * uvStride)
            let dstOffset = row * chromaW
            for col in 0..<chromaW {
                uPlane[dstOffset + col] = uvRow[col * 2]
                vPlane[dstOffset + col] = uvRow[col * 2 + 1]
            }
        }

        uPlane.withUnsafeBufferPointer { uBuf in
            vPlane.withUnsafeBufferPointer { vBuf in
                guard let uBase = uBuf.baseAddress,
                      let vBase = vBuf.baseAddress else { return }
                visio_push_ios_camera_frame(
                    yPtr, UInt32(yStride),
                    uBase, UInt32(chromaW),
                    vBase, UInt32(chromaW),
                    UInt32(width), UInt32(height)
                )
            }
        }
    }
}
