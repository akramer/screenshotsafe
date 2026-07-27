//
//  ViewController.swift
//  Shared (App)
//
//  Created by Adam Kramer on 5/17/26.
//

import WebKit
import AuthenticationServices
import CryptoKit
import Security

#if os(iOS)
import AVFoundation
import UIKit
typealias PlatformViewController = UIViewController
#elseif os(macOS)
import Cocoa
import SafariServices
typealias PlatformViewController = NSViewController
#endif

let extensionBundleIdentifier = "com.screenshotsafe.SafariExtension"

private nonisolated struct ScreenshotSafeTokenExchangeResponse: Decodable {
    let token: String
    let displayName: String
    let username: String

    enum CodingKeys: String, CodingKey {
        case token
        case displayName = "display_name"
        case username
    }
}

private enum ScreenshotSafeAuthorizationError: LocalizedError {
    case incompatibleServer
    case invalidCallback
    case invalidState
    case cancelled
    case tokenExchange(String)
    case couldNotStart

    var errorDescription: String? {
        switch self {
        case .incompatibleServer:
            return "This ScreenshotSafe server needs an update before the app can be configured."
        case .invalidCallback:
            return "ScreenshotSafe received an invalid login callback."
        case .invalidState:
            return "ScreenshotSafe could not verify the login callback."
        case .cancelled:
            return "Login was cancelled."
        case .tokenExchange(let message):
            return message
        case .couldNotStart:
            return "ScreenshotSafe could not open the login session."
        }
    }
}

private final class ScreenshotSafeRedirectBlocker: NSObject, URLSessionTaskDelegate {
    func urlSession(
        _ session: URLSession,
        task: URLSessionTask,
        willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest,
        completionHandler: @escaping (URLRequest?) -> Void
    ) {
        completionHandler(nil)
    }
}

