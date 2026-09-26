// Native file metadata and real filesystem notifications, using synthetic media.
import Foundation
import ImageIO
import CoreGraphics
import UniformTypeIdentifiers
import AVFoundation

@main struct FolderNativeTests {
  @MainActor static func main() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("photobridge-folder-native-" + UUID().uuidString).resolvingSymlinksInPath()
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let photo = root.appendingPathComponent("sample.jpg")
    let context = CGContext(data: nil, width: 32, height: 32, bitsPerComponent: 8, bytesPerRow: 128, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
    let destination = CGImageDestinationCreateWithURL(photo as CFURL, UTType.jpeg.identifier as CFString, 1, nil)!
    CGImageDestinationAddImage(destination, context.makeImage()!, [kCGImagePropertyExifDictionary: [kCGImagePropertyExifDateTimeOriginal: "2000:01:02 12:34:56", "OffsetTimeOriginal": "+05:30"]] as CFDictionary)
    precondition(CGImageDestinationFinalize(destination))
    let info = try FolderMedia.image(photo)
    let expected = ISO8601DateFormatter().date(from: "2000-01-02T07:04:56Z")!
    precondition(abs(info.date!.timeIntervalSince(expected)) < 1, "EXIF timezone was lost")
    let before = try Data(contentsOf: photo)
    let entry = FolderEntry(relative: "sample.jpg", source_id: "fixture", revision: "1", size: UInt64(before.count), media_type: "image/jpeg", modified_ms: 1_600_000_000_000)
    let prepared = try await FolderMedia.prepare(root: root, entry: entry)
    precondition(prepared.paired == nil && prepared.metadata["date_source"] == "embedded")
    precondition(prepared.metadata["created_at_ms"] == String(Int64(expected.timeIntervalSince1970 * 1000)))
    let after = try Data(contentsOf: photo)
    precondition(after == before, "Metadata inspection modified source bytes")
    // Apple paired-video identity and capture time are read without conversion.
    let movie = root.appendingPathComponent("sample.mov")
    let writer = try AVAssetWriter(outputURL: movie, fileType: .mov)
    let input = AVAssetWriterInput(mediaType: .video, outputSettings: [AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: 32, AVVideoHeightKey: 32])
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32ARGB, kCVPixelBufferWidthKey as String: 32, kCVPixelBufferHeightKey as String: 32])
    writer.add(input)
    let identity = AVMutableMetadataItem(); identity.identifier = AVMetadataIdentifier(rawValue: "mdta/com.apple.quicktime.content.identifier"); identity.value = "synthetic-pair-id" as NSString; identity.dataType = kCMMetadataBaseDataType_UTF8 as String
    let capture = AVMutableMetadataItem(); capture.identifier = .quickTimeMetadataCreationDate; capture.value = "2000-01-02T07:04:56Z" as NSString; capture.dataType = kCMMetadataBaseDataType_UTF8 as String
    writer.metadata = [identity, capture]
    precondition(writer.startWriting()); writer.startSession(atSourceTime: .zero)
    while !input.isReadyForMoreMediaData { try? await Task.sleep(nanoseconds: 1_000_000) }
    var pixel: CVPixelBuffer?
    precondition(CVPixelBufferPoolCreatePixelBuffer(nil, adaptor.pixelBufferPool!, &pixel) == kCVReturnSuccess)
    CVPixelBufferLockBaseAddress(pixel!, [])
    CVPixelBufferGetBaseAddress(pixel!)!.initializeMemory(as: UInt8.self, repeating: 0, count: CVPixelBufferGetBytesPerRow(pixel!) * CVPixelBufferGetHeight(pixel!))
    CVPixelBufferUnlockBaseAddress(pixel!, [])
    precondition(adaptor.append(pixel!, withPresentationTime: .zero))
    writer.endSession(atSourceTime: CMTime(value: 1, timescale: 30)); input.markAsFinished()
    await writer.finishWriting(); precondition(writer.status == .completed)
    let videoInfo = try await FolderMedia.video(movie)
    precondition(videoInfo.1 == "synthetic-pair-id", "Apple video pairing identity was lost")
    precondition(abs(videoInfo.0!.timeIntervalSince(expected)) < 1, "Video capture date was lost")
    let invalid = root.appendingPathComponent("invalid.jpg")
    try Data("not an image".utf8).write(to: invalid)
    do { _ = try FolderMedia.image(invalid); fatalError("Invalid photo accepted") } catch {}
    final class Counter {
      let lock = NSLock(); var value = 0
      func increment() { lock.lock(); value += 1; lock.unlock() }
      func read() -> Int { lock.lock(); defer { lock.unlock() }; return value }
    }
    let notifications = Counter()
    let watcher = FolderWatch(url: root) { paths, _ in
      if paths.contains(where: { URL(fileURLWithPath: $0).resolvingSymlinksInPath().path.hasPrefix(root.path) }) { notifications.increment() }
    }
    precondition(watcher.active, "FSEventStream could not start")
    try? await Task.sleep(nanoseconds: 200_000_000)
    try Data("synthetic change".utf8).write(to: root.appendingPathComponent("new.jpg"))
    for _ in 0..<40 {
      if notifications.read() > 0 { break }
      try? await Task.sleep(nanoseconds: 200_000_000)
    }
    withExtendedLifetime(watcher) { precondition(notifications.read() > 0, "FSEvents did not report a real folder change") }
    print("Native folder checks passed: EXIF timezone, Apple video identity/date, source preservation, invalid media and live FSEvents")
  }
}
