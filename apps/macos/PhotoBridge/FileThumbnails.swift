import AppKit
import QuickLookThumbnailing
import SwiftUI

/// Bounded requests and cache shared by folder and transfer rows.
@MainActor final class FileThumbnailPipeline {
  static let shared = FileThumbnailPipeline()
  private struct Work {
    let id: UUID
    let root: URL
    let file: URL
    let request: QLThumbnailGenerator.Request
    let key: NSString
    let completion: (NSImage?) -> Void
  }
  private var pending: [Work] = []
  private var active: [UUID: Work] = [:]
  private let cache = NSCache<NSString, NSImage>()
  init() { cache.totalCostLimit = 32 * 1024 * 1024; cache.countLimit = 256 }
  func request(id: UUID, source: FolderSource, relative: String, revision: String, size: CGFloat,
    completion: @escaping (NSImage?) -> Void) {
    var stale = false
    guard let root = try? URL(resolvingBookmarkData: source.bookmark,
      options: [.withSecurityScope, .withoutUI, .withoutMounting], relativeTo: nil,
      bookmarkDataIsStale: &stale),
      !relative.isEmpty, !relative.hasPrefix("/"),
      !relative.split(separator: "/").contains("..") else { completion(nil); return }
    let file = root.appendingPathComponent(relative).standardizedFileURL
    guard file.path.hasPrefix(root.standardizedFileURL.path + "/") else { completion(nil); return }
    let key = "\(source.id)|\(file.path)|\(revision)|\(size)" as NSString
    if let image = cache.object(forKey: key) { completion(image); return }
    guard pending.count < 128 else { completion(nil); return }
    pending.append(Work(id: id, root: root, file: file,
      request: QLThumbnailGenerator.Request(fileAt: file, size: CGSize(width: size, height: size),
        scale: 2, representationTypes: .thumbnail), key: key, completion: completion))
    pump()
  }
  func cancel(_ id: UUID) {
    pending.removeAll { $0.id == id }
    if let work = active.removeValue(forKey: id) {
      QLThumbnailGenerator.shared.cancel(work.request)
      work.root.stopAccessingSecurityScopedResource()
    }
    pump()
  }
  private func pump() {
    while active.count < 4, !pending.isEmpty {
      let work = pending.removeFirst()
      _ = work.root.startAccessingSecurityScopedResource()
      // Never follow a file symlink outside the registered source.
      guard work.file.resolvingSymlinksInPath().path == work.file.path else {
        work.root.stopAccessingSecurityScopedResource(); work.completion(nil); continue
      }
      active[work.id] = work
      let requestID = work.id
      QLThumbnailGenerator.shared.generateBestRepresentation(for: work.request) { representation, _ in
        Task { @MainActor in
          guard let completed = self.active.removeValue(forKey: requestID) else { return }
          completed.root.stopAccessingSecurityScopedResource()
          let image = representation?.nsImage
          if let image {
            self.cache.setObject(image, forKey: completed.key,
              cost: Int(completed.request.size.width * completed.request.size.height * 16))
          }
          completed.completion(image)
          self.pump()
        }
      }
    }
  }
}

struct MacFileThumbnail: View {
  let source: FolderSource
  let relative: String
  let revision: String
  var video = false
  var size: CGFloat = 56
  @AppStorage("macListThumbnails") private var enabled = true
  @State private var image: NSImage?
  @State private var token = UUID()
  var body: some View {
    ZStack {
      RoundedRectangle(cornerRadius: 8).fill(.quaternary)
      if enabled, let image { Image(nsImage: image).resizable().scaledToFit() }
      else { Image(systemName: video ? "video" : "photo").foregroundStyle(.secondary) }
    }.frame(width: size, height: size).clipped().clipShape(RoundedRectangle(cornerRadius: 8))
      .task(id: "\(enabled)|\(source.id)|\(relative)|\(revision)|\(size)") {
        FileThumbnailPipeline.shared.cancel(token)
        image = nil; token = UUID()
        guard enabled else { return }
        let expected = token
        FileThumbnailPipeline.shared.request(id: expected, source: source, relative: relative,
          revision: revision, size: size) { result in
            guard token == expected else { return }; image = result
          }
      }.onDisappear { FileThumbnailPipeline.shared.cancel(token); token = UUID() }
  }
}

struct MacTransferThumbnail: View {
  let job: BackupJob
  @ObservedObject private var folders = FolderSources.shared
  @AppStorage("macListThumbnails") private var enabled = true
  @State private var entry: FolderEntry?
  var body: some View {
    Group {
      if let entry, let source = folders.sources.first(where: { $0.id == job.asset.metadata?["source_ref"] }) {
        MacFileThumbnail(source: source, relative: entry.relative, revision: entry.revision,
          video: job.asset.kind == "video")
      } else {
        Image(systemName: job.asset.kind == "video" ? "video" : "photo")
          .foregroundStyle(.secondary).frame(width: 56, height: 56)
          .background(.quaternary, in: RoundedRectangle(cornerRadius: 8))
      }
    }.task(id: "\(enabled)|\(job.id)|\(folders.indexRevision)|\(folders.sources.map { $0.id }.joined(separator: "|"))") {
      entry = nil
      guard enabled, let source = job.asset.metadata?["source_ref"],
        folders.sources.contains(where: { $0.id == source }),
        let value = try? await folders.previewEntry(source, job: job.id, sourceID: job.asset.source_id),
        !Task.isCancelled, value.source_id == job.asset.source_id,
        (value.revision == job.asset.revision || job.asset.revision.hasPrefix(value.revision + "|")) else { return }
      entry = value
    }
  }
}
