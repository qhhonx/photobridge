import AVFoundation
import SwiftUI

struct PairingScanner: UIViewControllerRepresentable {
  let scanned: (String) -> Void
  func makeUIViewController(context: Context) -> ScannerController {
    ScannerController(scanned: scanned)
  }
  func updateUIViewController(_ controller: ScannerController, context: Context) {}
}
final class ScannerController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
  private let session = AVCaptureSession()
  private let worker = DispatchQueue(label: "app.photobridge.camera")
  private let scanned: (String) -> Void
  private var finished = false
  private var preview: AVCaptureVideoPreviewLayer?
  init(scanned: @escaping (String) -> Void) {
    self.scanned = scanned
    super.init(nibName: nil, bundle: nil)
  }
  required init?(coder: NSCoder) { fatalError("init(coder:) is unavailable") }
  override func viewDidLoad() {
    super.viewDidLoad()
    view.backgroundColor = .black
    AVCaptureDevice.requestAccess(for: .video) { allowed in
      if allowed {
        self.worker.async { self.configure() }
      } else {
        DispatchQueue.main.async {
          let label = UILabel(frame: self.view.bounds)
          label.text = NSLocalizedString("camera_permission_needed", comment: "")
          label.textColor = .white
          label.textAlignment = .center
          label.numberOfLines = 0
          self.view.addSubview(label)
        }
      }
    }
  }
  private func configure() {
    guard let camera = AVCaptureDevice.default(for: .video),
      let input = try? AVCaptureDeviceInput(device: camera), session.canAddInput(input)
    else { return }
    session.addInput(input)
    let output = AVCaptureMetadataOutput()
    guard session.canAddOutput(output) else { return }
    session.addOutput(output)
    output.setMetadataObjectsDelegate(self, queue: .main)
    output.metadataObjectTypes = [.qr]
    DispatchQueue.main.async {
      let layer = AVCaptureVideoPreviewLayer(session: self.session)
      layer.videoGravity = .resizeAspectFill
      layer.frame = self.view.bounds
      self.view.layer.addSublayer(layer)
      self.preview = layer
    }
    session.startRunning()
  }
  override func viewDidLayoutSubviews() {
    super.viewDidLayoutSubviews()
    preview?.frame = view.bounds
  }
  override func viewDidDisappear(_ animated: Bool) {
    super.viewDidDisappear(animated)
    worker.async { self.session.stopRunning() }
  }
  func metadataOutput(
    _ output: AVCaptureMetadataOutput, didOutput metadataObjects: [AVMetadataObject],
    from connection: AVCaptureConnection
  ) {
    guard !finished,
      let payload = (metadataObjects.first as? AVMetadataMachineReadableCodeObject)?.stringValue
    else { return }
    finished = true
    scanned(payload)
  }
}
