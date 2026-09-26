import Foundation
import CoreServices

/// Native notifications are hints. Rust rescans persistently indexed sources.
final class FolderWatch {
  private var stream: FSEventStreamRef?
  private final class Callback {
    let changed: ([String], Bool) -> Void
    init(_ changed: @escaping ([String], Bool) -> Void) { self.changed = changed }
  }
  private let queue = DispatchQueue(label: "app.photobridge.folder-events")
  private(set) var active = false
  init(url: URL, changed: @escaping ([String], Bool) -> Void) {
    let callback = Unmanaged.passRetained(Callback(changed)).toOpaque()
    var context = FSEventStreamContext(version: 0, info: callback, retain: nil, release: { info in
      if let info { Unmanaged<Callback>.fromOpaque(info).release() }
    }, copyDescription: nil)
    stream = FSEventStreamCreate(nil, { _, info, count, paths, flags, _ in
      guard let info else { return }
      let values = Unmanaged<CFArray>.fromOpaque(paths).takeUnretainedValue() as! [String]
      let full = (0..<count).contains { flags[$0] & FSEventStreamEventFlags(kFSEventStreamEventFlagMustScanSubDirs | kFSEventStreamEventFlagRootChanged | kFSEventStreamEventFlagUserDropped | kFSEventStreamEventFlagKernelDropped) != 0 }
      Unmanaged<Callback>.fromOpaque(info).takeUnretainedValue().changed(values, full)
    }, &context, [url.path] as CFArray, FSEventStreamEventId(kFSEventStreamEventIdSinceNow), 2,
      FSEventStreamCreateFlags(kFSEventStreamCreateFlagUseCFTypes | kFSEventStreamCreateFlagWatchRoot | kFSEventStreamCreateFlagFileEvents))
    if let stream {
      FSEventStreamSetDispatchQueue(stream, queue)
      active = FSEventStreamStart(stream)
    } else { Unmanaged<Callback>.fromOpaque(callback).release() }
  }
  deinit { if let stream { FSEventStreamStop(stream); FSEventStreamInvalidate(stream); FSEventStreamRelease(stream) } }
}