private final class ScreenshotSafeAuthorizationCoordinator:
    NSObject,
    ASWebAuthenticationPresentationContextProviding
{
    private let settingsStore: ScreenshotSafeSettingsStore
    private let uploadClient: ScreenshotSafeUploadClient
    private let anchorProvider: () -> ASPresentationAnchor
    private let redirectBlocker = ScreenshotSafeRedirectBlocker()
    private var authenticationSession: ASWebAuthenticationSession?
    private var preflightSession: URLSession?
    private var activeAttemptID: UUID?

    init(
        settingsStore: ScreenshotSafeSettingsStore,
        uploadClient: ScreenshotSafeUploadClient,
        anchorProvider: @escaping () -> ASPresentationAnchor
    ) {
        self.settingsStore = settingsStore
        self.uploadClient = uploadClient
        self.anchorProvider = anchorProvider
    }

    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        anchorProvider()
    }

    func cancel() {
        activeAttemptID = nil
        authenticationSession?.cancel()
        authenticationSession = nil
        preflightSession?.invalidateAndCancel()
        preflightSession = nil
    }

    func logIn(
        serverInput: String,
        completion: @escaping (Result<ScreenshotSafeConnection, Error>) -> Void
    ) {
        cancel()

        let origin: String
        do {
            origin = try ScreenshotSafeServerURLNormalizer.normalize(serverInput)
        } catch {
            completionOnMain(.failure(error), completion)
            return
        }
        let attemptID = UUID()
        activeAttemptID = attemptID

        guard
            let state = randomBase64URL(byteCount: 32),
            let verifier = randomBase64URL(byteCount: 64)
        else {
            finish(
                attemptID: attemptID,
                result: .failure(ScreenshotSafeAuthorizationError.couldNotStart),
                completion: completion
            )
            return
        }
        let challenge = Data(SHA256.hash(data: Data(verifier.utf8))).base64URLEncodedString()
#if os(iOS)
        let redirectURI = "com.screenshotsafe:/authorize/ios"
#else
        let redirectURI = "com.screenshotsafe:/authorize/macos"
#endif

        guard
            let authorizeURL = endpoint(origin: origin, path: "/extension/authorize", queryItems: [
                URLQueryItem(name: "redirect_uri", value: redirectURI),
                URLQueryItem(name: "state", value: state),
                URLQueryItem(name: "code_challenge", value: challenge),
                URLQueryItem(name: "code_challenge_method", value: "S256"),
            ])
        else {
            finish(
                attemptID: attemptID,
                result: .failure(ScreenshotSafeConfigurationError.invalidServerURL),
                completion: completion
            )
            return
        }

        preflight(authorizeURL: authorizeURL, attemptID: attemptID) { [weak self] result in
            guard let self = self, self.activeAttemptID == attemptID else { return }
            switch result {
            case .failure(let error):
                self.finish(attemptID: attemptID, result: .failure(error), completion: completion)
            case .success:
                self.beginAuthentication(
                    attemptID: attemptID,
                    authorizeURL: authorizeURL,
                    origin: origin,
                    redirectURI: redirectURI,
                    expectedState: state,
                    verifier: verifier,
                    completion: completion
                )
            }
        }
    }

    private func preflight(
        authorizeURL: URL,
        attemptID: UUID,
        completion: @escaping (Result<Void, Error>) -> Void
    ) {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.httpShouldSetCookies = false
        let session = URLSession(
            configuration: configuration,
            delegate: redirectBlocker,
            delegateQueue: nil
        )
        preflightSession = session
        var request = URLRequest(url: authorizeURL)
        request.cachePolicy = .reloadIgnoringLocalCacheData
        session.dataTask(with: request) { [weak self] _, response, error in
            defer {
                session.finishTasksAndInvalidate()
                if self?.preflightSession === session {
                    self?.preflightSession = nil
                }
            }
            guard self?.activeAttemptID == attemptID else { return }
            if let error = error {
                completion(.failure(error))
                return
            }
            guard let http = response as? HTTPURLResponse else {
                completion(.failure(ScreenshotSafeUploadError.invalidResponse))
                return
            }
            if http.statusCode == 200 || (300..<400).contains(http.statusCode) {
                completion(.success(()))
            } else if http.statusCode == 400 || http.statusCode == 404 {
                completion(.failure(ScreenshotSafeAuthorizationError.incompatibleServer))
            } else {
                completion(.failure(ScreenshotSafeUploadError.server("Server returned \(http.statusCode).")))
            }
        }.resume()
    }

    private func beginAuthentication(
        attemptID: UUID,
        authorizeURL: URL,
        origin: String,
        redirectURI: String,
        expectedState: String,
        verifier: String,
        completion: @escaping (Result<ScreenshotSafeConnection, Error>) -> Void
    ) {
        let session = ASWebAuthenticationSession(
            url: authorizeURL,
            callbackURLScheme: "com.screenshotsafe"
        ) { [weak self] callbackURL, error in
            guard let self = self, self.activeAttemptID == attemptID else { return }
            self.authenticationSession = nil
            if let authenticationError = error as? ASWebAuthenticationSessionError,
               authenticationError.code == .canceledLogin {
                self.finish(
                    attemptID: attemptID,
                    result: .failure(ScreenshotSafeAuthorizationError.cancelled),
                    completion: completion
                )
                return
            }
            if let error = error {
                self.finish(attemptID: attemptID, result: .failure(error), completion: completion)
                return
            }
            do {
                let code = try self.authorizationCode(
                    callbackURL: callbackURL,
                    redirectURI: redirectURI,
                    expectedState: expectedState
                )
                self.exchange(
                    attemptID: attemptID,
                    code: code,
                    verifier: verifier,
                    redirectURI: redirectURI,
                    origin: origin,
                    completion: completion
                )
            } catch {
                self.finish(attemptID: attemptID, result: .failure(error), completion: completion)
            }
        }
        session.presentationContextProvider = self
        session.prefersEphemeralWebBrowserSession = false
        authenticationSession = session
        guard session.start() else {
            authenticationSession = nil
            finish(
                attemptID: attemptID,
                result: .failure(ScreenshotSafeAuthorizationError.couldNotStart),
                completion: completion
            )
            return
        }
    }

    private func authorizationCode(
        callbackURL: URL?,
        redirectURI: String,
        expectedState: String
    ) throws -> String {
        guard
            let callbackURL = callbackURL,
            let expectedURL = URL(string: redirectURI),
            callbackURL.scheme == expectedURL.scheme,
            callbackURL.host == nil,
            callbackURL.path == expectedURL.path,
            let components = URLComponents(url: callbackURL, resolvingAgainstBaseURL: false)
        else {
            throw ScreenshotSafeAuthorizationError.invalidCallback
        }
        var values: [String: String] = [:]
        for item in components.queryItems ?? [] {
            guard values[item.name] == nil, let value = item.value else {
                throw ScreenshotSafeAuthorizationError.invalidCallback
            }
            values[item.name] = value
        }
        guard values["state"] == expectedState else {
            throw ScreenshotSafeAuthorizationError.invalidState
        }
        if values["error"] != nil {
            throw ScreenshotSafeAuthorizationError.cancelled
        }
        guard let code = values["code"], !code.isEmpty else {
            throw ScreenshotSafeAuthorizationError.invalidCallback
        }
        return code
    }

    private func exchange(
        attemptID: UUID,
        code: String,
        verifier: String,
        redirectURI: String,
        origin: String,
        completion: @escaping (Result<ScreenshotSafeConnection, Error>) -> Void
    ) {
        guard let url = endpoint(origin: origin, path: "/api/auth/extension/token") else {
            finish(
                attemptID: attemptID,
                result: .failure(ScreenshotSafeConfigurationError.invalidServerURL),
                completion: completion
            )
            return
        }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try? JSONSerialization.data(withJSONObject: [
            "code": code,
            "code_verifier": verifier,
            "redirect_uri": redirectURI,
        ])

        URLSession.shared.dataTask(with: request) { [weak self] data, response, error in
            guard let self = self, self.activeAttemptID == attemptID else { return }
            if let error = error {
                self.finish(attemptID: attemptID, result: .failure(error), completion: completion)
                return
            }
            guard let http = response as? HTTPURLResponse, let data = data else {
                self.finish(
                    attemptID: attemptID,
                    result: .failure(ScreenshotSafeUploadError.invalidResponse),
                    completion: completion
                )
                return
            }
            guard (200..<300).contains(http.statusCode) else {
                let message = (try? JSONSerialization.jsonObject(with: data) as? [String: Any])?["error"] as? String
                self.finish(
                    attemptID: attemptID,
                    result: .failure(ScreenshotSafeAuthorizationError.tokenExchange(
                        message ?? "Token exchange failed (\(http.statusCode))."
                    )),
                    completion: completion
                )
                return
            }

            let exchange: ScreenshotSafeTokenExchangeResponse
            do {
                exchange = try JSONDecoder().decode(ScreenshotSafeTokenExchangeResponse.self, from: data)
            } catch {
                self.finish(attemptID: attemptID, result: .failure(error), completion: completion)
                return
            }
            let settings = ScreenshotSafeSettings(
                serverURL: origin,
                apiToken: exchange.token,
                defaultExpiry: self.settingsStore.loadRegistry().defaultExpiry
            )
            self.uploadClient.verify(settings: settings) { result in
                guard self.activeAttemptID == attemptID else {
                    self.uploadClient.revoke(settings: settings) { _ in }
                    return
                }
                switch result {
                case .failure(let error):
                    self.uploadClient.revoke(settings: settings) { _ in }
                    self.finish(attemptID: attemptID, result: .failure(error), completion: completion)
                case .success(let ping):
                    do {
                        let saved = try self.settingsStore.upsertVerifiedConnection(
                            origin: origin,
                            token: exchange.token,
                            displayName: ping.displayName.isEmpty ? exchange.displayName : ping.displayName,
                            username: ping.username.isEmpty ? exchange.username : ping.username
                        )
                        if let superseded = saved.superseded {
                            let oldSettings = ScreenshotSafeSettings(
                                serverURL: origin,
                                apiToken: superseded.token,
                                defaultExpiry: ""
                            )
                            self.uploadClient.revoke(settings: oldSettings) { _ in
                                self.settingsStore.deleteCredential(superseded)
                            }
                        }
                        self.finish(
                            attemptID: attemptID,
                            result: .success(saved.connection),
                            completion: completion
                        )
                    } catch {
                        self.uploadClient.revoke(settings: settings) { _ in }
                        self.finish(attemptID: attemptID, result: .failure(error), completion: completion)
                    }
                }
            }
        }.resume()
    }

    private func endpoint(origin: String, path: String, queryItems: [URLQueryItem] = []) -> URL? {
        guard let base = URL(string: origin) else {
            return nil
        }
        let url = base.appendingPathComponent(path.trimmingCharacters(in: CharacterSet(charactersIn: "/")))
        guard !queryItems.isEmpty else {
            return url
        }
        var components = URLComponents(url: url, resolvingAgainstBaseURL: false)
        components?.queryItems = queryItems
        return components?.url
    }

    private func randomBase64URL(byteCount: Int) -> String? {
        var bytes = [UInt8](repeating: 0, count: byteCount)
        guard SecRandomCopyBytes(kSecRandomDefault, byteCount, &bytes) == errSecSuccess else {
            return nil
        }
        return Data(bytes).base64URLEncodedString()
    }

    private func finish<T>(
        attemptID: UUID,
        result: Result<T, Error>,
        completion: @escaping (Result<T, Error>) -> Void
    ) {
        guard activeAttemptID == attemptID else { return }
        activeAttemptID = nil
        completionOnMain(result, completion)
    }

    private func completionOnMain<T>(
        _ result: Result<T, Error>,
        _ completion: @escaping (Result<T, Error>) -> Void
    ) {
        DispatchQueue.main.async {
            completion(result)
        }
    }
}

private extension Data {
    func base64URLEncodedString() -> String {
        base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
}

#if os(macOS)
private final class FlippedStackView: NSStackView {
    override var isFlipped: Bool { true }
}
#endif

class ViewController: PlatformViewController, WKNavigationDelegate, WKScriptMessageHandler {

    @IBOutlet var webView: WKWebView!

    private let settingsStore = ScreenshotSafeSettingsStore()
    private let uploadClient = ScreenshotSafeUploadClient()
    private var authorizationCoordinator: ScreenshotSafeAuthorizationCoordinator!
    private var refreshTimer: Timer?
    private var retentionPolicy: ScreenshotSafeRetentionPolicy?

#if os(macOS)
    private let serverURLField = NSTextField()
    private let expiryPopup = NSPopUpButton()
    private let statusLabel = NSTextField(labelWithString: "")
    private let safariStatusLabel = NSTextField(labelWithString: "")
    private let serverListStack = FlippedStackView()
    private let serverScrollView = NSScrollView()
    private var serverScrollHeightConstraint: NSLayoutConstraint!
    private let emptyServersLabel = NSTextField(labelWithString: "No ScreenshotSafe servers are connected yet.")
    private let loginButton = NSButton()
#elseif os(iOS)
    private let serverURLField = UITextField()
    private let expiryButton = UIButton(type: .system)
    private let statusLabel = UILabel()
    private let serverListStack = UIStackView()
    private let emptyServersLabel = UILabel()
    private let loginButton = UIButton(type: .system)
    private var selectedExpiry = ""
#endif

