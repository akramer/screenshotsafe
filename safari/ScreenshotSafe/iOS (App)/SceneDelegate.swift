//
//  SceneDelegate.swift
//  iOS (App)
//
//  Created by Adam Kramer on 5/17/26.
//

import UIKit

class SceneDelegate: UIResponder, UIWindowSceneDelegate {

    var window: UIWindow?

    func scene(_ scene: UIScene, willConnectTo session: UISceneSession, options connectionOptions: UIScene.ConnectionOptions) {
        connectionOptions.urlContexts.forEach { _ = ScreenshotSafeSettingsStore().saveConfiguration(from: $0.url) }
        guard let _ = (scene as? UIWindowScene) else { return }
    }

    func scene(_ scene: UIScene, openURLContexts URLContexts: Set<UIOpenURLContext>) {
        URLContexts.forEach { _ = ScreenshotSafeSettingsStore().saveConfiguration(from: $0.url) }
    }

}
