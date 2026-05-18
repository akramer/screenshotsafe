//
//  ViewController.swift
//  Shared (App)
//
//  Created by Adam Kramer on 5/17/26.
//

import WebKit

#if os(iOS)
import UIKit
typealias PlatformViewController = UIViewController
#elseif os(macOS)
import Cocoa
import SafariServices
typealias PlatformViewController = NSViewController
#endif

let extensionBundleIdentifier = "com.screenshotsafe.safari.Extension"

class ViewController: PlatformViewController, WKNavigationDelegate, WKScriptMessageHandler {

    @IBOutlet var webView: WKWebView!

#if os(macOS)
    private let settingsStore = ScreenshotSafeSettingsStore()
    private let uploadClient = ScreenshotSafeUploadClient()
    private let serverURLField = NSTextField()
    private let apiTokenField = NSSecureTextField()
    private let expiryPopup = NSPopUpButton()
    private let statusLabel = NSTextField(labelWithString: "")
    private let safariStatusLabel = NSTextField(labelWithString: "")
#endif

    override func viewDidLoad() {
        super.viewDidLoad()

#if os(macOS)
        buildMacSettingsView()
        refreshSafariExtensionState()
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(settingsDidChange),
            name: .screenshotSafeSettingsDidChange,
            object: nil
        )
#else
        self.webView.navigationDelegate = self

        self.webView.scrollView.isScrollEnabled = false

        self.webView.configuration.userContentController.add(self, name: "controller")

        self.webView.loadFileURL(Bundle.main.url(forResource: "Main", withExtension: "html")!, allowingReadAccessTo: Bundle.main.resourceURL!)
#endif
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
#if os(iOS)
        webView.evaluateJavaScript("show('ios')")
#elseif os(macOS)
        webView.evaluateJavaScript("show('mac')")

        SFSafariExtensionManager.getStateOfSafariExtension(withIdentifier: extensionBundleIdentifier) { (state, error) in
            guard let state = state, error == nil else {
                // Insert code to inform the user that something went wrong.
                return
            }

            DispatchQueue.main.async {
                if #available(macOS 13, *) {
                    webView.evaluateJavaScript("show('mac', \(state.isEnabled), true)")
                } else {
                    webView.evaluateJavaScript("show('mac', \(state.isEnabled), false)")
                }
            }
        }
#endif
    }

    func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
#if os(macOS)
        if (message.body as! String != "open-preferences") {
            return
        }

        SFSafariApplication.showPreferencesForExtension(withIdentifier: extensionBundleIdentifier) { error in
            guard error == nil else {
                // Insert code to inform the user that something went wrong.
                return
            }

            DispatchQueue.main.async {
                NSApp.terminate(self)
            }
        }
#endif
    }

}