    override func viewDidLoad() {
        super.viewDidLoad()

#if os(macOS)
        authorizationCoordinator = ScreenshotSafeAuthorizationCoordinator(
            settingsStore: settingsStore,
            uploadClient: uploadClient,
            anchorProvider: { [weak self] in self?.view.window ?? NSWindow() }
        )
        buildMacSettingsView()
        refreshSafariExtensionState()
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(settingsDidChange),
            name: .screenshotSafeSettingsDidChange,
            object: nil
        )
#else
        authorizationCoordinator = ScreenshotSafeAuthorizationCoordinator(
            settingsStore: settingsStore,
            uploadClient: uploadClient,
            anchorProvider: { [weak self] in self?.view.window ?? UIWindow() }
        )
        buildIOSSettingsView()
        NotificationCenter.default.addObserver(
            self,
            selector: #selector(settingsDidChange),
            name: .screenshotSafeSettingsDidChange,
            object: nil
        )
#endif
        migrateLegacyConfiguration()
        let legacyDefaultExpiry = settingsStore.loadRegistry().defaultExpiry
        if !legacyDefaultExpiry.isEmpty,
           settingsStore.loadRegistry().defaultConnection != nil {
            saveDefaultExpiryToServer(legacyDefaultExpiry)
        }
    }

#if os(macOS)
    override func viewDidAppear() {
        super.viewDidAppear()
        startRefreshTimer()
    }

    override func viewWillDisappear() {
        super.viewWillDisappear()
        stopRefreshTimer()
    }
#elseif os(iOS)
    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        startRefreshTimer()
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        stopRefreshTimer()
    }
#endif

    private func startRefreshTimer() {
        refreshTimer?.invalidate()
        refreshTimer = Timer.scheduledTimer(withTimeInterval: 60, repeats: true) { [weak self] _ in
            self?.refreshAllConnections()
        }
    }

    private func stopRefreshTimer() {
        refreshTimer?.invalidate()
        refreshTimer = nil
    }

    deinit {
        refreshTimer?.invalidate()
        authorizationCoordinator?.cancel()
    }

    private func migrateLegacyConfiguration() {
        guard let legacy = settingsStore.legacyConfiguration() else {
            return
        }

        let origin: String
        do {
            origin = try ScreenshotSafeServerURLNormalizer.normalize(legacy.serverURL)
        } catch {
            settingsStore.clearLegacyConfiguration()
            showStatus(
                "The previous server address was invalid. Log in to add it again.",
                isError: true
            )
            return
        }

        if settingsStore.loadRegistry().connections.contains(where: { $0.origin == origin }) {
            settingsStore.clearLegacyConfiguration()
            return
        }

        showStatus("Importing your previous \(origin) connection…", isError: false)
        let settings = ScreenshotSafeSettings(
            serverURL: origin,
            apiToken: legacy.token,
            defaultExpiry: settingsStore.loadRegistry().defaultExpiry
        )
        uploadClient.verify(settings: settings) { [weak self] result in
            DispatchQueue.main.async {
                guard let self = self else { return }

                // A user may have completed a new login while migration was
                // checking the old token. Never replace that newer credential.
                if self.settingsStore.loadRegistry().connections.contains(where: {
                    $0.origin == origin
                }) {
                    self.settingsStore.clearLegacyConfiguration()
                    return
                }

                switch result {
                case .success(let ping):
                    do {
                        let hasDefault = self.settingsStore.loadRegistry().defaultConnectionID != nil
                        _ = try self.settingsStore.upsertVerifiedConnection(
                            origin: origin,
                            token: legacy.token,
                            displayName: ping.displayName,
                            username: ping.username,
                            makeDefault: !hasDefault
                        )
                        self.settingsStore.clearLegacyConfiguration()
                        self.showStatus(
                            "Imported your previous \(origin) connection.",
                            isError: false,
                            success: true
                        )
                    } catch {
                        self.showStatus(error.localizedDescription, isError: true)
                    }
                case .failure(let error):
                    let status = self.connectionStatus(for: error)
                    if status == .loginRequired || status == .accountDisabled {
                        self.settingsStore.clearLegacyConfiguration()
                        self.showStatus(
                            "Your previous \(origin) token is no longer valid. Log in again.",
                            isError: true
                        )
                    } else {
                        self.showStatus(
                            "Could not import your previous \(origin) connection yet: \(error.localizedDescription)",
                            isError: true
                        )
                    }
                }
            }
        }
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

    private func saveDefaultExpiryToServer(_ value: String) {
        let registry = settingsStore.loadRegistry()
        guard
            let connection = registry.defaultConnection,
            let token = try? settingsStore.token(for: connection)
        else {
            showStatus("Connect a ScreenshotSafe server before changing the default expiry.", isError: true)
            return
        }

        let mode: String
        let seconds: UInt64?
        if value.isEmpty {
            mode = "inherit"
            seconds = nil
        } else if value == "never" {
            mode = "never"
            seconds = nil
        } else {
            mode = "duration"
            seconds = Self.expirySeconds(from: value)
        }
        guard mode != "duration" || seconds != nil else {
            showStatus("That expiry duration is invalid.", isError: true)
            return
        }

        let settings = ScreenshotSafeSettings(
            serverURL: connection.origin,
            apiToken: token,
            defaultExpiry: ""
        )
        uploadClient.updateDefaultRetention(
            settings: settings,
            mode: mode,
            seconds: seconds
        ) { [weak self] result in
            DispatchQueue.main.async {
                guard let self = self else { return }
                switch result {
                case .success(let policy):
                    try? self.settingsStore.setDefaultExpiry("")
                    self.applyRetentionPolicy(policy)
                    self.showStatus(
                        "Default expiry saved for \(connection.origin).",
                        isError: false,
                        success: true
                    )
                case .failure(let error):
                    self.showStatus(error.localizedDescription, isError: true)
                }
            }
        }
    }

    private func applyRetentionPolicy(_ policy: ScreenshotSafeRetentionPolicy?) {
        guard let policy = policy else { return }
        retentionPolicy = policy
#if os(iOS)
        selectedExpiry = policy.editorValue
        updateExpiryButtonTitle()
        updateExpiryMenu()
#elseif os(macOS)
        let value = policy.editorValue
        for index in 0..<expiryPopup.numberOfItems {
            guard let item = expiryPopup.item(at: index),
                  let itemValue = item.representedObject as? String else { continue }
            let seconds = Self.expirySeconds(from: itemValue)
            item.isEnabled = itemValue.isEmpty
                || (itemValue == "never" && policy.allowNever)
                || (seconds != nil
                    && policy.effectiveMaxExpirySeconds.map { seconds! <= $0 } != false)
            if itemValue == value {
                expiryPopup.selectItem(at: index)
            }
        }
#endif
    }

    private static func expirySeconds(from value: String) -> UInt64? {
        guard let unit = value.last, let amount = UInt64(value.dropLast()) else {
            return nil
        }
        let multiplier: UInt64
        switch unit {
        case "m": multiplier = 60
        case "h": multiplier = 3600
        case "d": multiplier = 86400
        case "w": multiplier = 604800
        default: return nil
        }
        return amount.multipliedReportingOverflow(by: multiplier).overflow
            ? nil
            : amount * multiplier
    }

}

#if os(iOS)
private extension ViewController {
    var expiryOptions: [(title: String, value: String)] {
        let options: [(title: String, value: String)] = [
            ("Server default", ""),
            ("1 hour", "1h"),
            ("24 hours", "24h"),
            ("7 days", "7d"),
            ("30 days", "30d"),
            ("Never expire", "never"),
        ]
        guard let policy = retentionPolicy else { return options }
        return options.filter { option in
            if option.value.isEmpty { return true }
            if option.value == "never" { return policy.allowNever }
            guard let seconds = Self.expirySeconds(from: option.value) else { return false }
            return policy.effectiveMaxExpirySeconds.map { seconds <= $0 } ?? true
        }
    }

