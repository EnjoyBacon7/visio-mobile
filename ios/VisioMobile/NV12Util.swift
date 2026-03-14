import CoreVideo
import visioFFI

/// Push an NV12 CVPixelBuffer to Rust as I420 via the C FFI.
///
/// Optionally accepts pre-allocated U/V plane buffers (resized if needed)
/// to avoid per-frame heap allocations on hot paths.
func pushNV12FrameToRust(
    _ pixelBuffer: CVPixelBuffer,
    uPlane: inout [UInt8],
    vPlane: inout [UInt8]
) {
    CVPixelBufferLockBaseAddress(pixelBuffer, .readOnly)
    defer { CVPixelBufferUnlockBaseAddress(pixelBuffer, .readOnly) }

    let width = CVPixelBufferGetWidth(pixelBuffer)
    let height = CVPixelBufferGetHeight(pixelBuffer)
    let chromaW = width / 2
    let chromaH = height / 2
    let chromaSize = chromaW * chromaH

    guard let yBase = CVPixelBufferGetBaseAddressOfPlane(pixelBuffer, 0),
          let uvBase = CVPixelBufferGetBaseAddressOfPlane(pixelBuffer, 1) else { return }

    let yStride = CVPixelBufferGetBytesPerRowOfPlane(pixelBuffer, 0)
    let uvStride = CVPixelBufferGetBytesPerRowOfPlane(pixelBuffer, 1)

    let yPtr = yBase.assumingMemoryBound(to: UInt8.self)
    let uvPtr = uvBase.assumingMemoryBound(to: UInt8.self)

    // Resize buffers only when resolution changes.
    if uPlane.count != chromaSize {
        uPlane = [UInt8](repeating: 0, count: chromaSize)
        vPlane = [UInt8](repeating: 0, count: chromaSize)
    }

    // De-interleave NV12 UV plane into separate U and V planes (I420).
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
