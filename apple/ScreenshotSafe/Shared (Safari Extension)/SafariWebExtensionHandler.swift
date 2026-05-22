//
//  SafariWebExtensionHandler.swift
//  Shared (Safari Extension)
//
//  Created by Adam Kramer on 5/17/26.
//

import SafariServices
import os.log

@objc(SafariWebExtensionHandler)
class SafariWebExtensionHandler: NSObject, NSExtensionRequestHandling {

    func beginRequest(with context: NSExtensionContext) {
        let request = context.inputItems.first as? NSExtensionItem

        let profile: UUID?
        if #available(iOS 17.0, macOS 14.0, *) {
            profile = request?.userInfo?[SFExtensionProfileKey] as? UUID
        } else {
            profile = request?.userInfo?["profile"] as? UUID
        }

        let message: Any?
        if #available(iOS 15.0, macOS 11.0, *) {
            message = request?.userInfo?[SFExtensionMessageKey]
        } else {
            message = request?.userInfo?["message"]
        }

        os_log(.default, "Received message from browser.runtime.sendNativeMessage: %@ (profile: %@)", String(describing: message), profile?.uuidString ?? "none")

        let response = NSExtensionItem()
        let responseMessage = handle(message: message)
        if #available(iOS 15.0, macOS 11.0, *) {
            response.userInfo = [ SFExtensionMessageKey: responseMessage ]
        } else {
            response.userInfo = [ "message": responseMessage ]
        }

        context.completeRequest(returningItems: [ response ], completionHandler: nil)
    }

    private func handle(message: Any?) -> [String: Any] {
        guard let message = message as? [String: Any],
              let type = message["type"] as? String else {
            return ["ok": false, "error": "Invalid native message"]
        }

        let settingsStore = ScreenshotSafeSettingsStore()

        switch type {
        case "sss-get-native-settings":
            guard settingsStore.isUsingAppGroup else {
                return [
                    "ok": false,
                    "error": "App Group \(ScreenshotSafeSettingsStore.appGroupIdentifier) is unavailable. Check Signing & Capabilities for the macOS app and Safari extension.",
                ]
            }

            let settings = settingsStore.load()
            return [
                "ok": true,
                "settings": [
                    "serverUrl": settings.serverURL,
                    "apiToken": settings.apiToken,
                    "defaultExpiry": settings.defaultExpiry,
                ],
            ]

        case "sss-set-native-settings":
            guard settingsStore.isUsingAppGroup else {
                return [
                    "ok": false,
                    "error": "App Group \(ScreenshotSafeSettingsStore.appGroupIdentifier) is unavailable. Check Signing & Capabilities for the macOS app and Safari extension.",
                ]
            }

            guard let values = message["settings"] as? [String: Any] else {
                return ["ok": false, "error": "Missing settings"]
            }
            let current = settingsStore.load()
            settingsStore.save(ScreenshotSafeSettings(
                serverURL: values["serverUrl"] as? String ?? current.serverURL,
                apiToken: values["apiToken"] as? String ?? current.apiToken,
                defaultExpiry: values["defaultExpiry"] as? String ?? current.defaultExpiry
            ))
            return ["ok": true]

        default:
            return ["ok": false, "error": "Unknown native message type"]
        }
    }

}
