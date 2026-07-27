import Foundation
import Security

struct ScreenshotSafeSettings {
    var serverURL: String
    var apiToken: String
    var defaultExpiry: String

    var isConfigured: Bool {
        !serverURL.isEmpty && !apiToken.isEmpty
    }
}

enum ScreenshotSafeConnectionStatus: String, Codable {
    case connected
    case loginRequired = "login_required"
    case accountDisabled = "account_disabled"
    case unreachable
    case incompatible
    case serverError = "server_error"
    case checking
}

struct ScreenshotSafeConnection: Codable, Identifiable, Equatable {
    var id: UUID
    var credentialID: UUID
    var origin: String
    var displayName: String
    var username: String
    var lastStatus: ScreenshotSafeConnectionStatus
    var lastCheckedAt: Date?
}

struct ScreenshotSafeConnectionRegistry: Codable {
    static let currentSchemaVersion = 2

    var schemaVersion = currentSchemaVersion
    var defaultConnectionID: UUID?
    var connections: [ScreenshotSafeConnection] = []
    var defaultExpiry = ""

    var defaultConnection: ScreenshotSafeConnection? {
        guard let defaultConnectionID = defaultConnectionID else {
            return nil
        }
        return connections.first { $0.id == defaultConnectionID }
    }
}

struct ScreenshotSafeSupersededCredential {
    let credentialID: UUID
    let token: String
}

struct ScreenshotSafeQRConfiguration: Decodable {
    let type: String
    let version: Int
    let serverURL: String
    let token: String

    enum CodingKeys: String, CodingKey {
        case type
        case version
        case serverURL = "server_url"
        case token
    }
}

struct ScreenshotSafeLegacyConfiguration {
    let serverURL: String
    let token: String
}

enum ScreenshotSafeConfigurationError: LocalizedError {
    case invalidServerURL
    case insecureServerURL
    case invalidQRCode
    case unsupportedQRCode
    case invalidToken
    case connectionNotFound
    case appGroupUnavailable
    case keychain(OSStatus)
    case storage(Error)

    var errorDescription: String? {
        switch self {
        case .invalidServerURL:
            return "Enter a valid ScreenshotSafe domain, like screenshots.example.com."
        case .insecureServerURL:
            return "Use HTTPS. Plain HTTP is allowed only for localhost development."
        case .invalidQRCode:
            return "That is not a ScreenshotSafe setup QR code."
        case .unsupportedQRCode:
            return "That setup QR code was created by an incompatible ScreenshotSafe server."
        case .invalidToken:
            return "The setup QR code contains an invalid ScreenshotSafe token."
        case .connectionNotFound:
            return "That ScreenshotSafe server is no longer configured."
        case .appGroupUnavailable:
            return "The ScreenshotSafe app group is unavailable."
        case .keychain(let status):
            return "ScreenshotSafe could not access the shared Keychain (\(status))."
        case .storage(let error):
            return "ScreenshotSafe could not save its server list: \(error.localizedDescription)"
        }
    }
}

extension Notification.Name {
    static let screenshotSafeSettingsDidChange = Notification.Name("ScreenshotSafeSettingsDidChange")
}

enum ScreenshotSafeServerURLNormalizer {
    static func normalize(_ value: String) throws -> String {
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            throw ScreenshotSafeConfigurationError.invalidServerURL
        }

        let candidate: String
        if trimmed.range(of: #"^[A-Za-z][A-Za-z0-9+.-]*://"#, options: .regularExpression) != nil {
            candidate = trimmed
        } else {
            candidate = "\(looksLocal(trimmed) ? "http" : "https")://\(trimmed)"
        }

        guard
            let components = URLComponents(string: candidate),
            let scheme = components.scheme?.lowercased(),
            let host = components.host?.lowercased(),
            !host.isEmpty,
            components.user == nil,
            components.password == nil,
            components.query == nil,
            components.fragment == nil,
            components.path.isEmpty || components.path == "/",
            scheme == "https" || scheme == "http"
        else {
            throw ScreenshotSafeConfigurationError.invalidServerURL
        }

