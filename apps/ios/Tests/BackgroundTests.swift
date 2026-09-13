import AVFoundation
import Photos
import XCTest

@testable import PhotoBridge

@MainActor final class BackgroundTests: XCTestCase {
  func testNativeBackgroundSessionAndNewPhotoDiscovery() async throws {
    // Install the exact test host, then grant Photos before running this test.
    // Fail immediately instead of waiting forever on a permission dialog.
    guard PHPhotoLibrary.authorizationStatus(for: .readWrite) == .authorized else {
      XCTFail("Grant Photos on the disposable simulator after installing the test host")
      return
    }
    let model = BackupModel.shared
    await model.open()
    XCTAssertTrue(model.ready)
    let startupLog = try JSONDecoder().decode([ActivityEntry].self,
      from: await Bridge.call(["op": "activity_log", "receiver": false]))
    let startup = try XCTUnwrap(startupLog.first { $0.code == "sender_ready" }?.context)
    XCTAssertNotNil(startup.execution)
    XCTAssertNotNil(startup.paused)
    XCTAssertNotNil(startup.active_requests)
    await model.setPaused(true)
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(
      "native-integration-" + UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    let pairingData = try await Bridge.call([
      "op": "start_receiver", "root": root.appendingPathComponent("receiver").path,
      "listen": "127.0.0.1:18484", "capacity": 32 * 1024 * 1024,
    ])
    let pairing = try JSONDecoder().decode(Pairing.self, from: pairingData)
    try Keychain.save(pairingData)
    model.pairing = pairing
    _ = try await Bridge.call(["op": "check_pairing", "pairing": try JSONSerialization.jsonObject(with: pairingData)])
    var resources: [[String: Any]] = []
    for (role, file, mime, size) in [
      ("photo", "sample.heic", "image/heic", 4 * 1024 * 1024 + 37),
      ("paired_video", "sample.mov", "video/quicktime", 31),
    ] {
      let path = root.appendingPathComponent(file)
      try Data(repeating: role == "photo" ? 7 : 9, count: size).write(to: path)
      resources.append(["role": role, "filename": file, "media_type": mime, "path": path.path])
    }
    let request: [String: Any] = [
      "op": "enqueue", "receiver_id": pairing.receiverID,
      "source_id": "integration-" + UUID().uuidString, "revision": "1", "kind": "motion",
      "resources": resources,
    ]
    let first = try JSONDecoder().decode(BackupJob.self, from: await Bridge.call(request))
    await BackgroundTransfer.shared.kick()
    try await waitUntil {
      await model.refresh()
      return model.jobs.first(where: { $0.id == first.id })?.state == "queued"
    }
    // A paused sender must explain its idle state without silently dropping work.
    await BackgroundTransfer.shared.recordSnapshot("app_background", execution: "background")
    let pausedLog = try JSONDecoder().decode([ActivityEntry].self,
      from: await Bridge.call(["op": "activity_log", "receiver": false]))
    let snapshot = try XCTUnwrap(pausedLog.first { $0.code == "app_background" }?.context)
    XCTAssertEqual(snapshot.paused, true)
    XCTAssertGreaterThanOrEqual(snapshot.queued ?? 0, 1)
    XCTAssertEqual(snapshot.execution, "background")
    XCTAssertEqual(snapshot.active_requests, 0)
    await model.setPaused(false)
    try await waitUntil {
      await model.refresh()
      return model.jobs.first(where: { $0.id == first.id })?.state == "received"
    }
    XCTAssertEqual(
      model.jobs.first(where: { $0.id == first.id })?.confirmedBytes, 4 * 1024 * 1024 + 68)
    let journalData = try await Bridge.call(["op": "activity_log", "receiver": false])
    let journal = try JSONDecoder().decode([ActivityEntry].self, from: journalData)
    let codes = Set(journal.filter { $0.job_id == first.id }.map(\.code))
    XCTAssertTrue(codes.isSuperset(of: ["bundle_submitted", "request_completed"]))
    XCTAssertFalse(String(decoding: journalData, as: UTF8.self).contains("sample.heic"))
    XCTAssertFalse(String(decoding: journalData, as: UTF8.self).contains(pairing.token))
    let completion = try XCTUnwrap(journal.first { $0.job_id == first.id && $0.code == "request_completed" }?.context)
    XCTAssertEqual(completion.phase, "bundle")
    XCTAssertNotNil(completion.http_status)
    XCTAssertNil(completion.system_error)
    await BackgroundTransfer.shared.kick()
    let idleLog = try JSONDecoder().decode([ActivityEntry].self,
      from: await Bridge.call(["op": "activity_log", "receiver": false]))
    let idle = try XCTUnwrap(idleLog.first { $0.code == "dispatch_waiting" }?.context)
    XCTAssertEqual(idle.reason, "no_eligible_job")
    XCTAssertEqual(idle.queued, 0)
    XCTAssertEqual(idle.active_requests, 0)
    // Repeated polling of the same state must not flood the bounded log.
    await BackgroundTransfer.shared.kick()
    let repeatedLog = try JSONDecoder().decode([ActivityEntry].self,
      from: await Bridge.call(["op": "activity_log", "receiver": false]))
    XCTAssertEqual(repeatedLog.filter { $0.code == "dispatch_waiting" }.count,
      idleLog.filter { $0.code == "dispatch_waiting" }.count)
    let duplicate = try JSONDecoder().decode(BackupJob.self, from: await Bridge.call(request))
    XCTAssertEqual(first.id, duplicate.id)

    // Authentication failures require a repaired pairing, not an infinite retry loop.
    let invalid = Pairing(
      version: pairing.version, receiverID: pairing.receiverID,
      endpoint: pairing.endpoint, certificate: pairing.certificate,
      token: String(repeating: "0", count: 64))
    try Keychain.save(JSONEncoder().encode(invalid))
    model.pairing = invalid
    var badRequest = request
    badRequest["source_id"] = "authentication-" + UUID().uuidString
    let bad = try JSONDecoder().decode(BackupJob.self, from: await Bridge.call(badRequest))
    await BackgroundTransfer.shared.kick()
    try await waitUntil {
      await model.refresh()
      return model.jobs.first(where: { $0.id == bad.id })?.state == "failed"
    }
    XCTAssertEqual(model.jobs.first(where: { $0.id == bad.id })?.errorCode, "authentication")
    try Keychain.save(pairingData)
    model.pairing = pairing
    await model.retry(bad.id)
    try await waitUntil {
      await model.refresh()
      return model.jobs.first(where: { $0.id == bad.id })?.state == "received"
    }

    // Persistent history must see a newly imported photo with an old capture date.
    // Run only on a disposable simulator whose Photos permission is pre-granted.
    XCTAssertEqual(PHPhotoLibrary.authorizationStatus(for: .readWrite), .authorized)
    await model.setAutoBackup(true)
    XCTAssertTrue(model.autoBackup)
    let before = Set(model.jobs.map(\.id))
    let image = UIGraphicsImageRenderer(size: CGSize(width: 32, height: 32)).image { context in
      UIColor.systemGreen.setFill()
      context.fill(CGRect(x: 0, y: 0, width: 32, height: 32))
    }
    try await PHPhotoLibrary.shared().performChanges {
      let creation = PHAssetChangeRequest.creationRequestForAsset(from: image)
      creation.creationDate = Date(timeIntervalSince1970: 1_000_000)
    }
    try await waitUntil {
      await model.discoverPhotos()
      await model.refresh()
      return model.jobs.contains { !before.contains($0.id) && $0.state == "received" }
    }
    // The actual PhotoKit export must be reclaimed only after its complete receipt.
    let queuedData = try await Bridge.call(["op": "jobs"])
    let queued = try XCTUnwrap(JSONSerialization.jsonObject(with: queuedData) as? [[String: Any]])
    let discovered = try XCTUnwrap(
      queued.first { row in
        guard let id = row["id"] as? Int64 else { return false }
        return !before.contains(id) && row["state"] as? String == "received"
      })
    let sources = try XCTUnwrap(discovered["sources"] as? [String: String])
    XCTAssertFalse(sources.isEmpty)
    for source in sources.values { XCTAssertFalse(FileManager.default.fileExists(atPath: source)) }
    let count = model.jobs.count
    await model.discoverPhotos()
    await model.refresh()
    XCTAssertEqual(model.jobs.count, count)
    await model.setAutoBackup(false)
    await model.setPaused(true)
    _ = try await Bridge.call(["op": "stop_receiver"])
    // Same endpoint now presents a different certificate. No photo data may be
    // accepted and cancellation of the trust challenge must not become a retry.
    _ = try await Bridge.call([
      "op": "start_receiver", "root": root.appendingPathComponent("replacement").path,
      "listen": "127.0.0.1:18484", "capacity": 32 * 1024 * 1024,
    ])
    var untrustedRequest = request
    untrustedRequest["source_id"] = "wrong-certificate-" + UUID().uuidString
    let untrusted = try JSONDecoder().decode(
      BackupJob.self, from: await Bridge.call(untrustedRequest))
    await model.setPaused(false)
    try await waitUntil {
      await model.refresh()
      return model.jobs.first(where: { $0.id == untrusted.id })?.state == "failed"
    }
    XCTAssertEqual(model.jobs.first(where: { $0.id == untrusted.id })?.errorCode, "authentication")
    XCTAssertEqual(model.jobs.first(where: { $0.id == untrusted.id })?.confirmedBytes, 0)
    await model.setPaused(true)
    _ = try await Bridge.call(["op": "stop_receiver"])
    try FileManager.default.removeItem(at: root)
  }
  func testBonjourFindsMovedPhysicalReceiver() async throws {
    struct Fixture: Decodable { let saved: Pairing; let endpoint: String }
    let path = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("receiver-discovery-fixture.json")
    guard FileManager.default.fileExists(atPath: path.path) else {
      throw XCTSkip("Install the isolated Android discovery fixture before this cross-device test")
    }
    defer { try? FileManager.default.removeItem(at: path) }
    let fixture = try JSONDecoder().decode(Fixture.self, from: Data(contentsOf: path))
    let model = BackupModel.shared
    await model.open(); await model.setPaused(true)
    try Keychain.save(JSONEncoder().encode(fixture.saved))
    model.pairing = fixture.saved
    let discovery = ReceiverDiscovery(model: model)
    discovery.update(pairing: fixture.saved, active: true)
    defer { discovery.update(pairing: nil, active: false) }
    try await waitUntil { model.pairing?.endpoint == fixture.endpoint }
    XCTAssertTrue(model.paused)
    XCTAssertEqual(model.pairing?.receiverID, fixture.saved.receiverID)
    XCTAssertTrue(model.pairing?.certificate == fixture.saved.certificate)
    let stored = try JSONDecoder().decode(Pairing.self, from: XCTUnwrap(Keychain.read()))
    XCTAssertEqual(stored.endpoint, fixture.endpoint)
    XCTAssertEqual(stored.certificateName, "127.0.0.1")
    // Exercise URLSession's saved certificate name against the physical Wi-Fi IP,
    // not just the Rust probe. Only synthetic bytes enter the isolated receiver.
    let file = FileManager.default.temporaryDirectory.appendingPathComponent("bonjour-" + UUID().uuidString + ".jpg")
    try Data(repeating: 4, count: 128 * 1024 + 7).write(to: file)
    defer { try? FileManager.default.removeItem(at: file) }
    let job = try JSONDecoder().decode(BackupJob.self, from: await Bridge.call([
      "op": "enqueue", "receiver_id": stored.receiverID, "source_id": "bonjour-" + UUID().uuidString,
      "revision": "1", "kind": "photo", "resources": [["role": "photo", "filename": "synthetic.jpg", "media_type": "image/jpeg", "path": file.path]]]))
    await model.setPaused(false)
    try await waitUntil {
      await model.refresh()
      return model.jobs.first { $0.id == job.id }?.state == "received"
    }
    await model.setPaused(true)
    XCTAssertEqual(model.jobs.first { $0.id == job.id }?.confirmedBytes, 128 * 1024 + 7)
  }
  func testPersistedOldRouteCallbackClassification() throws {
    let pairing = Pairing(version: 1, receiverID: "known", endpoint: "https://192.168.1.3:8484", certificate: "fixture", token: "fixture")
    let attempt = NativeAttempt(jobID: 1, generation: 2, receiverID: "known")
    let restored = try JSONDecoder().decode(NativeAttempt.self, from: JSONEncoder().encode(attempt))
    XCTAssertTrue(BackgroundTransfer.isOldRoute(receiverID: restored.receiverID, originalURL: URL(string: "https://192.168.1.2:8484/v1/assets"), current: pairing))
    XCTAssertFalse(BackgroundTransfer.isOldRoute(receiverID: "another", originalURL: URL(string: "https://192.168.1.2:8484/v1/assets"), current: pairing))
    XCTAssertFalse(BackgroundTransfer.isOldRoute(receiverID: "known", originalURL: URL(string: "https://192.168.1.3:8484/v1/assets"), current: pairing))
    let legacy = try JSONDecoder().decode(NativeAttempt.self, from: Data("{\"job_id\":1,\"generation\":2}".utf8))
    XCTAssertNil(legacy.receiverID)
  }
  func testReceiverRouteUpdatePreservesPauseAndQueue() async throws {
    let model = BackupModel.shared
    await model.open()
    await model.setPaused(true)
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("route-test-" + UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    let receiverRoot = root.appendingPathComponent("receiver").path
    let firstData = try await Bridge.call(["op": "start_receiver", "root": receiverRoot,
      "listen": "127.0.0.1:18486", "capacity": 32 * 1024 * 1024])
    let original = try JSONDecoder().decode(Pairing.self, from: firstData)
    await model.pair(String(decoding: firstData, as: UTF8.self))
    let file = root.appendingPathComponent("synthetic.jpg")
    try Data(repeating: 9, count: 4 * 1024 * 1024 + 31).write(to: file)
    let command: [String: Any] = ["op": "enqueue", "receiver_id": original.receiverID,
      "source_id": "route-test-" + UUID().uuidString, "revision": "1", "kind": "photo",
      "resources": [["role": "photo", "filename": "synthetic.jpg", "media_type": "image/jpeg", "path": file.path]]]
    let queued = try JSONDecoder().decode(BackupJob.self, from: await Bridge.call(command))
    // Reproduce an old-route authentication failure without transmitting media.
    _ = try await Bridge.call(["op": "pause_sender", "paused": false])
    let pending = try JSONDecoder().decode(NativeRequest.self, from: await Bridge.call(["op": "prepare_native", "receiver_id": original.receiverID]))
    _ = try await Bridge.call(["op": "bind_native", "attempt": pending.attempt.object, "task_id": "route-fixture"])
    _ = try await Bridge.call(["op": "finish_native", "attempt": pending.attempt.object, "task_id": "route-fixture", "status_code": 401, "body": ""])
    _ = try await Bridge.call(["op": "pause_sender", "paused": true])
    _ = try await Bridge.call(["op": "stop_receiver"])
    let newData = try await Bridge.call(["op": "start_receiver", "root": receiverRoot,
      "listen": "127.0.0.1:18487", "capacity": 32 * 1024 * 1024])
    let relocated = try JSONDecoder().decode(Pairing.self, from: newData)
    XCTAssertEqual(relocated.receiverID, original.receiverID)
    XCTAssertEqual(relocated.certificateName, "127.0.0.1")
    await model.relocateReceiver(to: relocated.endpoint, expected: original)
    XCTAssertEqual(model.pairing?.endpoint, relocated.endpoint)
    XCTAssertTrue(model.paused)
    let saved = try JSONDecoder().decode(Pairing.self, from: XCTUnwrap(Keychain.read()))
    XCTAssertEqual(saved.endpoint, relocated.endpoint)
    XCTAssertTrue(saved.certificate == original.certificate && saved.token == original.token)
    await model.refresh()
    XCTAssertEqual(model.jobs.first { $0.id == queued.id }?.state, "queued")
    await model.setPaused(false)
    try await waitUntil {
      await model.refresh()
      return model.jobs.first { $0.id == queued.id }?.state == "received"
    }
    await model.setPaused(true)
    let replay = try JSONDecoder().decode(BackupJob.self, from: await Bridge.call(command))
    XCTAssertEqual(replay.id, queued.id)
    XCTAssertEqual(replay.state, "received")
    _ = try await Bridge.call(["op": "stop_receiver"])
    _ = try await Bridge.call(["op": "start_receiver", "root": root.appendingPathComponent("impostor").path,
      "listen": "127.0.0.1:18488", "capacity": 32 * 1024 * 1024])
    await model.relocateReceiver(to: "https://127.0.0.1:18488", expected: saved)
    XCTAssertEqual(model.pairing?.endpoint, saved.endpoint)
    XCTAssertTrue(model.paused)
    let log = try JSONDecoder().decode([ActivityEntry].self, from: await Bridge.call(["op": "activity_log", "receiver": false]))
    XCTAssertTrue(log.contains { $0.code == "receiver_address_updated" })
    _ = try await Bridge.call(["op": "stop_receiver"])
    try FileManager.default.removeItem(at: root)
  }
  func testBoundedExportDoesNotWriteBeyondItsAllowance() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let settings = StorageSettings(
      cache_budget_bytes: 100, receiver_budget_bytes: 100, min_free_bytes: 0, auto_reclaim: true,
      log_days: 14, log_limit: 5000)
    let snapshot = StorageSnapshot(
      settings: settings, used_bytes: 0, free_bytes: 1 << 30, export_allowance: 8, reason: nil)
    let path = root.appendingPathComponent("original.jpg")
    let writer = try BoundedExportWriter(url: path, snapshot: snapshot)
    writer.consume(Data(repeating: 1, count: 5))
    writer.consume(Data(repeating: 2, count: 4))
    XCTAssertEqual((writer.finish(nil) as? Bridge.Failure)?.code, "local_cache_budget")
    XCTAssertEqual(try Data(contentsOf: path).count, 5)
  }
  private func waitUntil(_ condition: () async -> Bool) async throws {
    let deadline = Date().addingTimeInterval(90)
    while Date() < deadline {
      if await condition() { return }
      try await Task.sleep(nanoseconds: 300_000_000)
    }
    XCTFail("Timed out waiting for receiver acknowledgement; inspect non-sensitive job state")
    throw Bridge.Failure(code: "network")
  }
}

@MainActor final class BackgroundPreparationTests: XCTestCase {
  func testVideoOriginalExportAndTransfer() async throws {
    #if !targetEnvironment(simulator)
      throw XCTSkip("Synthetic media is simulator-only")
    #endif
    let access = await PHPhotoLibrary.requestAuthorization(for: .readWrite)
    XCTAssertEqual(access, .authorized)
    guard access == .authorized else { return }
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("video-acceptance-" + UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    let video = root.appendingPathComponent("fixture.mp4")
    let writer = try AVAssetWriter(outputURL: video, fileType: .mp4)
    let input = AVAssetWriterInput(mediaType: .video, outputSettings: [AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: 64, AVVideoHeightKey: 64])
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32ARGB, kCVPixelBufferWidthKey as String: 64, kCVPixelBufferHeightKey as String: 64])
    writer.add(input)
    XCTAssertTrue(writer.startWriting())
    writer.startSession(atSourceTime: .zero)
    for frame in 0..<10 {
      while !input.isReadyForMoreMediaData { try await Task.sleep(nanoseconds: 10_000_000) }
      var buffer: CVPixelBuffer?
      XCTAssertEqual(CVPixelBufferCreate(kCFAllocatorDefault, 64, 64, kCVPixelFormatType_32ARGB, nil, &buffer), kCVReturnSuccess)
      let pixels = try XCTUnwrap(buffer)
      CVPixelBufferLockBaseAddress(pixels, [])
      memset(CVPixelBufferGetBaseAddress(pixels), Int32(frame * 20), CVPixelBufferGetDataSize(pixels))
      CVPixelBufferUnlockBaseAddress(pixels, [])
      XCTAssertTrue(adaptor.append(pixels, withPresentationTime: CMTime(value: Int64(frame), timescale: 5)))
    }
    input.markAsFinished()
    await writer.finishWriting()
    XCTAssertEqual(writer.status, .completed)
    var source: String?
    try await PHPhotoLibrary.shared().performChanges {
      source = PHAssetChangeRequest.creationRequestForAssetFromVideo(atFileURL: video)?.placeholderForCreatedAsset?.localIdentifier
    }
    let id = try XCTUnwrap(source)
    let model = BackupModel.shared
    await model.open()
    await model.setPaused(true)
    await model.setAutoBackup(false)
    _ = try? await Bridge.call(["op": "stop_receiver"])
    let data = try await Bridge.call(["op": "start_receiver", "root": root.appendingPathComponent("receiver").path, "listen": "127.0.0.1:18485", "capacity": 32 * 1024 * 1024])
    let pairing = try JSONDecoder().decode(Pairing.self, from: data)
    try Keychain.save(data)
    model.pairing = pairing
    _ = try await Bridge.call(["op": "check_pairing", "pairing": try JSONSerialization.jsonObject(with: data)])
    await model.setPaused(false)
    let imported = await model.importAssets([id], requestAuthorization: false)
    XCTAssertTrue(imported)
    let deadline = Date().addingTimeInterval(45)
    repeat {
      await BackgroundTransfer.shared.kick()
      await model.refresh()
      if model.jobs.contains(where: { $0.asset.source_id == id && $0.state == "received" }) { break }
      try await Task.sleep(nanoseconds: 200_000_000)
    } while Date() < deadline
    let job = try XCTUnwrap(model.jobs.first(where: { $0.asset.source_id == id }))
    XCTAssertEqual(job.asset.kind, "video")
    XCTAssertEqual(job.state, "received")
    XCTAssertGreaterThan(job.confirmedBytes, 0)
    await model.setPaused(true)
    _ = try await Bridge.call(["op": "stop_receiver"])
  }
  func testContinuesPastOneBatchAndStopsWhenDrained() async {
    var pending = 27
    var rounds = 0
    await BackgroundPreparationLoop.run(allowed: { true }, step: {
      guard pending > 0 else { return .finished }
      pending -= min(5, pending)
      rounds += 1
      return .progressed
    })
    XCTAssertEqual(pending, 0)
    XCTAssertEqual(rounds, 6)
  }
  func testWaitsForCapacityAndHonorsPause() async {
    var paused = false
    var steps = 0
    var waits = 0
    await BackgroundPreparationLoop.run(allowed: { !paused }, step: {
      steps += 1
      return .waiting
    }, wait: {
      waits += 1
      paused = true
    })
    XCTAssertEqual(steps, 1)
    XCTAssertEqual(waits, 1)
  }
  func testExpirationCancelsTheNextBatch() async {
    var rounds = 0
    let task = Task { @MainActor in
      await BackgroundPreparationLoop.run(allowed: { true }, step: {
        rounds += 1
        return .waiting
      })
    }
    while rounds == 0 { await Task.yield() }
    task.cancel()
    await task.value
    XCTAssertEqual(rounds, 1)
  }
}
