//
//  AppDelegate.swift
//  macOS (App)
//
//  Created by Adam Kramer on 5/17/26.
//

import Cocoa

@main
class AppDelegate: NSObject, NSApplicationDelegate {

    func applicationDidFinishLaunching(_ notification: Notification) {
        // Override point for customization after application launch.
    }

    func application(_ application: NSApplication, open urls: [URL]) {
        let settingsStore = ScreenshotSafeSettingsStore()
        urls.forEach { _ = settingsStore.saveConfiguration(from: $0) }
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        return true
    }

}