        if scheme == "http" && !isLoopback(host) {
            throw ScreenshotSafeConfigurationError.insecureServerURL
        }

        var origin = URLComponents()
        origin.scheme = scheme
        origin.host = host
        origin.port = components.port
        guard let result = origin.string, URL(string: result) != nil else {
            throw ScreenshotSafeConfigurationError.invalidServerURL
        }
        return result
    }

    private static func looksLocal(_ value: String) -> Bool {
        let lowered = value.lowercased()
        return lowered == "localhost"
            || lowered.hasPrefix("localhost:")
            || lowered.hasPrefix("127.")
            || lowered.hasPrefix("0.0.0.0")
            || lowered.hasPrefix("[::1]")
            || lowered.hasPrefix("::1")
    }

    private static func isLoopback(_ host: String) -> Bool {
        host == "localhost"
            || host == "::1"
            || host == "0.0.0.0"
            || host.hasPrefix("127.")
    }
}

final class ScreenshotSafeTokenStore {
    static let service = "com.screenshotsafe.server-token"
    static let accessGroup = ScreenshotSafeSettingsStore.appGroupIdentifier

    func token(for credentialID: UUID) throws -> String? {
        var query = baseQuery(credentialID: credentialID)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne

        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound {
            return nil
        }
        guard status == errSecSuccess, let data = result as? Data, let token = String(data: data, encoding: .utf8) else {
            throw ScreenshotSafeConfigurationError.keychain(status)
        }
        return token
    }

    func save(_ token: String, credentialID: UUID) throws {
        let tokenData = Data(token.utf8)
        var query = baseQuery(credentialID: credentialID)
        let updateStatus = SecItemUpdate(
            query as CFDictionary,
            [kSecValueData as String: tokenData] as CFDictionary
        )
        if updateStatus == errSecSuccess {
            return
        }
        guard updateStatus == errSecItemNotFound else {
            throw ScreenshotSafeConfigurationError.keychain(updateStatus)
        }

        query[kSecValueData as String] = tokenData
        query[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
        let addStatus = SecItemAdd(query as CFDictionary, nil)
        guard addStatus == errSecSuccess else {
            throw ScreenshotSafeConfigurationError.keychain(addStatus)
        }
    }

    func delete(credentialID: UUID) throws {
        let status = SecItemDelete(baseQuery(credentialID: credentialID) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw ScreenshotSafeConfigurationError.keychain(status)
        }
    }

    private func baseQuery(credentialID: UUID) -> [String: Any] {
        var query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: Self.service,
            kSecAttrAccount as String: credentialID.uuidString,
            kSecAttrAccessGroup as String: Self.accessGroup,
            kSecAttrSynchronizable as String: false,
        ]
#if os(macOS)
        if #available(macOS 10.15, *) {
            query[kSecUseDataProtectionKeychain as String] = true
        }
#endif
        return query
    }
}

final class ScreenshotSafeSettingsStore {
    static let appGroupIdentifier = "group.com.screenshotsafe.safari"

    private enum Key {
        static let registry = "connectionRegistryV2"
        static let serverURL = "serverUrl"
        static let apiToken = "apiToken"
        static let defaultExpiry = "defaultExpiry"
    }

    private let defaults: UserDefaults
    private let tokenStore: ScreenshotSafeTokenStore
    let isUsingAppGroup: Bool

    init(tokenStore: ScreenshotSafeTokenStore = ScreenshotSafeTokenStore()) {
        if let appGroupDefaults = UserDefaults(suiteName: Self.appGroupIdentifier) {
            self.defaults = appGroupDefaults
            self.isUsingAppGroup = true
        } else {
            self.defaults = .standard
            self.isUsingAppGroup = false
        }
        self.tokenStore = tokenStore
    }

    func load() -> ScreenshotSafeSettings {
        let registry = loadRegistry()
        guard
            let connection = registry.defaultConnection,
            let token = try? tokenStore.token(for: connection.credentialID)
        else {
            return ScreenshotSafeSettings(
                serverURL: "",
                apiToken: "",
                defaultExpiry: registry.defaultExpiry
            )
        }
        return ScreenshotSafeSettings(
            serverURL: connection.origin,
            apiToken: token,
            defaultExpiry: registry.defaultExpiry
        )
    }