    func buildIOSSettingsView() {
        let root = UIView()
        root.backgroundColor = .systemBackground

        let scrollView = UIScrollView()
        scrollView.backgroundColor = .systemBackground
        scrollView.keyboardDismissMode = .interactive
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(scrollView)
        view = root

        let content = UIStackView()
        content.axis = .vertical
        content.spacing = 20
        content.translatesAutoresizingMaskIntoConstraints = false
        scrollView.addSubview(content)

        let titleLabel = UILabel()
        titleLabel.text = "ScreenshotSafe"
        titleLabel.font = .preferredFont(forTextStyle: .largeTitle)
        titleLabel.adjustsFontForContentSizeCategory = true

        let subtitleLabel = UILabel()
        subtitleLabel.text = "Connect ScreenshotSafe servers and choose where Safari and the share extension upload."
        subtitleLabel.font = .preferredFont(forTextStyle: .subheadline)
        subtitleLabel.textColor = .secondaryLabel
        subtitleLabel.adjustsFontForContentSizeCategory = true
        subtitleLabel.numberOfLines = 0

        let connectedHeading = sectionHeading("Connected servers")
        let refreshButton = UIButton(type: .system)
        refreshButton.setTitle("Refresh all", for: .normal)
        refreshButton.addTarget(self, action: #selector(refreshAllConnections), for: .touchUpInside)
        connectedHeading.addArrangedSubview(refreshButton)

        serverListStack.axis = .vertical
        serverListStack.spacing = 12
        emptyServersLabel.text = "No ScreenshotSafe servers are connected yet."
        emptyServersLabel.textColor = .secondaryLabel
        emptyServersLabel.numberOfLines = 0

        configureTextField(serverURLField, placeholder: "screenshots.example.com", keyboardType: .URL, secure: false)
        serverURLField.addTarget(self, action: #selector(logInToServer), for: .editingDidEndOnExit)
        configureExpiryMenu()

        let scanButton = UIButton(type: .system)
        scanButton.setTitle("Scan Setup QR Code", for: .normal)
        scanButton.addTarget(self, action: #selector(scanSetupQRCode), for: .touchUpInside)
        scanButton.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true

        loginButton.setTitle("Log in", for: .normal)
        loginButton.titleLabel?.font = .preferredFont(forTextStyle: .headline)
        loginButton.addTarget(self, action: #selector(logInToServer), for: .touchUpInside)
        loginButton.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true

        let addActions = UIStackView(arrangedSubviews: [loginButton, scanButton])
        addActions.axis = .horizontal
        addActions.spacing = 12
        addActions.distribution = .fillEqually

        statusLabel.textColor = .secondaryLabel
        statusLabel.font = .preferredFont(forTextStyle: .subheadline)
        statusLabel.adjustsFontForContentSizeCategory = true
        statusLabel.numberOfLines = 0

        content.addArrangedSubview(titleLabel)
        content.addArrangedSubview(subtitleLabel)
        content.addArrangedSubview(connectedHeading)
        content.addArrangedSubview(emptyServersLabel)
        content.addArrangedSubview(serverListStack)
        content.addArrangedSubview(formLabel("Add a server"))
        content.addArrangedSubview(serverURLField)
        content.addArrangedSubview(addActions)
        content.addArrangedSubview(expiryStack())
        content.addArrangedSubview(statusLabel)

        NSLayoutConstraint.activate([
            scrollView.leadingAnchor.constraint(equalTo: root.safeAreaLayoutGuide.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: root.safeAreaLayoutGuide.trailingAnchor),
            scrollView.topAnchor.constraint(equalTo: root.safeAreaLayoutGuide.topAnchor),
            scrollView.bottomAnchor.constraint(equalTo: root.safeAreaLayoutGuide.bottomAnchor),
            content.leadingAnchor.constraint(equalTo: scrollView.contentLayoutGuide.leadingAnchor, constant: 24),
            content.trailingAnchor.constraint(equalTo: scrollView.contentLayoutGuide.trailingAnchor, constant: -24),
            content.topAnchor.constraint(equalTo: scrollView.contentLayoutGuide.topAnchor, constant: 32),
            content.bottomAnchor.constraint(equalTo: scrollView.contentLayoutGuide.bottomAnchor, constant: -32),
            content.widthAnchor.constraint(equalTo: scrollView.frameLayoutGuide.widthAnchor, constant: -48),
        ])

        loadConnectionsIntoView()
        refreshAllConnections()
    }

    func sectionHeading(_ title: String) -> UIStackView {
        let label = UILabel()
        label.text = title
        label.font = .preferredFont(forTextStyle: .headline)
        let spacer = UIView()
        let stack = UIStackView(arrangedSubviews: [label, spacer])
        stack.axis = .horizontal
        stack.alignment = .center
        return stack
    }

    func configureTextField(_ textField: UITextField, placeholder: String, keyboardType: UIKeyboardType, secure: Bool) {
        textField.borderStyle = .roundedRect
        textField.placeholder = placeholder
        textField.keyboardType = keyboardType
        textField.autocapitalizationType = .none
        textField.autocorrectionType = .no
        textField.isSecureTextEntry = secure
        textField.returnKeyType = .done
        textField.addTarget(self, action: #selector(dismissKeyboard), for: .editingDidEndOnExit)
        textField.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
    }

    func configureExpiryMenu() {
        expiryButton.contentHorizontalAlignment = .leading
        expiryButton.showsMenuAsPrimaryAction = true
        expiryButton.heightAnchor.constraint(greaterThanOrEqualToConstant: 44).isActive = true
        updateExpiryButtonTitle()
        updateExpiryMenu()
    }

    func updateExpiryMenu() {
        expiryButton.menu = UIMenu(children: expiryOptions.map { option in
            UIAction(title: option.title, state: option.value == selectedExpiry ? .on : .off) { [weak self] _ in
                guard let self = self else { return }
                self.selectedExpiry = option.value
                self.updateExpiryButtonTitle()
                self.updateExpiryMenu()
                self.saveDefaultExpiryToServer(option.value)
            }
        })
    }

    func updateExpiryButtonTitle() {
        let title = expiryOptions.first(where: { $0.value == selectedExpiry })?.title ?? "Server default"
        expiryButton.setTitle(title, for: .normal)
    }

    func fieldStack(label: String, field: UITextField) -> UIStackView {
        let labelView = formLabel(label)
        let stack = UIStackView(arrangedSubviews: [labelView, field])
        stack.axis = .vertical
        stack.spacing = 8
        return stack
    }

    func expiryStack() -> UIStackView {
        let stack = UIStackView(arrangedSubviews: [formLabel("Default Expiry"), expiryButton])
        stack.axis = .vertical
        stack.spacing = 8
        return stack
    }

    func formLabel(_ text: String) -> UILabel {
        let label = UILabel()
        label.text = text
        label.textColor = .secondaryLabel
        label.font = .preferredFont(forTextStyle: .subheadline)
        label.adjustsFontForContentSizeCategory = true
        return label
    }

    @objc func scanSetupQRCode() {
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            presentQRCodeScanner()
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .video) { [weak self] granted in
                DispatchQueue.main.async {
                    if granted {
                        self?.presentQRCodeScanner()
                    } else {
                        self?.showQRScanError("Camera access is needed to scan setup QR codes.")
                    }
                }
            }
        default:
            showQRScanError("Camera access is needed to scan setup QR codes.")
        }
    }

    func presentQRCodeScanner() {
        let scanner = QRCodeScannerViewController()
        scanner.onCodeScanned = { [weak self] value in
            self?.handleScannedQRCode(value)
        }
        scanner.modalPresentationStyle = .fullScreen
        present(scanner, animated: true)
    }

    func handleScannedQRCode(_ value: String) {
        let configuration: (origin: String, token: String)
        do {
            configuration = try settingsStore.parseQRCode(value)
        } catch {
            showQRScanError(error.localizedDescription)
            return
        }
        showStatus("Checking \(configuration.origin)…", isError: false)
        let settings = ScreenshotSafeSettings(
            serverURL: configuration.origin,
            apiToken: configuration.token,
            defaultExpiry: settingsStore.loadRegistry().defaultExpiry
        )
        uploadClient.verify(settings: settings) { [weak self] result in
            DispatchQueue.main.async {
                guard let self = self else { return }
                switch result {
                case .failure(let error):
                    self.showQRScanError(error.localizedDescription)
                case .success(let ping):
                    do {
                        let saved = try self.settingsStore.upsertVerifiedConnection(
                            origin: configuration.origin,
                            token: configuration.token,
                            displayName: ping.displayName,
                            username: ping.username
                        )
                        self.finishSupersededCredential(saved.superseded, origin: configuration.origin)
                        self.loadConnectionsIntoView()
                        self.showStatus("Connected to \(configuration.origin). It is now used for uploads.", isError: false, success: true)
                    } catch {
                        self.showQRScanError(error.localizedDescription)
                    }
                }
            }
        }
    }

    func showQRScanError(_ message: String) {
        statusLabel.text = message
        statusLabel.textColor = .systemRed
    }

    func loadConnectionsIntoView() {
        let registry = settingsStore.loadRegistry()
        selectedExpiry = registry.defaultExpiry
        updateExpiryButtonTitle()
        updateExpiryMenu()
        serverListStack.arrangedSubviews.forEach {
            serverListStack.removeArrangedSubview($0)
            $0.removeFromSuperview()
        }
        emptyServersLabel.isHidden = !registry.connections.isEmpty
        for connection in registry.connections {
            serverListStack.addArrangedSubview(
                connectionRow(connection, isDefault: connection.id == registry.defaultConnectionID)
            )
        }
    }

    @objc func logInToServer() {
        dismissKeyboard()
        let input = serverURLField.text ?? ""
        loginButton.isEnabled = false
        loginButton.setTitle("Opening login…", for: .normal)
        showStatus("Connecting to \(input)…", isError: false)
        authorizationCoordinator.logIn(serverInput: input) { [weak self] result in
            guard let self = self else { return }
            self.loginButton.isEnabled = true
            self.loginButton.setTitle("Log in", for: .normal)
            switch result {
            case .success(let connection):
                self.serverURLField.text = ""
                self.loadConnectionsIntoView()
                self.showStatus("Connected to \(connection.origin). It is now used for uploads.", isError: false, success: true)
            case .failure(let error):
                self.showStatus(error.localizedDescription, isError: true)
            }
        }
    }

    @objc func settingsDidChange() {
        guard Thread.isMainThread else {
            DispatchQueue.main.async { [weak self] in
                self?.settingsDidChange()
            }
            return
        }
        loadConnectionsIntoView()
    }

    @objc func dismissKeyboard() {
        view.endEditing(true)
    }

    @objc func refreshAllConnections() {
        let registry = settingsStore.loadRegistry()
        for connection in registry.connections {
            refreshConnection(connection.id)
        }
    }

    @objc func selectDefaultConnection(_ sender: UIButton) {
        guard let id = connectionID(from: sender) else { return }
        do {
            try settingsStore.setDefaultConnection(id: id)
            loadConnectionsIntoView()
            if let connection = settingsStore.loadRegistry().connections.first(where: { $0.id == id }) {
                showStatus("Uploads will use \(connection.origin).", isError: false, success: true)
            }
        } catch {
            showStatus(error.localizedDescription, isError: true)
        }
    }

    @objc func retryConnection(_ sender: UIButton) {
        guard let id = connectionID(from: sender) else { return }
        refreshConnection(id)
    }

    @objc func reconnectConnection(_ sender: UIButton) {
        guard
            let id = connectionID(from: sender),
            let connection = settingsStore.loadRegistry().connections.first(where: { $0.id == id })
        else { return }
        serverURLField.text = connection.origin
        logInToServer()
    }

    @objc func logOutConnection(_ sender: UIButton) {
        guard
            let id = connectionID(from: sender),
            let connection = settingsStore.loadRegistry().connections.first(where: { $0.id == id })
        else { return }
        let token = try? settingsStore.token(for: connection)
        guard let token = token else {
            removeConnection(id)
            return
        }
        let settings = ScreenshotSafeSettings(serverURL: connection.origin, apiToken: token, defaultExpiry: "")
        uploadClient.revoke(settings: settings) { [weak self] revoked in
            DispatchQueue.main.async {
                guard let self = self else { return }
                if revoked {
                    self.removeConnection(id)
                } else {
                    let alert = UIAlertController(
                        title: "Could not revoke token",
                        message: "Forget this server locally anyway? Revoke its token from the server settings later.",
                        preferredStyle: .alert
                    )
                    alert.addAction(UIAlertAction(title: "Cancel", style: .cancel))
                    alert.addAction(UIAlertAction(title: "Forget", style: .destructive) { _ in
                        self.removeConnection(id)
                    })
                    self.present(alert, animated: true)
                }
            }
        }
    }

    func connectionRow(_ connection: ScreenshotSafeConnection, isDefault: Bool) -> UIView {
        let container = UIView()
        container.backgroundColor = .secondarySystemBackground
        container.layer.cornerRadius = 10

        let defaultButton = UIButton(type: .system)
        defaultButton.setTitle(isDefault ? "✓" : "○", for: .normal)
        defaultButton.titleLabel?.font = .systemFont(ofSize: 22, weight: .semibold)
        defaultButton.accessibilityLabel = "Use \(connection.origin) for uploads"
        defaultButton.accessibilityIdentifier = connection.id.uuidString
        defaultButton.addTarget(self, action: #selector(selectDefaultConnection), for: .touchUpInside)

        let origin = UILabel()
        origin.text = connection.origin
        origin.font = .preferredFont(forTextStyle: .headline)
        origin.numberOfLines = 1
        origin.adjustsFontForContentSizeCategory = true

        let account = UILabel()
        account.text = connection.displayName.isEmpty
            ? "Not logged in"
            : "\(connection.displayName)\(connection.username.isEmpty ? "" : " (\(connection.username))")"
        account.textColor = .secondaryLabel
        account.font = .preferredFont(forTextStyle: .caption1)
        account.numberOfLines = 1

        let details = UIStackView(arrangedSubviews: [origin, account])
        details.axis = .vertical
        details.spacing = 2
        let header = UIStackView(arrangedSubviews: [defaultButton, details])
        header.axis = .horizontal
        header.spacing = 10
        header.alignment = .center

        let state = UILabel()
        state.text = statusText(connection)
        state.textColor = statusColor(connection.lastStatus)
        state.font = .preferredFont(forTextStyle: .caption1)

        let retry = connectionButton("Retry", id: connection.id, action: #selector(retryConnection))
        let reconnect = connectionButton("Log in again", id: connection.id, action: #selector(reconnectConnection))
        reconnect.isHidden = connection.lastStatus == .connected
        let logout = connectionButton("Log out", id: connection.id, action: #selector(logOutConnection))
        logout.setTitleColor(.systemRed, for: .normal)
        let actions = UIStackView(arrangedSubviews: [retry, reconnect, logout])
        actions.axis = .horizontal
        actions.spacing = 12

        let stack = UIStackView(arrangedSubviews: [header, state, actions])
        stack.axis = .vertical
        stack.spacing = 8
        stack.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 12),
            stack.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -12),
            stack.topAnchor.constraint(equalTo: container.topAnchor, constant: 12),
            stack.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -12),
        ])
        return container
    }

    func connectionButton(_ title: String, id: UUID, action: Selector) -> UIButton {
        let button = UIButton(type: .system)
        button.setTitle(title, for: .normal)
        button.accessibilityIdentifier = id.uuidString
        button.addTarget(self, action: action, for: .touchUpInside)
        return button
    }

    func connectionID(from button: UIButton) -> UUID? {
        button.accessibilityIdentifier.flatMap(UUID.init(uuidString:))
    }

    func refreshConnection(_ id: UUID) {
        guard let connection = settingsStore.loadRegistry().connections.first(where: { $0.id == id }) else {
            return
        }
        try? settingsStore.updateConnection(id: id, status: .checking)
        loadConnectionsIntoView()
        guard let token = try? settingsStore.token(for: connection) else {
            try? settingsStore.updateConnection(id: id, status: .loginRequired)
            loadConnectionsIntoView()
            return
        }
        let settings = ScreenshotSafeSettings(serverURL: connection.origin, apiToken: token, defaultExpiry: "")
        uploadClient.verify(settings: settings) { [weak self] result in
            DispatchQueue.main.async {
                guard let self = self else { return }
                switch result {
                case .success(let ping):
                    try? self.settingsStore.updateConnection(
                        id: id,
                        status: .connected,
                        displayName: ping.displayName,
                        username: ping.username
                    )
                    if self.settingsStore.loadRegistry().defaultConnectionID == id {
                        self.applyRetentionPolicy(ping.retention)
                    }
                case .failure(let error):
                    try? self.settingsStore.updateConnection(id: id, status: self.connectionStatus(for: error))
                }
                self.loadConnectionsIntoView()
            }
        }
    }

    func removeConnection(_ id: UUID) {
        do {
            let removed = try settingsStore.removeConnection(id: id)
            loadConnectionsIntoView()
            showStatus("Removed \(removed.origin).", isError: false, success: true)
        } catch {
            showStatus(error.localizedDescription, isError: true)
        }
    }

    func finishSupersededCredential(
        _ credential: ScreenshotSafeSupersededCredential?,
        origin: String
    ) {
        guard let credential = credential else { return }
        let settings = ScreenshotSafeSettings(serverURL: origin, apiToken: credential.token, defaultExpiry: "")
        uploadClient.revoke(settings: settings) { [weak self] _ in
            self?.settingsStore.deleteCredential(credential)
        }
    }

    func connectionStatus(for error: Error) -> ScreenshotSafeConnectionStatus {
        let message = error.localizedDescription.lowercased()
        if message.contains("token was rejected") { return .loginRequired }
        if message.contains("account is disabled") { return .accountDisabled }
        if message.contains("needs an update") { return .incompatible }
        if error is URLError { return .unreachable }
        return .serverError
    }

    func statusText(_ connection: ScreenshotSafeConnection) -> String {
        let label: String
        switch connection.lastStatus {
        case .connected: label = "Connected"
        case .loginRequired: label = "Login required"
        case .accountDisabled: label = "Account disabled"
        case .unreachable: label = "Server unreachable"
        case .incompatible: label = "Server needs an update"
        case .serverError: label = "Server error"
        case .checking: label = "Checking…"
        }
        guard let date = connection.lastCheckedAt else { return label }
        return "\(label) · \(date.formatted(date: .omitted, time: .shortened))"
    }

    func statusColor(_ status: ScreenshotSafeConnectionStatus) -> UIColor {
        switch status {
        case .connected: return .systemGreen
        case .checking: return .secondaryLabel
        default: return .systemRed
        }
    }

    func showStatus(_ message: String, isError: Bool, success: Bool = false) {
        statusLabel.text = message
        statusLabel.textColor = isError ? .systemRed : (success ? .systemGreen : .secondaryLabel)
    }
}

private final class QRCodeScannerViewController: UIViewController, AVCaptureMetadataOutputObjectsDelegate {
    var onCodeScanned: ((String) -> Void)?

    private let session = AVCaptureSession()
    private var previewLayer: AVCaptureVideoPreviewLayer?
    private let statusLabel = UILabel()

    override func viewDidLoad() {
        super.viewDidLoad()

        view.backgroundColor = .black
        configureScanner()
        configureOverlay()
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        previewLayer?.frame = view.bounds
    }

    override func viewWillAppear(_ animated: Bool) {
        super.viewWillAppear(animated)
        if !session.isRunning {
            DispatchQueue.global(qos: .userInitiated).async { [session] in
                session.startRunning()
            }
        }
    }

    override func viewWillDisappear(_ animated: Bool) {
        super.viewWillDisappear(animated)
        if session.isRunning {
            session.stopRunning()
        }
    }

    private func configureScanner() {
        guard
            let device = AVCaptureDevice.default(for: .video),
            let input = try? AVCaptureDeviceInput(device: device),
            session.canAddInput(input)
        else {
            showScannerUnavailable()
            return
        }

        session.addInput(input)

        let output = AVCaptureMetadataOutput()
        guard session.canAddOutput(output) else {
            showScannerUnavailable()
            return
        }

        session.addOutput(output)
        output.setMetadataObjectsDelegate(self, queue: .main)
        output.metadataObjectTypes = [.qr]

        let previewLayer = AVCaptureVideoPreviewLayer(session: session)
        previewLayer.videoGravity = .resizeAspectFill
        previewLayer.frame = view.bounds
        view.layer.insertSublayer(previewLayer, at: 0)
        self.previewLayer = previewLayer
    }

    private func configureOverlay() {
        let cancelButton = UIButton(type: .system)
        var cancelConfiguration = UIButton.Configuration.plain()
        cancelConfiguration.title = "Cancel"
        cancelConfiguration.contentInsets = NSDirectionalEdgeInsets(top: 8, leading: 14, bottom: 8, trailing: 14)
        cancelButton.configuration = cancelConfiguration
        cancelButton.tintColor = .white
        cancelButton.backgroundColor = UIColor.black.withAlphaComponent(0.45)
        cancelButton.layer.cornerRadius = 8
        cancelButton.addTarget(self, action: #selector(cancelScan), for: .touchUpInside)
        cancelButton.translatesAutoresizingMaskIntoConstraints = false

        if statusLabel.text == nil {
            statusLabel.text = "Scan the ScreenshotSafe setup QR code."
        }
        statusLabel.textColor = .white
        statusLabel.font = .preferredFont(forTextStyle: .headline)
        statusLabel.adjustsFontForContentSizeCategory = true
        statusLabel.textAlignment = .center
        statusLabel.numberOfLines = 0
        statusLabel.backgroundColor = UIColor.black.withAlphaComponent(0.45)
        statusLabel.layer.cornerRadius = 8
        statusLabel.layer.masksToBounds = true
        statusLabel.translatesAutoresizingMaskIntoConstraints = false

        view.addSubview(cancelButton)
        view.addSubview(statusLabel)

        NSLayoutConstraint.activate([
            cancelButton.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 16),
            cancelButton.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -16),
            statusLabel.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 24),
            statusLabel.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -24),
            statusLabel.bottomAnchor.constraint(equalTo: view.safeAreaLayoutGuide.bottomAnchor, constant: -32),
        ])
    }

    private func showScannerUnavailable() {
        statusLabel.text = "QR scanning is unavailable on this device."
    }

    @objc private func cancelScan() {
        dismiss(animated: true)
    }

    func metadataOutput(
        _ output: AVCaptureMetadataOutput,
        didOutput metadataObjects: [AVMetadataObject],
        from connection: AVCaptureConnection
    ) {
        guard
            let code = metadataObjects.first as? AVMetadataMachineReadableCodeObject,
            let value = code.stringValue
        else {
            return
        }

        session.stopRunning()
        dismiss(animated: true) { [onCodeScanned] in
            onCodeScanned?(value)
        }
    }
}
#endif

