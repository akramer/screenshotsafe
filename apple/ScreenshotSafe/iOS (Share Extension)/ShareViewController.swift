import UIKit
import UniformTypeIdentifiers

final class ShareViewController: UIViewController {
    private let settingsStore = ScreenshotSafeSettingsStore()
    private let uploadClient = ScreenshotSafeUploadClient()

    private let imageView = UIImageView()
    private let titleField = UITextField()
    private let statusLabel = UILabel()
    private let uploadButton = UIButton(type: .system)
    private let cancelButton = UIButton(type: .system)

    private var imageData: Data?
    private var filename = "screenshot.png"

    override func viewDidLoad() {
        super.viewDidLoad()
        buildView()
        loadSharedImage()
    }

    private func buildView() {
        view.backgroundColor = .systemBackground

        let titleLabel = UILabel()
        titleLabel.text = "Upload to ScreenshotSafe"
        titleLabel.font = .preferredFont(forTextStyle: .title2)
        titleLabel.adjustsFontForContentSizeCategory = true

        imageView.contentMode = .scaleAspectFit
        imageView.backgroundColor = .secondarySystemBackground
        imageView.layer.borderColor = UIColor.separator.cgColor
        imageView.layer.borderWidth = 1
        imageView.layer.cornerRadius = 8
        imageView.clipsToBounds = true
        imageView.translatesAutoresizingMaskIntoConstraints = false
        imageView.heightAnchor.constraint(equalToConstant: 240).isActive = true

        titleField.borderStyle = .roundedRect
        titleField.placeholder = "Screenshot title"
        titleField.text = "Screenshot"
        titleField.returnKeyType = .done
        titleField.addTarget(self, action: #selector(dismissKeyboard), for: .editingDidEndOnExit)

        statusLabel.text = "Loading shared screenshot..."
        statusLabel.textColor = .secondaryLabel
        statusLabel.font = .preferredFont(forTextStyle: .subheadline)
        statusLabel.adjustsFontForContentSizeCategory = true
        statusLabel.numberOfLines = 0

        uploadButton.setTitle("Upload", for: .normal)
        uploadButton.titleLabel?.font = .preferredFont(forTextStyle: .headline)
        uploadButton.isEnabled = false
        uploadButton.addTarget(self, action: #selector(upload), for: .touchUpInside)

        cancelButton.setTitle("Cancel", for: .normal)
        cancelButton.addTarget(self, action: #selector(cancel), for: .touchUpInside)

        let buttonStack = UIStackView(arrangedSubviews: [cancelButton, uploadButton])
        buttonStack.axis = .horizontal
        buttonStack.alignment = .center
        buttonStack.distribution = .equalSpacing

        let stack = UIStackView(arrangedSubviews: [titleLabel, imageView, titleField, statusLabel, buttonStack])
        stack.axis = .vertical
        stack.spacing = 16
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)

        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 20),
            stack.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -20),
            stack.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 20),
            stack.bottomAnchor.constraint(lessThanOrEqualTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -20)
        ])
    }

    private func loadSharedImage() {
        guard let item = extensionContext?.inputItems.first as? NSExtensionItem,
              let providers = item.attachments else {
            showError("No shared image was provided.")
            return
        }

        if let subject = item.attributedTitle?.string, !subject.isEmpty {
            titleField.text = subject
        }

        guard let provider = providers.first(where: { provider in
            provider.hasItemConformingToTypeIdentifier(UTType.image.identifier) ||
                provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier)
        }) else {
            showError("ScreenshotSafe can upload shared image files.")
            return
        }

        let typeIdentifier = provider.hasItemConformingToTypeIdentifier(UTType.image.identifier)
            ? UTType.image.identifier
            : UTType.fileURL.identifier

        provider.loadItem(forTypeIdentifier: typeIdentifier, options: nil) { [weak self] item, error in
            if let error = error {
                DispatchQueue.main.async { self?.showError(error.localizedDescription) }
                return
            }

            self?.resolveImageData(from: item) { result in
                DispatchQueue.main.async {
                    switch result {
                    case .success(let payload):
                        self?.imageData = payload.data
                        self?.filename = payload.filename
                        self?.imageView.image = UIImage(data: payload.data)
                        self?.statusLabel.text = "Ready to upload."
                        self?.statusLabel.textColor = .secondaryLabel
                        self?.uploadButton.isEnabled = true
                    case .failure(let error):
                        self?.showError(error.localizedDescription)
                    }
                }
            }
        }
    }

    private func resolveImageData(from item: NSSecureCoding?, completion: @escaping (Result<(data: Data, filename: String), Error>) -> Void) {
        if let data = item as? Data {
            completion(.success((normalizedPNGData(from: data) ?? data, "screenshot.png")))
            return
        }

        if let url = item as? URL {
            let didAccess = url.startAccessingSecurityScopedResource()
            defer {
                if didAccess {
                    url.stopAccessingSecurityScopedResource()
                }
            }

            do {
                let data = try Data(contentsOf: url)
                completion(.success((normalizedPNGData(from: data) ?? data, "screenshot.png")))
            } catch {
                completion(.failure(error))
            }
            return
        }

        if let image = item as? UIImage, let data = image.pngData() {
            completion(.success((data, "screenshot.png")))
            return
        }

        completion(.failure(NSError(domain: "ScreenshotSafeShareExtension", code: 1, userInfo: [
            NSLocalizedDescriptionKey: "The shared item could not be read as an image."
        ])))
    }

    private func normalizedPNGData(from data: Data) -> Data? {
        UIImage(data: data)?.pngData()
    }

    @objc private func upload() {
        guard let imageData = imageData else {
            showError("No image is ready to upload.")
            return
        }

        let settings = settingsStore.load()
        uploadButton.isEnabled = false
        statusLabel.text = "Uploading..."
        statusLabel.textColor = .secondaryLabel

        uploadClient.upload(
            imageData: imageData,
            filename: filename,
            title: titleField.text ?? "Screenshot",
            sourceURL: nil,
            settings: settings
        ) { [weak self] result in
            DispatchQueue.main.async {
                switch result {
                case .success(let upload):
                    UIPasteboard.general.string = upload.shareURL.absoluteString
                    self?.statusLabel.text = "Uploaded and copied share link."
                    self?.statusLabel.textColor = .systemGreen
                    self?.completeAfterDelay()
                case .failure(let error):
                    self?.showError(error.localizedDescription)
                    self?.uploadButton.isEnabled = true
                }
            }
        }
    }

    @objc private func cancel() {
        extensionContext?.cancelRequest(withError: NSError(domain: NSCocoaErrorDomain, code: NSUserCancelledError, userInfo: nil))
    }

    @objc private func dismissKeyboard() {
        titleField.resignFirstResponder()
    }

    private func showError(_ message: String) {
        statusLabel.text = message
        statusLabel.textColor = .systemRed
        uploadButton.isEnabled = false
    }

    private func completeAfterDelay() {
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.8) { [weak self] in
            self?.extensionContext?.completeRequest(returningItems: [], completionHandler: nil)
        }
    }
}
