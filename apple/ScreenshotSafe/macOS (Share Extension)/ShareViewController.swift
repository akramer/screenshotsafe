import Cocoa

final class ShareViewController: NSViewController {
    private let settingsStore = ScreenshotSafeSettingsStore()
    private let uploadClient = ScreenshotSafeUploadClient()

    private let imageView = NSImageView()
    private let titleField = NSTextField()
    private let statusLabel = NSTextField(labelWithString: "Loading shared screenshot...")
    private let uploadButton = NSButton(title: "Upload", target: nil, action: nil)
    private let cancelButton = NSButton(title: "Cancel", target: nil, action: nil)

    private var imageData: Data?
    private var filename = "screenshot.png"

    override func loadView() {
        view = NSView(frame: NSRect(x: 0, y: 0, width: 520, height: 420))
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        buildView()
        loadSharedImage()
    }

    private func buildView() {
        let title = NSTextField(labelWithString: "Upload to ScreenshotSafe")
        title.font = .systemFont(ofSize: 20, weight: .semibold)

        imageView.imageScaling = .scaleProportionallyUpOrDown
        imageView.wantsLayer = true
        imageView.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        imageView.layer?.borderColor = NSColor.separatorColor.cgColor
        imageView.layer?.borderWidth = 1
        imageView.translatesAutoresizingMaskIntoConstraints = false
        imageView.heightAnchor.constraint(equalToConstant: 220).isActive = true

        titleField.placeholderString = "Screenshot title"
        titleField.stringValue = "Screenshot"

        statusLabel.textColor = .secondaryLabelColor
        statusLabel.maximumNumberOfLines = 2

        uploadButton.target = self
        uploadButton.action = #selector(upload)
        uploadButton.bezelStyle = .rounded
        uploadButton.isEnabled = false

        cancelButton.target = self
        cancelButton.action = #selector(cancel)
        cancelButton.bezelStyle = .rounded

        let buttons = NSStackView(views: [cancelButton, uploadButton])
        buttons.orientation = .horizontal
        buttons.alignment = .centerY
        buttons.spacing = 8

        let stack = NSStackView(views: [title, imageView, titleField, statusLabel, buttons])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 14
        stack.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(stack)

        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 20),
            stack.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -20),
            stack.topAnchor.constraint(equalTo: view.topAnchor, constant: 20),
            titleField.widthAnchor.constraint(equalTo: stack.widthAnchor),
            imageView.widthAnchor.constraint(equalTo: stack.widthAnchor),
        ])
    }

    private func loadSharedImage() {
        guard let item = extensionContext?.inputItems.first as? NSExtensionItem,
              let providers = item.attachments else {
            showError("No shared image was provided.")
            return
        }

        if let subject = item.attributedTitle?.string, !subject.isEmpty {
            titleField.stringValue = subject
        }

        guard let provider = providers.first(where: { $0.hasItemConformingToTypeIdentifier("public.image") || $0.hasItemConformingToTypeIdentifier("public.file-url") }) else {
            showError("ScreenshotSafe can upload shared image files.")
            return
        }

        let typeIdentifier = provider.hasItemConformingToTypeIdentifier("public.image") ? "public.image" : "public.file-url"
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
                        self?.imageView.image = NSImage(data: payload.data)
                        self?.statusLabel.stringValue = "Ready to upload."
                        self?.statusLabel.textColor = .secondaryLabelColor
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
            do {
                let data = try Data(contentsOf: url)
                completion(.success((normalizedPNGData(from: data) ?? data, "screenshot.png")))
            } catch {
                completion(.failure(error))
            }
            return
        }

        if let image = item as? NSImage, let data = image.pngData() {
            completion(.success((data, "screenshot.png")))
            return
        }

        completion(.failure(NSError(domain: "ScreenshotSafeShareExtension", code: 1, userInfo: [
            NSLocalizedDescriptionKey: "The shared item could not be read as an image."
        ])))
    }

    private func normalizedPNGData(from data: Data) -> Data? {
        NSImage(data: data)?.pngData()
    }

    @objc private func upload() {
        guard let imageData = imageData else {
            showError("No image is ready to upload.")
            return
        }

        let settings = settingsStore.load()
        uploadButton.isEnabled = false
        statusLabel.stringValue = "Uploading..."
        statusLabel.textColor = .secondaryLabelColor

        uploadClient.upload(
            imageData: imageData,
            filename: filename,
            title: titleField.stringValue,
            sourceURL: nil,
            settings: settings
        ) { [weak self] result in
            DispatchQueue.main.async {
                switch result {
                case .success(let upload):
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(upload.shareURL.absoluteString, forType: .string)
                    self?.statusLabel.stringValue = "Uploaded and copied share link."
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

    private func showError(_ message: String) {
        statusLabel.stringValue = message
        statusLabel.textColor = .systemRed
        uploadButton.isEnabled = false
    }

    private func completeAfterDelay() {
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.8) { [weak self] in
            self?.extensionContext?.completeRequest(returningItems: [], completionHandler: nil)
        }
    }
}

private extension NSImage {
    func pngData() -> Data? {
        guard
            let tiff = tiffRepresentation,
            let bitmap = NSBitmapImageRep(data: tiff)
        else {
            return nil
        }
        return bitmap.representation(using: .png, properties: [:])
    }
}
