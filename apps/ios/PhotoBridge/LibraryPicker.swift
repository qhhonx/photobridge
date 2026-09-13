import PhotosUI
import SwiftUI

struct LibraryPicker: UIViewControllerRepresentable {
  let selected: ([String]) -> Void
  func makeCoordinator() -> Coordinator { Coordinator(selected) }
  func makeUIViewController(context: Context) -> PHPickerViewController {
    var configuration = PHPickerConfiguration(photoLibrary: .shared())
    configuration.selectionLimit = 0
    configuration.filter = .any(of: [.images, .videos])
    configuration.preferredAssetRepresentationMode = .current
    let controller = PHPickerViewController(configuration: configuration)
    controller.delegate = context.coordinator
    return controller
  }
  func updateUIViewController(_ controller: PHPickerViewController, context: Context) {}
  final class Coordinator: NSObject, PHPickerViewControllerDelegate {
    let selected: ([String]) -> Void
    init(_ selected: @escaping ([String]) -> Void) { self.selected = selected }
    func picker(_ picker: PHPickerViewController, didFinishPicking results: [PHPickerResult]) {
      selected(results.compactMap(\.assetIdentifier))
    }
  }
}
