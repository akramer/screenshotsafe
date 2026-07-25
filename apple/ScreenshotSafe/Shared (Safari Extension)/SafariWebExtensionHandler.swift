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

        let messageType = (message as? [String: Any])?["type"] as? String ?? "invalid"
        os_log(.default, "Received Safari web extension message: %{public}@ (profile: %{public}@)", messageType, profile?.uuidString ?? "none")

        handle(message: message) { responseMessage in
            let response = NSExtensionItem()
            if #available(iOS 15.0, macOS 11.0, *) {
                response.userInfo = [ SFExtensionMessageKey: responseMessage ]
            } else {
                response.userInfo = [ "message": responseMessage ]
            }

            context.completeRequest(returningItems: [ response ], completionHandler: nil)
        }
    }

    private func handle(message: Any?, completion: @escaping ([String: Any]) -> Void) {
        guard let message = message as? [String: Any],
              let type = message["type"] as? String else {
            completion(["ok": false, "error": "Invalid native message"])
            return
        }

        let settingsStore = ScreenshotSafeSettingsStore()
        guard settingsStore.isUsingAppGroup else {
            completion([
                "ok": false,
                "error": "App Group \(ScreenshotSafeSettingsStore.appGroupIdentifier) is unavailable. Check Signing & Capabilities for the app and Safari extension.",
            ])
            return
        }

        switch type {
        case "sss-get-native-settings":
            let settings = settingsStore.load()
            completion([
                "ok": true,
                "settings": [
                    "configured": settings.isConfigured,
                    "serverUrl": settings.serverURL,
                    "defaultExpiry": settings.defaultExpiry,
                ],
            ])

        case "sss-verify-native-settings":
            let settings = settingsStore.load()
            ScreenshotSafeUploadClient().verify(settings: settings) { result in
                switch result {
                case .success:
                    completion([
                        "ok": true,
                        "serverUrl": settings.serverURL,
                    ])
                case .failure(let error):
                    completion([
                        "ok": false,
                        "error": error.localizedDescription,
                    ])
                }
            }

        case "sss-upload-screenshot":
            let settings = settingsStore.load()
            guard settings.isConfigured else {
                completion([
                    "ok": false,
                    "error": ScreenshotSafeUploadError.notConfigured.localizedDescription,
                ])
                return
            }

            guard
                let imageBase64 = message["imageBase64"] as? String,
                let imageData = decodeBase64Image(imageBase64),
                !imageData.isEmpty
            else {
                completion(["ok": false, "error": "The edited screenshot data is invalid."])
                return
            }

            var uploadSettings = settings
            if let expiresIn = message["expiresIn"] as? String {
                uploadSettings.defaultExpiry = expiresIn
            }

            let filename = message["filename"] as? String ?? "screenshot.png"
            let title = message["title"] as? String ?? "Screenshot"
            let sourceURL = (message["sourceUrl"] as? String).flatMap { value in
                value.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? nil : value
            }
            let imageDPI = (message["imageDpi"] as? NSNumber)?.doubleValue

            ScreenshotSafeUploadClient().upload(
                imageData: imageData,
                filename: filename,
                title: title,
                sourceURL: sourceURL,
                imageDPI: imageDPI,
                settings: uploadSettings
            ) { result in
                switch result {
                case .success(let upload):
                    completion([
                        "ok": true,
                        "result": [
                            "id": upload.id,
                            "shareId": upload.shareId,
                            "shareUrl": upload.shareURL.absoluteString,
                            "rawUrl": upload.rawURL.absoluteString,
                        ],
                    ])
                case .failure(let error):
                    completion([
                        "ok": false,
                        "error": error.localizedDescription,
                    ])
                }
            }

        default:
            completion(["ok": false, "error": "Unknown native message type"])
        }
    }

    private func decodeBase64Image(_ value: String) -> Data? {
        let encoded: String
        if let comma = value.firstIndex(of: ",") {
            encoded = String(value[value.index(after: comma)...])
        } else {
            encoded = value
        }
        return Data(base64Encoded: encoded, options: .ignoreUnknownCharacters)
    }
}
