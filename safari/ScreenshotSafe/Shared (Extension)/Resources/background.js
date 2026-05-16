/**
 * ScreenshotSafe Extension — Background Worker
 *
 * Keeps captured screenshot drafts long enough for the full-tab editor to load.
 */

const extensionRuntime = (globalThis.browser && globalThis.browser.runtime) ||
    (globalThis.chrome && globalThis.chrome.runtime);

const drafts = new Map();
const draftTtlMs = 10 * 60 * 1000;

if (extensionRuntime) {
    extensionRuntime.onInstalled.addListener(() => {
        console.log('ScreenshotSafe extension installed');
    });

    extensionRuntime.onMessage.addListener((message, _sender, sendResponse) => {
        if (!message || typeof message !== 'object') {
            return false;
        }

        if (message.type === 'sss-save-draft') {
            drafts.set(message.id, {
                draft: message.draft,
                expiresAt: Date.now() + draftTtlMs,
            });
            sendResponse({ ok: true });
            return false;
        }

        if (message.type === 'sss-get-draft') {
            cleanupDrafts();
            const entry = drafts.get(message.id);
            if (!entry) {
                sendResponse({ ok: false, error: 'Screenshot draft expired. Capture again.' });
                return false;
            }
            drafts.delete(message.id);
            sendResponse({ ok: true, draft: entry.draft });
            return false;
        }

        return false;
    });
}

function cleanupDrafts() {
    const now = Date.now();
    drafts.forEach((entry, id) => {
        if (entry.expiresAt <= now) {
            drafts.delete(id);
        }
    });
}
