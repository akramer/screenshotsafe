/**
 * ScreenshotSafe Chrome Extension — Background Service Worker
 *
 * Handles keyboard shortcut capture and badge updates.
 */

// Listen for extension installation
chrome.runtime.onInstalled.addListener(() => {
    console.log('ScreenshotSafe extension installed');
});
