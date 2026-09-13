import Foundation
import Security
import OSLog
#if os(iOS)
  import UIKit
#endif

struct NativeAttempt: Codable {
  let jobID: Int64
  let generation: Int64
  var receiverID: String? = nil
  enum CodingKeys: String, CodingKey {
    case jobID = "job_id"
    case generation
    case receiverID = "receiver_id"
  }
  var object: [String: Any] { ["job_id": jobID, "generation": generation] }
}
struct NativeRequest: Decodable {
  let attempt: NativeAttempt
  let method: String
  let path: String
  let contentType: String
  let bodyFile: String
  enum CodingKeys: String, CodingKey {
    case attempt, method, path
    case contentType = "content_type"
    case bodyFile = "body_file"
  }
}

/// A platform executor, not a second queue. Rust provides file-backed requests
/// and validates receiver acknowledgements, retries and stale task generations.
@MainActor
final class BackgroundTransfer: NSObject, @preconcurrency URLSessionDataDelegate,
  @preconcurrency URLSessionTaskDelegate
{
  static let shared = BackgroundTransfer()
  static let identifier = "app.photobridge.uploads.v1"
  var eventsCompletion: (() -> Void)?
  var waitingForNetwork = false
  private var responseBodies: [Int: Data] = [:]
  private var oversized: Set<Int> = []
  private var rejectedTrust: Set<Int> = []
  private var relocatedTasks: Set<Int> = []
  private var finishingTasks: Set<Int> = []
  private var reconciled = false
  private var serialWork: Task<Void, Never>?
  private var lastDecision: String?
  private var progressTracker = TaskProgressTracker()
  private var progressPublication: Task<Void, Never>?
  private func publishProgress() {
    guard progressPublication == nil else { return }
    progressPublication = Task {
      try? await Task.sleep(nanoseconds: 150_000_000)
      let jobs = progressTracker.jobs
      BackupModel.shared.transferProgress = jobs
      waitingForNetwork = !jobs.isEmpty && jobs.values.allSatisfy(\.waitingForNetwork)
      BackupModel.shared.waitingForNetwork = waitingForNetwork
      progressPublication = nil
    }
  }
  private func observeProgress(_ task: URLSessionTask) {
    guard task.state != .completed, task.countOfBytesSent > 0,
      let description = task.taskDescription,
      let attempt = try? JSONDecoder().decode(NativeAttempt.self, from: Data(description.utf8)) else { return }
    progressTracker.update(taskID: task.taskIdentifier, attempt: attempt,
      sent: task.countOfBytesSent, expected: task.countOfBytesExpectedToSend,
      phase: phase(task.originalRequest) ?? "manifest",
      baseline: BackupModel.shared.jobs.first { $0.id == attempt.jobID }?.confirmedBytes ?? 0)
    publishProgress()
  }
  private lazy var session: URLSession = {
    #if os(iOS)
      let configuration = URLSessionConfiguration.background(withIdentifier: Self.identifier)
      configuration.sessionSendsLaunchEvents = true
      configuration.isDiscretionary = false
    #else
      let configuration = URLSessionConfiguration.default
    #endif
    configuration.waitsForConnectivity = true
    configuration.allowsCellularAccess = false
    configuration.httpMaximumConnectionsPerHost = 4
    configuration.timeoutIntervalForRequest = 60
    configuration.timeoutIntervalForResource = 3600
    // URLSession delegate callbacks are explicitly delivered on the main queue.
    return URLSession(configuration: configuration, delegate: self, delegateQueue: .main)
  }()
  func connect() { _ = session }

  // Metadata-only diagnostics: no paths, photo names, endpoints or credentials.
  func record(_ code: String, jobID: Int64? = nil, bytes: Int64? = nil,
    context: [String: Any] = [:]) async {
    Logger(subsystem: "app.photobridge", category: "background").info("\(code, privacy: .public) job=\(jobID ?? -1) bytes=\(bytes ?? -1)")
    guard BackupModel.shared.ready else { return }
    var command: [String: Any] = ["op": "record_event", "receiver": false, "code": code]
    if let jobID { command["job_id"] = jobID }
    if let bytes, bytes >= 0 { command["bytes"] = bytes }
    if !context.isEmpty { command["context"] = context }
    _ = try? await Bridge.call(command)
  }
  private var execution: String {
    #if os(iOS)
      switch UIApplication.shared.applicationState {
      case .active: return "foreground"
      case .background: return "background"
      default: return "inactive"
      }
    #else
      return "desktop"
    #endif
  }
  private func phase(_ request: URLRequest?) -> String? {
    guard let request else { return nil }
    if request.url?.path == "/v1/bundles" { return "bundle" }
    return request.httpMethod == "PUT" ? "upload" : request.url?.path.hasSuffix("/commit") == true ? "commit" : "manifest"
  }
  /// Query whole-queue counts, not the UI's first page. This is observational and
  /// cannot change retry timing or imply that the operating system ran a task.
  func recordSnapshot(_ code: String, reason: String? = nil,
    tasks suppliedTasks: [URLSessionTask]? = nil, execution observedExecution: String? = nil) async {
    let model = BackupModel.shared
    guard model.ready else { return }
    if model.deviceSnapshot == nil { await model.refreshDeviceStatus() }
    var context: [String: Any] = ["execution": observedExecution ?? execution,
      "paused": model.paused, "pending_imports": model.pendingImports,
      "discovery_pending": model.discoveryPending]
    if let reason { context["reason"] = reason }
    if let receiver = model.pairing?.receiverID,
      let data = try? await Bridge.call(["op": "sender_summary", "receiver_id": receiver]),
      let counts = try? JSONSerialization.jsonObject(with: data) as? [String: Any] {
      for key in ["queued", "running", "waiting", "failed", "next_retry_at"] {
        context[key] = counts[key]
      }
    }
    let tasks: [URLSessionTask]
    if let suppliedTasks { tasks = suppliedTasks } else { tasks = await session.allTasks }
    context["active_requests"] = tasks.count
    var jobID: Int64?
    if let task = tasks.first {
      jobID = task.taskDescription.flatMap { try? JSONDecoder().decode(NativeAttempt.self, from: Data($0.utf8)) }?.jobID
      context["phase"] = phase(task.originalRequest)
      context["bytes_sent"] = max(0, task.countOfBytesSent)
      if task.countOfBytesExpectedToSend >= 0 { context["bytes_expected"] = task.countOfBytesExpectedToSend }
    }
    await record(code, jobID: jobID, context: context)
  }
  private func recordDecision(_ reason: String, tasks: [URLSessionTask]) async {
    let model = BackupModel.shared
    let key = "\(reason)|\(model.queueRevision)|\(model.pendingImports)|\(model.discoveryPending)|\(tasks.count)"
    guard key != lastDecision else { return }
    lastDecision = key
    await recordSnapshot("dispatch_waiting", reason: reason, tasks: tasks)
  }
  #if os(iOS)
    private var handoff: UIBackgroundTaskIdentifier = .invalid
    func enteredBackground() {
      guard handoff == .invalid else { return }
      handoff = UIApplication.shared.beginBackgroundTask(withName: "PhotoBridge request handoff") { [weak self] in
        guard let self else { return }
        self.endHandoff()
        Task { await self.recordSnapshot("background_time_expired", execution: "background") }
      }
      Task {
        await recordSnapshot("app_background", execution: "background")
        await kick()
      }
    }
    func enteredForeground() {
      endHandoff()
      Task { await recordSnapshot("app_foreground", execution: "foreground") }
    }
    private func endHandoff() {
      guard handoff != .invalid else { return }
      UIApplication.shared.endBackgroundTask(handoff)
      handoff = .invalid
    }
  #endif

  // Serialize enumeration, binding and callbacks across suspension points.
  @discardableResult private func schedule(_ operation: @escaping @MainActor () async -> Void)
    -> Task<Void, Never>
  {
    let previous = serialWork
    let next = Task {
      await previous?.value
      await operation()
    }
    serialWork = next
    return next
  }
  func kick(reconcile: Bool = false) async {
    await schedule {
      if reconcile { self.reconciled = false }
      await self.pump()
    }.value
  }
  func cancelAll() async {
    await schedule {
      for task in await self.session.allTasks { task.cancel() }
      self.waitingForNetwork = false
    }.value
  }
  func replaceRoute(_ updated: Pairing, expected: Pairing) async {
    await schedule {
      let model = BackupModel.shared
      guard let current = model.pairing, current.receiverID == expected.receiverID,
        current.endpoint == expected.endpoint, current.token == expected.token,
        current.certificate == expected.certificate else { return }
      do {
        // Mark cancellations before saving the new route. Late trust callbacks
        // from the old address must requeue, never permanently fail the asset.
        for task in await self.session.allTasks {
          self.relocatedTasks.insert(task.taskIdentifier)
          task.cancel()
        }
        try await Keychain.saveAsync(JSONEncoder().encode(updated))
        model.pairing = updated
        _ = try await Bridge.call(["op": "recover_connection", "receiver_id": updated.receiverID])
        self.waitingForNetwork = false
        await self.record("receiver_address_updated")
        // Keep explicit user pause; receipt/source identities do not change.
        await self.pump()
      } catch { model.message = error.localizedDescription }
    }.value
  }
  private func pump() async {
    let model = BackupModel.shared
    guard model.ready else { return }
    do {
      let tasks = await session.allTasks
      for task in tasks { observeProgress(task) }
      let live = tasks.map { String($0.taskIdentifier) } + finishingTasks.map(String.init)
      var orphans: [String] = []
      if !reconciled {
        let data = try await Bridge.call(["op": "reconcile_native", "live": live])
        orphans = try JSONDecoder().decode([String].self, from: data)
        reconciled = true
      }
      for task in tasks {
        if orphans.contains(String(task.taskIdentifier)) || model.paused {
          task.cancel()
        } else if task.state == .suspended {
          task.resume()
        }
      }
      guard !model.paused else {
        await recordDecision("paused", tasks: tasks)
        return
      }
      guard let pairing = model.pairing else {
        await recordDecision("unpaired", tasks: tasks)
        return
      }
      // Rust limits prepared jobs to a small window; the OS can execute these
      // file uploads consecutively without waking us between assets.
      while !model.paused {
        let plan = try await Bridge.call(["op": "prepare_native", "receiver_id": pairing.receiverID])
        guard let request = try JSONDecoder().decode(NativeRequest?.self, from: plan) else {
          await recordDecision("no_eligible_job", tasks: await session.allTasks)
          #if os(iOS)
            endHandoff()
          #endif
          return
        }
        do {
          guard let url = URL(string: pairing.endpoint + request.path), url.scheme == "https",
            url.host == URL(string: pairing.endpoint)?.host
          else { throw Bridge.Failure(code: "invalid_input") }
          let file = URL(fileURLWithPath: request.bodyFile)
          // Background transfers may read immutable request files while locked,
          // after the first device unlock. Pairing uses the same Keychain policy.
          #if os(iOS)
            try FileManager.default.setAttributes(
              [.protectionKey: FileProtectionType.completeUntilFirstUserAuthentication],
              ofItemAtPath: file.path)
          #endif
          var native = URLRequest(url: url)
          native.httpMethod = request.method
          native.setValue("Bearer " + pairing.token, forHTTPHeaderField: "Authorization")
          if let device = model.deviceSnapshot?.device {
            native.setValue(device.id, forHTTPHeaderField: "X-PhotoBridge-Sender")
            native.setValue(senderDeviceType, forHTTPHeaderField: "X-PhotoBridge-Device-Type")
          }
          native.setValue(request.contentType, forHTTPHeaderField: "Content-Type")
          let task = session.uploadTask(with: native, fromFile: file)
          var descriptor = request.attempt
          descriptor.receiverID = pairing.receiverID
          task.taskDescription = String(
            decoding: try JSONEncoder().encode(descriptor), as: UTF8.self)
          do {
            _ = try await Bridge.call([
              "op": "bind_native", "attempt": request.attempt.object,
              "task_id": String(task.taskIdentifier),
            ])
            let stage = request.path == "/v1/bundles" ? "bundle_submitted" : request.method == "PUT" ? "upload_submitted" : request.path.hasSuffix("/commit") ? "commit_submitted" : "manifest_submitted"
            await record(stage, jobID: request.attempt.jobID,
              bytes: (try? FileManager.default.attributesOfItem(atPath: file.path)[.size] as? NSNumber)?.int64Value,
              context: ["execution": execution, "phase": phase(native) ?? "manifest"])
            lastDecision = nil
            task.resume()
          } catch {
            task.cancel()
            throw error
          }
        } catch {
          _ = try? await Bridge.call(["op": "abandon_native", "attempt": request.attempt.object])
          throw error
        }
      }
    } catch {
      model.message = error.localizedDescription
      await recordDecision("preparation_failed", tasks: await session.allTasks)
    }
  }

  static func isOldRoute(receiverID: String?, originalURL: URL?, current: Pairing?) -> Bool {
    guard let current, receiverID == current.receiverID, let originalURL,
      let now = URL(string: current.endpoint) else { return false }
    return originalURL.scheme != now.scheme || originalURL.host != now.host
      || (originalURL.port ?? 443) != (now.port ?? 443)
  }

  func urlSession(_ session: URLSession, dataTask: URLSessionDataTask, didReceive data: Data) {
    let id = dataTask.taskIdentifier
    guard !oversized.contains(id) else { return }
    if (responseBodies[id]?.count ?? 0) + data.count > 256 * 1024 {
      oversized.insert(id)
      dataTask.cancel()
    } else {
      responseBodies[id, default: Data()].append(data)
    }
  }
  func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
    let id = task.taskIdentifier
    let body = responseBodies.removeValue(forKey: id) ?? Data()
    let invalid = oversized.remove(id) != nil
    let moved = relocatedTasks.remove(id) != nil
    let untrusted = rejectedTrust.remove(id) != nil && !moved
    let attempt = task.taskDescription.flatMap {
      try? JSONDecoder().decode(NativeAttempt.self, from: Data($0.utf8))
    }
    let code = (task.response as? HTTPURLResponse)?.statusCode ?? 0
    let nsError = error as NSError?
    let cancelled = moved || (nsError?.code == NSURLErrorCancelled && !invalid && !untrusted)
    let authenticationErrors = [
      NSURLErrorServerCertificateUntrusted, NSURLErrorServerCertificateHasBadDate,
      NSURLErrorServerCertificateHasUnknownRoot, NSURLErrorServerCertificateNotYetValid,
      NSURLErrorSecureConnectionFailed, NSURLErrorUserCancelledAuthentication,
    ]
    finishingTasks.insert(id)
    #if os(iOS)
      let lifetime = ReceiptCallbackLifetime()
    #endif
    schedule {
      defer {
        self.finishingTasks.remove(id)
        #if os(iOS)
          lifetime.end()
        #endif
      }

      await BackupModel.shared.open()
      var context: [String: Any] = ["execution": self.execution, "http_status": code]
      context["phase"] = self.phase(task.originalRequest)
      context["system_error"] = nsError?.code
      if let attempt {
        // Survives process termination: URLSession persists the original URL and
        // descriptor. A late result from the same peer's old route is replayable.
        let routeChanged = Self.isOldRoute(receiverID: attempt.receiverID,
          originalURL: task.originalRequest?.url, current: BackupModel.shared.pairing)
        var command: [String: Any] = [
          "op": "finish_native", "attempt": attempt.object,
          "task_id": String(id), "status_code": code, "body": String(decoding: body, as: UTF8.self),
          "cancelled": cancelled || routeChanged,
        ]
        if moved || routeChanged {
          // Ignore stale old-route results; durable receiver offsets are queried again.
        } else if untrusted {
          command["failure"] = "authentication"
        } else if invalid {
          command["failure"] = "integrity"
        } else if let nsError, !cancelled {
          command["failure"] =
            authenticationErrors.contains(nsError.code) ? "authentication" : "network"
        }
        do { _ = try await Bridge.call(command) } catch let failure as Bridge.Failure
          where failure.code == "conflict"
        { /* A superseded callback is harmless. */  } catch {
          BackupModel.shared.message = error.localizedDescription
        }
      }
      await self.record(error == nil ? "request_completed" : "request_failed", jobID: attempt?.jobID, context: context)
      await self.pump()
      await BackupModel.shared.refresh()
      // Keep the last progress through receipt processing; clear only this task.
      self.progressTracker.remove(taskID: id)
      self.publishProgress()
      BackupModel.shared.scheduleBackgroundWork()
    }
  }
  func urlSession(
    _ session: URLSession, task: URLSessionTask, didSendBodyData bytesSent: Int64,
    totalBytesSent: Int64, totalBytesExpectedToSend: Int64
  ) {
    guard let description = task.taskDescription,
      let attempt = try? JSONDecoder().decode(NativeAttempt.self, from: Data(description.utf8))
    else { return }
    progressTracker.update(taskID: task.taskIdentifier, attempt: attempt,
      sent: totalBytesSent, expected: totalBytesExpectedToSend,
      phase: phase(task.originalRequest) ?? "manifest",
      baseline: BackupModel.shared.jobs.first { $0.id == attempt.jobID }?.confirmedBytes ?? 0)
    publishProgress()
  }
  func urlSession(_ session: URLSession, taskIsWaitingForConnectivity task: URLSessionTask) {
    if let description = task.taskDescription,
      let attempt = try? JSONDecoder().decode(NativeAttempt.self, from: Data(description.utf8)) {
      progressTracker.update(taskID: task.taskIdentifier, attempt: attempt,
        sent: task.countOfBytesSent, expected: task.countOfBytesExpectedToSend,
        phase: phase(task.originalRequest) ?? "manifest", baseline: 0)
      progressTracker.waiting(taskID: task.taskIdentifier)
      publishProgress()
    }
    Task { await self.recordSnapshot("request_waiting_network", tasks: [task]) }
  }
  func urlSessionDidFinishEvents(forBackgroundURLSession session: URLSession) {
    schedule {
      await self.recordSnapshot("background_events_finished")
      let completion = self.eventsCompletion
      self.eventsCompletion = nil
      completion?()
    }
  }
  func urlSession(
    _ session: URLSession, task: URLSessionTask,
    willPerformHTTPRedirection response: HTTPURLResponse,
    newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void
  ) {
    completionHandler(nil)  // Foreground sessions reject redirects; our protocol never emits them.
  }
  func urlSession(
    _ session: URLSession, task: URLSessionTask, didReceive challenge: URLAuthenticationChallenge,
    completionHandler: @escaping (URLSession.AuthChallengeDisposition, URLCredential?) -> Void
  ) {
    let taskID = task.taskIdentifier
    let reject = {
      DispatchQueue.main.async {
        self.rejectedTrust.insert(taskID)
        completionHandler(.cancelAuthenticationChallenge, nil)
      }
    }
    DispatchQueue.global(qos: .utility).async {
      guard challenge.protectionSpace.authenticationMethod == NSURLAuthenticationMethodServerTrust,
        let trust = challenge.protectionSpace.serverTrust,
        let data = try? Keychain.read(),
        let pairing = try? JSONDecoder().decode(Pairing.self, from: data),
        let pinnedDER = Data(base64Encoded: pairing.certificate),
        challenge.protectionSpace.host == URL(string: pairing.endpoint)?.host,
        let certificates = SecTrustCopyCertificateChain(trust) as? [SecCertificate],
        let leaf = certificates.first, SecCertificateCopyData(leaf) as Data == pinnedDER
      else {
        reject()
        return
      }
      SecTrustSetAnchorCertificates(trust, [leaf] as CFArray)
      SecTrustSetAnchorCertificatesOnly(trust, true)
      SecTrustSetPolicies(
        trust, SecPolicyCreateSSL(true, (pairing.certificateName ?? challenge.protectionSpace.host) as CFString))
      guard SecTrustEvaluateWithError(trust, nil) else {
        reject()
        return
      }
      completionHandler(.useCredential, URLCredential(trust: trust))
    }
  }
}

#if os(iOS)
/// URLSession delegate methods return synchronously, while durable receipt work
/// is serialized asynchronously. Hold execution until that work finishes.
@MainActor private final class ReceiptCallbackLifetime {
  private var identifier: UIBackgroundTaskIdentifier = .invalid
  init() {
    guard UIApplication.shared.applicationState != .active else { return }
    identifier = UIApplication.shared.beginBackgroundTask(withName: "PhotoBridge receipt") { [weak self] in
      self?.end()
    }
  }
  func end() {
    guard identifier != .invalid else { return }
    UIApplication.shared.endBackgroundTask(identifier)
    identifier = .invalid
  }
}
#endif