#if os(macOS)
private extension ViewController {
    func buildMacSettingsView() {
        let root = NSView()
        root.translatesAutoresizingMaskIntoConstraints = false
        view = root

        let title = NSTextField(labelWithString: "ScreenshotSafe")
        title.font = .systemFont(ofSize: 28, weight: .semibold)
        title.textColor = .labelColor

        let subtitle = NSTextField(labelWithString: "Configure the native app and Share Extension to upload screenshots to your ScreenshotSafe server.")
        subtitle.font = .systemFont(ofSize: 13)
        subtitle.textColor = .secondaryLabelColor
        subtitle.maximumNumberOfLines = 2
        subtitle.lineBreakMode = .byWordWrapping

        serverURLField.placeholderString = "https://screenshots.example.com"
        apiTokenField.placeholderString = "API token"

        expiryPopup.addItems(withTitles: [
            "Server default",
            "1 hour",
            "24 hours",
            "7 days",
            "30 days",
            "Never expire"
        ])
        expiryPopup.item(at: 0)?.representedObject = ""
        expiryPopup.item(at: 1)?.representedObject = "1h"
        expiryPopup.item(at: 2)?.representedObject = "24h"
        expiryPopup.item(at: 3)?.representedObject = "7d"
        expiryPopup.item(at: 4)?.representedObject = "30d"
        expiryPopup.item(at: 5)?.representedObject = "never"

        let saveButton = NSButton(title: "Save and Verify", target: self, action: #selector(saveAndVerifySettings))
        saveButton.bezelStyle = .rounded

        let safariButton = NSButton(title: "Open Safari Extension Settings", target: self, action: #selector(openSafariExtensionSettings))
        safariButton.bezelStyle = .rounded

        statusLabel.textColor = .secondaryLabelColor
        statusLabel.maximumNumberOfLines = 2
        safariStatusLabel.textColor = .secondaryLabelColor
        safariStatusLabel.maximumNumberOfLines = 2

        let form = NSGridView(views: [
            [fieldLabel("Server URL"), serverURLField],
            [fieldLabel("API Token"), apiTokenField],
            [fieldLabel("Default Expiry"), expiryPopup],
        ])
        form.column(at: 0).xPlacement = .trailing
        form.column(at: 1).width = 420
        form.rowSpacing = 12
        form.columnSpacing = 12

        let buttonRow = NSStackView(views: [saveButton, safariButton])
        buttonRow.orientation = .horizontal
        buttonRow.spacing = 10

        let stack = NSStackView(views: [title, subtitle, form, buttonRow, statusLabel, safariStatusLabel])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 16
        stack.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(stack)

        NSLayoutConstraint.activate([
            root.widthAnchor.constraint(greaterThanOrEqualToConstant: 560),
            root.heightAnchor.constraint(greaterThanOrEqualToConstant: 360),
            stack.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 32),
            stack.trailingAnchor.constraint(lessThanOrEqualTo: root.trailingAnchor, constant: -32),
            stack.topAnchor.constraint(equalTo: root.topAnchor, constant: 30),
        ])

        loadSettingsIntoForm()
    }

    func loadSettingsIntoForm() {
        let settings = settingsStore.load()
        serverURLField.stringValue = settings.serverURL
        apiTokenField.stringValue = settings.apiToken

        for index in 0..<expiryPopup.numberOfItems {
            if expiryPopup.item(at: index)?.representedObject as? String == settings.defaultExpiry {
                expiryPopup.selectItem(at: index)
                break
            }
        }
    }

    @objc func saveAndVerifySettings() {
        let settings = ScreenshotSafeSettings(
            serverURL: serverURLField.stringValue,
            apiToken: apiTokenField.stringValue,
            defaultExpiry: expiryPopup.selectedItem?.representedObject as? String ?? ""
        )
        settingsStore.save(settings)
        statusLabel.stringValue = "Checking connection..."
        statusLabel.textColor = .secondaryLabelColor

        uploadClient.verify(settings: settings) { [weak self] result in
            DispatchQueue.main.async {
                switch result {
                case .success:
                    self?.statusLabel.stringValue = "Connected. The Share Extension can upload screenshots."
                    self?.statusLabel.textColor = .systemGreen
                case .failure(let error):
                    self?.statusLabel.stringValue = error.localizedDescription
                    self?.statusLabel.textColor = .systemRed
                }
            }
        }
    }

    @objc func settingsDidChange() {
        loadSettingsIntoForm()
        statusLabel.stringValue = "Configuration imported from ScreenshotSafe link."
        statusLabel.textColor = .systemGreen
    }

    @objc func openSafariExtensionSettings() {
        SFSafariApplication.showPreferencesForExtension(withIdentifier: extensionBundleIdentifier) { [weak self] error in
            DispatchQueue.main.async {
                if let error = error {
                    self?.safariStatusLabel.stringValue = error.localizedDescription
                    self?.safariStatusLabel.textColor = .systemRed
                } else {
                    self?.safariStatusLabel.stringValue = "Safari extension settings opened."
                    self?.safariStatusLabel.textColor = .secondaryLabelColor
                }
            }
        }
    }

    func refreshSafariExtensionState() {
        SFSafariExtensionManager.getStateOfSafariExtension(withIdentifier: extensionBundleIdentifier) { [weak self] state, error in
            DispatchQueue.main.async {
                if let state = state {
                    self?.safariStatusLabel.stringValue = state.isEnabled
                        ? "Safari extension is enabled."
                        : "Safari extension is installed but disabled."
                } else if let error = error {
                    self?.safariStatusLabel.stringValue = error.localizedDescription
                } else {
                    self?.safariStatusLabel.stringValue = "Safari extension state is unavailable."
                }
            }
        }
    }

    func fieldLabel(_ text: String) -> NSTextField {
        let label = NSTextField(labelWithString: text)
        label.textColor = .secondaryLabelColor
        return label
    }
}
#endif
