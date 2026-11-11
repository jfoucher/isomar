import UIKit
import UniformTypeIdentifiers

/// Files on iOS: the system's document picker. What the user picks is copied into the app's container
/// before the engine reads it (a folder of charts to Application Support/charts, where it stays); a file to
/// write is written to a temporary path, then exported where the user says.
final class DocumentPicker: NSObject, UIDocumentPickerDelegate {
    static let shared = DocumentPicker()
    private var then: ((String) -> Void)?
    private var directory = false
    private var exported: URL?

    func choose(open: Bool, directory: Bool, name: String, then: @escaping (String) -> Void) {
        guard let presenter = Platform.presenter else { return }
        if !open && !directory {
            // Written first, then handed to the picker to put where the user wants it.
            let tmp = FileManager.default.temporaryDirectory.appendingPathComponent(name.isEmpty ? "isomar.gpx" : name)
            try? FileManager.default.removeItem(at: tmp)
            then(tmp.path)
            guard FileManager.default.fileExists(atPath: tmp.path) else { return }
            exported = tmp
            self.then = nil
            let picker = UIDocumentPickerViewController(forExporting: [tmp], asCopy: true)
            picker.delegate = self
            presenter.present(picker, animated: true)
            return
        }
        self.then = then
        self.directory = directory
        let picker = directory
            ? UIDocumentPickerViewController(forOpeningContentTypes: [.folder])
            : UIDocumentPickerViewController(forOpeningContentTypes: [.item], asCopy: true)
        picker.delegate = self
        picker.allowsMultipleSelection = false
        presenter.present(picker, animated: true)
    }

    func documentPicker(_ controller: UIDocumentPickerViewController, didPickDocumentsAt urls: [URL]) {
        if let e = exported { try? FileManager.default.removeItem(at: e); exported = nil; return }
        guard let url = urls.first, let then else { return }
        self.then = nil
        if !directory {
            // A copy in the app's inbox already (asCopy).
            then(url.path)
            return
        }
        // A folder elsewhere (Files, iCloud, a USB stick) is readable only while its access lasts: copied in.
        let ok = url.startAccessingSecurityScopedResource()
        let fm = FileManager.default
        let root = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0].appendingPathComponent("Isomar/charts")
        let dest = root.appendingPathComponent(url.lastPathComponent)
        IsomarModel.shared.show(toast: "Copying \(url.lastPathComponent)…")
        DispatchQueue.global(qos: .userInitiated).async {
            var failure: String?
            do {
                try fm.createDirectory(at: root, withIntermediateDirectories: true)
                if fm.fileExists(atPath: dest.path) { try fm.removeItem(at: dest) }
                try fm.copyItem(at: url, to: dest)
            } catch {
                failure = error.localizedDescription
            }
            if ok { url.stopAccessingSecurityScopedResource() }
            DispatchQueue.main.async {
                if let failure { IsomarModel.alert("Could not copy the charts", failure) } else { then(dest.path) }
            }
        }
    }

    func documentPickerWasCancelled(_ controller: UIDocumentPickerViewController) {
        if let e = exported { try? FileManager.default.removeItem(at: e); exported = nil }
        then = nil
    }
}
