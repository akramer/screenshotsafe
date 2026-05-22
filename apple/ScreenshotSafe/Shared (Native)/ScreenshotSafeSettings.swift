import Foundation

struct ScreenshotSafeSettings {
    var serverURL: String
    var apiToken: String
    var defaultExpiry: String

    var isConfigured: Bool {
        !serverURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty &&
            !apiToken.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }
}

extension Notification.Name {
    static let screenshotSafeSettingsDidChange = Notification.Name("ScreenshotSafeSettingsDidChange")
}

final class ScreenshotSafeSettingsStore {
    static let appGroupIdentifier = "group.com.screenshotsafe.safari"

    private enum Key {
        static let serverURL = "serverUrl"
        static let apiToken = "apiToken"
        static let defaultExpiry = "defaultExpiry"
    }

    private let defaults: UserDefaults
    let isUsingAppGroup: Bool

    init() {
        if let appGroupDefaults = UserDefaults(suiteName: Self.appGroupIdentifier) {
            self.defaults = appGroupDefaults
            self.isUsingAppGroup = true
        } else {
            self.defaults = .standard
            self.isUsingAppGroup = false
        }
    }

    func load() -> ScreenshotSafeSettings {
        ScreenshotSafeSettings(
            serverURL: defaults.string(forKey: Key.serverURL) ?? "",
            apiToken: defaults.string(forKey: Key.apiToken) ?? "",
            defaultExpiry: defaults.string(forKey: Key.defaultExpiry) ?? ""
        )
    }

    func save(_ settings: ScreenshotSafeSettings) {
        defaults.set(settings.serverURL.trimmingCharacters(in: .whitespacesAndNewlines), forKey: Key.serverURL)
        defaults.set(settings.apiToken.trimmingCharacters(in: .whitespacesAndNewlines), forKey: Key.apiToken)
        defaults.set(settings.defaultExpiry, forKey: Key.defaultExpiry)
        defaults.synchronize()
        NotificationCenter.default.post(name: .screenshotSafeSettingsDidChange, object: self)
    }

    @discardableResult
    func saveConfiguration(from url: URL) -> Bool {
        guard url.scheme == "screenshotsafe" else {
            return false
        }

        let isConfigureURL = url.host == "configure" || url.path == "/configure"
        guard isConfigureURL, let components = URLComponents(url: url, resolvingAgainstBaseURL: false) else {
            return false
        }

        let values = Dictionary(uniqueKeysWithValues: components.queryItems?.compactMap { item -> (String, String)? in
            guard let value = item.value else { return nil }
            return (item.name, value)
        } ?? [])

        guard
            let serverURL = values["server_url"],
            let token = values["token"] ?? values["api_token"],
            !serverURL.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
            !token.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        else {
            return false
        }

        save(ScreenshotSafeSettings(
            serverURL: serverURL,
            apiToken: token,
            defaultExpiry: values["default_expiry"] ?? load().defaultExpiry
        ))
        return true
    }
}