    func loadRegistry() -> ScreenshotSafeConnectionRegistry {
        guard
            let data = defaults.data(forKey: Key.registry),
            var registry = try? JSONDecoder().decode(ScreenshotSafeConnectionRegistry.self, from: data),
            registry.schemaVersion == ScreenshotSafeConnectionRegistry.currentSchemaVersion
        else {
            return ScreenshotSafeConnectionRegistry(
                defaultExpiry: defaults.string(forKey: Key.defaultExpiry) ?? ""
            )
        }

        var seenOrigins = Set<String>()
        registry.connections = registry.connections.filter { connection in
            guard
                let origin = try? ScreenshotSafeServerURLNormalizer.normalize(connection.origin),
                origin == connection.origin,
                seenOrigins.insert(origin).inserted
            else {
                return false
            }
            return true
        }
        if !registry.connections.contains(where: { $0.id == registry.defaultConnectionID }) {
            registry.defaultConnectionID = nil
        }
        return registry
    }

    func token(for connection: ScreenshotSafeConnection) throws -> String? {
        try tokenStore.token(for: connection.credentialID)
    }

    @discardableResult
    func upsertVerifiedConnection(
        origin: String,
        token: String,
        displayName: String,
        username: String,
        makeDefault: Bool = true
    ) throws -> (connection: ScreenshotSafeConnection, superseded: ScreenshotSafeSupersededCredential?) {
        guard isUsingAppGroup else {
            throw ScreenshotSafeConfigurationError.appGroupUnavailable
        }
        let normalizedOrigin = try ScreenshotSafeServerURLNormalizer.normalize(origin)
        var registry = loadRegistry()

        if
            let existingIndex = registry.connections.firstIndex(where: { $0.origin == normalizedOrigin }),
            let currentToken = try tokenStore.token(for: registry.connections[existingIndex].credentialID),
            currentToken == token
        {
            registry.connections[existingIndex].displayName = displayName
            registry.connections[existingIndex].username = username
            registry.connections[existingIndex].lastStatus = .connected
            registry.connections[existingIndex].lastCheckedAt = Date()
            if makeDefault {
                registry.defaultConnectionID = registry.connections[existingIndex].id
            }
            try saveRegistry(registry)
            return (registry.connections[existingIndex], nil)
        }

        let newCredentialID = UUID()
        try tokenStore.save(token, credentialID: newCredentialID)

        let existingIndex = registry.connections.firstIndex { $0.origin == normalizedOrigin }
        let oldCredential: ScreenshotSafeSupersededCredential?
        let connection: ScreenshotSafeConnection
        if let existingIndex = existingIndex {
            let oldConnection = registry.connections[existingIndex]
            oldCredential = try tokenStore.token(for: oldConnection.credentialID).map {
                ScreenshotSafeSupersededCredential(
                    credentialID: oldConnection.credentialID,
                    token: $0
                )
            }
            connection = ScreenshotSafeConnection(
                id: oldConnection.id,
                credentialID: newCredentialID,
                origin: normalizedOrigin,
                displayName: displayName,
                username: username,
                lastStatus: .connected,
                lastCheckedAt: Date()
            )
            registry.connections[existingIndex] = connection
        } else {
            oldCredential = nil
            connection = ScreenshotSafeConnection(
                id: UUID(),
                credentialID: newCredentialID,
                origin: normalizedOrigin,
                displayName: displayName,
                username: username,
                lastStatus: .connected,
                lastCheckedAt: Date()
            )
            registry.connections.append(connection)
        }

        if makeDefault || registry.defaultConnectionID == nil {
            registry.defaultConnectionID = connection.id
        }

        do {
            try saveRegistry(registry)
        } catch {
            try? tokenStore.delete(credentialID: newCredentialID)
            throw error
        }
        return (connection, oldCredential)
    }

    func setDefaultConnection(id: UUID) throws {
        var registry = loadRegistry()
        guard registry.connections.contains(where: { $0.id == id }) else {
            throw ScreenshotSafeConfigurationError.connectionNotFound
        }
        registry.defaultConnectionID = id
        try saveRegistry(registry)
    }

