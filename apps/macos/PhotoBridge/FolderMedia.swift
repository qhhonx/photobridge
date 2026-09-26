import Foundation
import AVFoundation
import ImageIO

enum FolderMediaError: LocalizedError {
  case unsupported
  var errorDescription: String? { NSLocalizedString("folder_unsupported_media", comment: "") }
}

struct FolderEntry: Decodable, Identifiable {
  let relative: String
  let source_id: String
  let revision: String
  let size: UInt64
  let media_type: String
  let modified_ms: UInt64
  var state: String?
  var id: String { relative }
}
enum FolderMedia {
  struct Prepared { let primary: String; let paired: String?; let metadata: [String: String] }
  struct ImageInfo { let date: Date?; let identifier: String? }
  static func image(_ url: URL) throws -> ImageInfo {
    guard let source = CGImageSourceCreateWithURL(url as CFURL, nil),
      let fields = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [String: Any] else { throw FolderMediaError.unsupported }
    let exif = fields[kCGImagePropertyExifDictionary as String] as? [String: Any] ?? [:]
    let apple = fields[kCGImagePropertyMakerAppleDictionary as String] as? [String: Any]
    let formatter = DateFormatter(); formatter.locale = Locale(identifier: "en_US_POSIX")
    formatter.dateFormat = "yyyy:MM:dd HH:mm:ss"
    if let offset = exif["OffsetTimeOriginal"] as? String {
      let parts = offset.split(separator: ":"); if parts.count == 2, let h = Int(parts[0]), let m = Int(parts[1]) { formatter.timeZone = TimeZone(secondsFromGMT: h * 3600 + (offset.hasPrefix("-") ? -m : m) * 60) }
    }
    return ImageInfo(date: (exif[kCGImagePropertyExifDateTimeOriginal as String] as? String).flatMap(formatter.date(from:)), identifier: apple?["17"] as? String)
  }
  static func video(_ url: URL) async throws -> (Date?, String?) {
    let asset = AVURLAsset(url: url)
    let metadata = try await asset.load(.metadata)
    var identifier: String?
    for item in metadata where item.identifier?.rawValue == "mdta/com.apple.quicktime.content.identifier" {
      identifier = try await item.load(.stringValue)
    }
    let creation = try await asset.load(.creationDate)
    var date = try await creation?.load(.dateValue)
    if date == nil, let text = try await creation?.load(.stringValue) { date = ISO8601DateFormatter().date(from: text) }
    return (date, identifier)
  }
  static func prepare(root: URL, entry: FolderEntry) async throws -> Prepared {
    let url = root.appendingPathComponent(entry.relative).standardizedFileURL
    guard url.resolvingSymlinksInPath().path == url.path else { throw FolderMediaError.unsupported }
    let isVideo = entry.media_type.hasPrefix("video/")
    let info: ImageInfo
    let videoInfo: (Date?, String?)
    if isVideo { info = ImageInfo(date: nil, identifier: nil); videoInfo = try await video(url) }
    else {
      info = try await Task.detached(priority: .utility) { try image(url) }.value
      videoInfo = (nil, nil)
    }
    let stem = url.deletingPathExtension()
    var primary = entry.relative; var paired: String?
    var date = isVideo ? videoInfo.0 : info.date
    // Same basename is only a candidate; Apple pairing identifiers must agree.
    if let identifier = isVideo ? videoInfo.1 : info.identifier {
      let extensions = isVideo ? ["HEIC", "heic", "JPG", "jpg", "JPEG", "jpeg"] : ["MOV", "mov"]
      for ext in extensions {
        let sibling = stem.appendingPathExtension(ext)
        guard FileManager.default.fileExists(atPath: sibling.path), sibling.resolvingSymlinksInPath().path == sibling.path else { continue }
        if isVideo {
          let imageInfo = try? await Task.detached(priority: .utility) { try image(sibling) }.value
          if imageInfo?.identifier == identifier {
            primary = String(sibling.path.dropFirst(root.path.count + 1)); paired = entry.relative; date = imageInfo?.date; break
          }
        } else if let companion = try? await video(sibling), companion.1 == identifier {
          paired = String(sibling.path.dropFirst(root.path.count + 1)); break
        }
      }
    }
    // Fallback is explicit, never the upload date. Originals remain unchanged.
    let capture = date ?? Date(timeIntervalSince1970: Double(entry.modified_ms) / 1000)
    return Prepared(primary: primary, paired: paired, metadata: ["created_at_ms": String(Int64(capture.timeIntervalSince1970 * 1000)), "date_source": date == nil ? "file_modified" : "embedded"])
  }
}
