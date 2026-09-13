// A headless user driver for temporary, synthetic update targets only.
import AppKit
import Sparkle

@MainActor final class Driver: NSObject, SPUUserDriver {
    let expectRejection: Bool
    var finished = false
    init(expectRejection: Bool) { self.expectRejection = expectRejection }
    func show(_ message: String) { print(message); fflush(stdout) }
    func show(_ request: SPUUpdatePermissionRequest, reply: @escaping (SUUpdatePermissionResponse) -> Void) {
        reply(SUUpdatePermissionResponse(automaticUpdateChecks: false, sendSystemProfile: false))
    }
    func showUserInitiatedUpdateCheck(cancellation: @escaping () -> Void) { show("Checking synthetic feed") }
    func showUpdateFound(with appcastItem: SUAppcastItem, state: SPUUserUpdateState, reply: @escaping (SPUUserUpdateChoice) -> Void) {
        if expectRejection { fatalError("Tampered signed feed was accepted") }
        show("Verified feed; update found"); reply(.install)
    }
    func showUpdateReleaseNotes(with downloadData: SPUDownloadData) {}
    func showUpdateReleaseNotesFailedToDownloadWithError(_ error: Error) {}
    func showUpdateNotFoundWithError(_ error: Error, acknowledgement: @escaping () -> Void) {
        fatalError("Expected an update: \(error)")
    }
    func showUpdaterError(_ error: Error, acknowledgement: @escaping () -> Void) {
        show("Updater error: \(error)")
        acknowledgement()
        exit(expectRejection ? 0 : 1)
    }
    func showDownloadInitiated(cancellation: @escaping () -> Void) { show("Downloading synthetic archive") }
    func showDownloadDidReceiveExpectedContentLength(_ expectedContentLength: UInt64) {}
    func showDownloadDidReceiveData(ofLength length: UInt64) {}
    func showDownloadDidStartExtractingUpdate() { show("Archive verified; extracting") }
    func showExtractionReceivedProgress(_ progress: Double) {}
    func showReady(toInstallAndRelaunch reply: @escaping (SPUUserUpdateChoice) -> Void) { reply(.install) }
    func showInstallingUpdate(withApplicationTerminated applicationTerminated: Bool, retryTerminatingApplication: @escaping () -> Void) { show("Installing temporary target") }
    func showUpdateInstalledAndRelaunched(_ relaunched: Bool, acknowledgement: @escaping () -> Void) {
        finished = true; show("PASS: Sparkle downloaded, verified and installed the synthetic update")
        acknowledgement(); exit(0)
    }
    func dismissUpdateInstallation() {}
}

@main struct SparkleIntegration {
    @MainActor static func main() throws {
        let application = NSApplication.shared
        application.setActivationPolicy(.prohibited)
        let bundle = Bundle(path: CommandLine.arguments[1])!
        precondition(bundle.bundleIdentifier?.hasPrefix("org.photobridge.update-fixture.") == true)
        let driver = Driver(expectRejection: CommandLine.arguments.contains("--expect-rejection"))
        let updater = SPUUpdater(hostBundle: bundle, applicationBundle: Bundle.main, userDriver: driver, delegate: nil)
        try updater.start()
        updater.checkForUpdates()
        DispatchQueue.main.asyncAfter(deadline: .now() + 60) { fatalError("Synthetic update timed out") }
        withExtendedLifetime((driver, updater)) { application.run() }
    }
}