    func updateConnection(
        id: UUID,
        status: ScreenshotSafeConnectionStatus,
        displayName: String? = nil,
        username: String? = nil
    ) throws {
        var registry = loadRegistry()
        guard let index = registry.connections.firstIndex(where: { $0.id == id }) else {
            throw ScreenshotSafeConfigurationError.connectionNotFound
        }
        registry.connections[index].lastStatus = status
        registry.connections[index].lastCheckedAt = Date()
        if let displayName = displayName, !displayName.isEmpty {
            registry.connections[index].displayName = displayName
        }
        if let username = username, !username.isEmpty {
            registry.connections[index].username = username
        }
        try saveRegistry(registry)
    }

    @discardableResult
    func removeConnection(id: UUID) throws -> ScreenshotSafeConnection {
        var registry = loadRegistry()
        guard let index = registry.connections.firstIndex(where: { $0.id == id }) else {
            throw ScreenshotSafeConfigurationError.connectionNotFound
        }
        let removed = registry.connections.remove(at: index)
        if registry.defaultConnectionID == id {
            registry.defaultConnectionID = registry.connections.first(where: {
                $0.lastStatus == .connected
            })?.id ?? registry.connections.first?.id
        }
        try saveRegistry(registry)
        try tokenStore.delete(credentialID: removed.credentialID)
        return removed
    }

    func deleteCredential(_ credential: ScreenshotSafeSupersededCredential) {
        try? tokenStore.delete(credentialID: credential.credentialID)
    }

    func setDefaultExpiry(_ value: String) throws {
        var registry = loadRegistry()
        registry.defaultExpiry = value
        try saveRegistry(registry)
        defaults.set(value, forKey: Key.defaultExpiry)
        postChange()
    }

    func parseQRCode(_ value: String) throws -> (origin: String, token: String) {
        guard let data = value.data(using: .utf8), data.count <= 4096 else {
            throw ScreenshotSafeConfigurationError.invalidQRCode
        }
        let payload: ScreenshotSafeQRConfiguration
        do {
            payload = try JSONDecoder().decode(ScreenshotSafeQRConfiguration.self, from: data)
        } catch {
            throw ScreenshotSafeConfigurationError.invalidQRCode
        }
        guard payload.type == "screenshotsafe_configuration", payload.version == 1 else {
            throw ScreenshotSafeConfigurationError.unsupportedQRCode
        }
        let token = payload.token.trimmingCharacters(in: .whitespacesAndNewlines)
        guard
            (8...512).contains(token.count),
            token.hasPrefix("sss_"),
            token.allSatisfy({ $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "_") })
        else {
            throw ScreenshotSafeConfigurationError.invalidToken
        }
        return (try ScreenshotSafeServerURLNormalizer.normalize(payload.serverURL), token)
    }

    func legacyConfiguration() -> ScreenshotSafeLegacyConfiguration? {
        guard
            let serverURL = defaults.string(forKey: Key.serverURL),
            let token = defaults.string(forKey: Key.apiToken),
            !serverURL.isEmpty,
            !token.isEmpty
        else {
            return nil
        }
        return ScreenshotSafeLegacyConfiguration(serverURL: serverURL, token: token)
    }

    func clearLegacyConfiguration() {
        defaults.removeObject(forKey: Key.serverURL)
        defaults.removeObject(forKey: Key.apiToken)
    }

    private func saveRegistry(_ registry: ScreenshotSafeConnectionRegistry) throws {
        do {
            let data = try JSONEncoder().encode(registry)
            defaults.set(data, forKey: Key.registry)
            postChange()
        } catch {
            throw ScreenshotSafeConfigurationError.storage(error)
        }
    }

    private func postChange() {
        let notify = {
            NotificationCenter.default.post(
                name: .screenshotSafeSettingsDidChange,
                object: self
            )
        }
        if Thread.isMainThread {
            notify()
        } else {
            DispatchQueue.main.async(execute: notify)
        }
    }
}