#if os(macOS)
private extension ViewController {
    func buildMacSettingsView() {
        let root = NSView()
        root.translatesAutoresizingMaskIntoConstraints = false
        view = root

        let title = NSTextField(labelWithString: "ScreenshotSafe")
        title.font = .systemFont(ofSize: 28, weight: .semibold)
        title.textColor = .labelColor

        let subtitle = NSTextField(labelWithString: "Connect ScreenshotSafe servers and choose where Safari and the Share Extension upload.")
        subtitle.font = .systemFont(ofSize: 13)
        subtitle.textColor = .secondaryLabelColor
        subtitle.maximumNumberOfLines = 2
        subtitle.lineBreakMode = .byWordWrapping

        serverURLField.placeholderString = "https://screenshots.example.com"
        serverURLField.target = self
        serverURLField.action = #selector(logInToServer)

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
        expiryPopup.target = self
        expiryPopup.action = #selector(defaultExpiryDidChange)

        loginButton.title = "Log in"
        loginButton.target = self
        loginButton.action = #selector(logInToServer)
        loginButton.bezelStyle = .rounded

        let safariButton = NSButton(title: "Open Safari Extension Settings", target: self, action: #selector(openSafariExtensionSettings))
        safariButton.bezelStyle = .rounded

        statusLabel.textColor = .secondaryLabelColor
        statusLabel.maximumNumberOfLines = 2
        statusLabel.isHidden = true
        safariStatusLabel.textColor = .secondaryLabelColor
        safariStatusLabel.maximumNumberOfLines = 2

        let connectedTitle = NSTextField(labelWithString: "Connected servers")
        connectedTitle.font = .systemFont(ofSize: 16, weight: .semibold)
        let refreshButton = NSButton(title: "Refresh all", target: self, action: #selector(refreshAllConnections))
        refreshButton.bezelStyle = .rounded
        let connectedHeader = NSStackView(views: [connectedTitle, NSView(), refreshButton])
        connectedHeader.orientation = .horizontal
        connectedHeader.alignment = .centerY
        connectedHeader.distribution = .fill
        connectedHeader.widthAnchor.constraint(equalToConstant: 640).isActive = true

        serverListStack.orientation = .vertical
        serverListStack.alignment = .leading
        serverListStack.spacing = 10
        serverScrollView.documentView = serverListStack
        serverScrollView.drawsBackground = false
        serverScrollView.borderType = .bezelBorder
        serverScrollView.hasVerticalScroller = false
        serverScrollView.hasHorizontalScroller = false
        serverScrollView.widthAnchor.constraint(equalToConstant: 640).isActive = true
        serverScrollHeightConstraint = serverScrollView.heightAnchor.constraint(equalToConstant: 56)
        serverScrollHeightConstraint.isActive = true
        emptyServersLabel.textColor = .secondaryLabelColor

        let addTitle = NSTextField(labelWithString: "Add a server")
        addTitle.font = .systemFont(ofSize: 16, weight: .semibold)
        let serverEntryRow = NSStackView(views: [serverURLField, loginButton])
        serverEntryRow.orientation = .horizontal
        serverEntryRow.alignment = .centerY
        serverEntryRow.spacing = 8
        serverURLField.widthAnchor.constraint(greaterThanOrEqualToConstant: 350).isActive = true
        let form = NSGridView(views: [
            [fieldLabel("ScreenshotSafe server"), serverEntryRow],
            [fieldLabel("Default Expiry"), expiryPopup],
        ])
        form.column(at: 0).xPlacement = .trailing
        form.column(at: 1).width = 490
        form.rowSpacing = 8
        form.columnSpacing = 12

        let safariRow = NSStackView(views: [safariStatusLabel, NSView(), safariButton])
        safariRow.orientation = .horizontal
        safariRow.alignment = .centerY
        safariRow.distribution = .fill
        safariRow.spacing = 10
        safariRow.widthAnchor.constraint(equalToConstant: 640).isActive = true

        let stack = NSStackView(views: [
            title,
            subtitle,
            connectedHeader,
            emptyServersLabel,
            serverScrollView,
            addTitle,
            form,
            statusLabel,
            safariRow,
        ])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 12
        stack.translatesAutoresizingMaskIntoConstraints = false
        root.addSubview(stack)

        NSLayoutConstraint.activate([
            root.widthAnchor.constraint(greaterThanOrEqualToConstant: 720),
            root.heightAnchor.constraint(greaterThanOrEqualToConstant: 480),
            stack.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 32),
            stack.trailingAnchor.constraint(lessThanOrEqualTo: root.trailingAnchor, constant: -32),
            stack.topAnchor.constraint(equalTo: root.topAnchor, constant: 24),
            stack.bottomAnchor.constraint(lessThanOrEqualTo: root.bottomAnchor, constant: -24),
        ])

        loadConnectionsIntoView()
        refreshAllConnections()
    }

    func loadConnectionsIntoView() {
        let registry = settingsStore.loadRegistry()
        for index in 0..<expiryPopup.numberOfItems {
            if expiryPopup.item(at: index)?.representedObject as? String == registry.defaultExpiry {
                expiryPopup.selectItem(at: index)
                break
            }
        }
        serverListStack.arrangedSubviews.forEach {
            serverListStack.removeArrangedSubview($0)
            $0.removeFromSuperview()
        }
        emptyServersLabel.isHidden = !registry.connections.isEmpty
        serverScrollView.isHidden = registry.connections.isEmpty
        for connection in registry.connections {
            let row = connectionRow(connection, isDefault: connection.id == registry.defaultConnectionID)
            serverListStack.addArrangedSubview(row)
            row.widthAnchor.constraint(equalToConstant: 620).isActive = true
        }
        let fittingSize = serverListStack.fittingSize
        let listHeight = max(fittingSize.height, 1)
        serverListStack.frame = NSRect(
            x: 0,
            y: 0,
            width: 620,
            height: listHeight
        )
        let maximumVisibleHeight: CGFloat = 220
        serverScrollHeightConstraint.constant = min(listHeight + 2, maximumVisibleHeight)
        serverScrollView.hasVerticalScroller = listHeight + 2 > maximumVisibleHeight
    }

    @objc func settingsDidChange() {
        guard Thread.isMainThread else {
            DispatchQueue.main.async { [weak self] in
                self?.settingsDidChange()
            }
            return
        }
        loadConnectionsIntoView()
    }

    @objc func logInToServer() {
        let input = serverURLField.stringValue
        loginButton.isEnabled = false
        loginButton.title = "Opening login…"
        showStatus("Connecting to \(input)…", isError: false)
        authorizationCoordinator.logIn(serverInput: input) { [weak self] result in
            guard let self = self else { return }
            self.loginButton.isEnabled = true
            self.loginButton.title = "Log in"
            switch result {
            case .success(let connection):
                self.serverURLField.stringValue = ""
                self.loadConnectionsIntoView()
                self.showStatus("Connected to \(connection.origin). It is now used for uploads.", isError: false, success: true)
            case .failure(let error):
                self.showStatus(error.localizedDescription, isError: true)
            }
        }
    }

    @objc func defaultExpiryDidChange() {
        saveDefaultExpiryToServer(
            expiryPopup.selectedItem?.representedObject as? String ?? ""
        )
    }

    @objc func refreshAllConnections() {
        for connection in settingsStore.loadRegistry().connections {
            refreshConnection(connection.id)
        }
    }

    @objc func selectDefaultConnection(_ sender: NSButton) {
        guard let id = connectionID(from: sender) else { return }
        do {
            try settingsStore.setDefaultConnection(id: id)
            loadConnectionsIntoView()
            if let connection = settingsStore.loadRegistry().connections.first(where: { $0.id == id }) {
                showStatus("Uploads will use \(connection.origin).", isError: false, success: true)
            }
        } catch {
            showStatus(error.localizedDescription, isError: true)
        }
    }

    @objc func retryConnection(_ sender: NSButton) {
        guard let id = connectionID(from: sender) else { return }
        refreshConnection(id)
    }

    @objc func reconnectConnection(_ sender: NSButton) {
        guard
            let id = connectionID(from: sender),
            let connection = settingsStore.loadRegistry().connections.first(where: { $0.id == id })
        else { return }
        serverURLField.stringValue = connection.origin
        logInToServer()
    }

    @objc func logOutConnection(_ sender: NSButton) {
        guard
            let id = connectionID(from: sender),
            let connection = settingsStore.loadRegistry().connections.first(where: { $0.id == id })
        else { return }
        let storedToken = try? settingsStore.token(for: connection)
        guard let token = storedToken else {
            removeConnection(id)
            return
        }
        let settings = ScreenshotSafeSettings(serverURL: connection.origin, apiToken: token, defaultExpiry: "")
        uploadClient.revoke(settings: settings) { [weak self] revoked in
            DispatchQueue.main.async {
                guard let self = self else { return }
                if revoked {
                    self.removeConnection(id)
                    return
                }
                let alert = NSAlert()
                alert.messageText = "Could not revoke token"
                alert.informativeText = "Forget this server locally anyway? Revoke its token from the server settings later."
                alert.addButton(withTitle: "Forget")
                alert.addButton(withTitle: "Cancel")
                if alert.runModal() == .alertFirstButtonReturn {
                    self.removeConnection(id)
                }
            }
        }
    }

    func connectionRow(_ connection: ScreenshotSafeConnection, isDefault: Bool) -> NSView {
        let container = NSView()
        container.wantsLayer = true
        container.layer?.cornerRadius = 8
        container.layer?.backgroundColor = NSColor.controlBackgroundColor.cgColor

        let defaultButton = NSButton(
            radioButtonWithTitle: "",
            target: self,
            action: #selector(selectDefaultConnection)
        )
        defaultButton.state = isDefault ? .on : .off
        defaultButton.identifier = NSUserInterfaceItemIdentifier(connection.id.uuidString)
        defaultButton.setAccessibilityLabel("Use \(connection.origin) for uploads")

        let origin = NSTextField(labelWithString: connection.origin)
        origin.font = .systemFont(ofSize: 13, weight: .semibold)
        origin.lineBreakMode = .byTruncatingMiddle
        let accountText = connection.displayName.isEmpty
            ? "Not logged in"
            : "\(connection.displayName)\(connection.username.isEmpty ? "" : " (\(connection.username))")"
        let account = NSTextField(labelWithString: accountText)
        account.font = .systemFont(ofSize: 11)
        account.textColor = .secondaryLabelColor
        let details = NSStackView(views: [origin, account])
        details.orientation = .vertical
        details.alignment = .leading
        details.spacing = 2
        details.widthAnchor.constraint(equalToConstant: 260).isActive = true

        let state = NSTextField(labelWithString: statusText(connection))
        state.font = .systemFont(ofSize: 11)
        state.textColor = statusColor(connection.lastStatus)
        state.widthAnchor.constraint(equalToConstant: 120).isActive = true

        let retry = connectionButton("Retry", id: connection.id, action: #selector(retryConnection))
        let reconnect = connectionButton("Log in again", id: connection.id, action: #selector(reconnectConnection))
        reconnect.isHidden = connection.lastStatus == .connected
        let logout = connectionButton("Log out", id: connection.id, action: #selector(logOutConnection))
        logout.contentTintColor = .systemRed
        let actions = NSStackView(views: [retry, reconnect, logout])
        actions.orientation = .horizontal
        actions.spacing = 6

        let row = NSStackView(views: [defaultButton, details, state, actions])
        row.orientation = .horizontal
        row.alignment = .centerY
        row.spacing = 10
        row.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(row)
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 12),
            row.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -12),
            row.topAnchor.constraint(equalTo: container.topAnchor, constant: 10),
            row.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -10),
        ])
        return container
    }

    func connectionButton(_ title: String, id: UUID, action: Selector) -> NSButton {
        let button = NSButton(title: title, target: self, action: action)
        button.bezelStyle = .rounded
        button.controlSize = .small
        button.identifier = NSUserInterfaceItemIdentifier(id.uuidString)
        return button
    }

    func connectionID(from button: NSButton) -> UUID? {
        button.identifier.flatMap { UUID(uuidString: $0.rawValue) }
    }

    func refreshConnection(_ id: UUID) {
        guard let connection = settingsStore.loadRegistry().connections.first(where: { $0.id == id }) else {
            return
        }
        try? settingsStore.updateConnection(id: id, status: .checking)
        loadConnectionsIntoView()
        let storedToken = try? settingsStore.token(for: connection)
        guard let token = storedToken else {
            try? settingsStore.updateConnection(id: id, status: .loginRequired)
            loadConnectionsIntoView()
            return
        }
        let settings = ScreenshotSafeSettings(serverURL: connection.origin, apiToken: token, defaultExpiry: "")
        uploadClient.verify(settings: settings) { [weak self] result in
            DispatchQueue.main.async {
                guard let self = self else { return }
                switch result {
                case .success(let ping):
                    try? self.settingsStore.updateConnection(
                        id: id,
                        status: .connected,
                        displayName: ping.displayName,
                        username: ping.username
                    )
                    if self.settingsStore.loadRegistry().defaultConnectionID == id {
                        self.applyRetentionPolicy(ping.retention)
                    }
                case .failure(let error):
                    try? self.settingsStore.updateConnection(id: id, status: self.connectionStatus(for: error))
                }
                self.loadConnectionsIntoView()
            }
        }
    }

    func removeConnection(_ id: UUID) {
        do {
            let removed = try settingsStore.removeConnection(id: id)
            loadConnectionsIntoView()
            showStatus("Removed \(removed.origin).", isError: false, success: true)
        } catch {
            showStatus(error.localizedDescription, isError: true)
        }
    }

    func connectionStatus(for error: Error) -> ScreenshotSafeConnectionStatus {
        let message = error.localizedDescription.lowercased()
        if message.contains("token was rejected") { return .loginRequired }
        if message.contains("account is disabled") { return .accountDisabled }
        if message.contains("needs an update") { return .incompatible }
        if error is URLError { return .unreachable }
        return .serverError
    }

    func statusText(_ connection: ScreenshotSafeConnection) -> String {
        let label: String
        switch connection.lastStatus {
        case .connected: label = "Connected"
        case .loginRequired: label = "Login required"
        case .accountDisabled: label = "Account disabled"
        case .unreachable: label = "Server unreachable"
        case .incompatible: label = "Server needs an update"
        case .serverError: label = "Server error"
        case .checking: label = "Checking…"
        }
        guard let date = connection.lastCheckedAt else { return label }
        let formatter = DateFormatter()
        formatter.dateStyle = .none
        formatter.timeStyle = .short
        return "\(label) · \(formatter.string(from: date))"
    }

    func statusColor(_ status: ScreenshotSafeConnectionStatus) -> NSColor {
        switch status {
        case .connected: return .systemGreen
        case .checking: return .secondaryLabelColor
        default: return .systemRed
        }
    }

    func showStatus(_ message: String, isError: Bool, success: Bool = false) {
        statusLabel.stringValue = message
        statusLabel.isHidden = message.isEmpty
        statusLabel.textColor = isError ? .systemRed : (success ? .systemGreen : .secondaryLabelColor)
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
