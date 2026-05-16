/**
 * ScreenshotSafe Extension — Background Worker
 *
 * Handles toolbar-click capture, settings menu access, and screenshot draft
 * handoff for the full-tab editor.
 */

const api = globalThis.browser || globalThis.chrome;
const usesChromeCallbackApi = Boolean(globalThis.chrome && api === globalThis.chrome);

const drafts = new Map();
const draftTtlMs = 10 * 60 * 1000;
const settingsMenuId = 'screenshotsafe-settings';
const delayedCapture3MenuId = 'screenshotsafe-capture-delay-3';
const delayedCapture10MenuId = 'screenshotsafe-capture-delay-10';

if (api && api.runtime) {
    api.runtime.onInstalled.addListener(() => {
        createSettingsMenu().catch(() => {});
        console.log('ScreenshotSafe extension installed');
    });

    if (api.runtime.onStartup) {
        api.runtime.onStartup.addListener(() => {
            createSettingsMenu().catch(() => {});
        });
    }

    api.runtime.onMessage.addListener((message, _sender, sendResponse) => {
        if (!message || typeof message !== 'object') {
            return false;
        }

        if (message.type === 'sss-save-draft') {
            saveDraft(message.id, message.draft);
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

if (api && api.action && api.action.onClicked) {
    api.action.onClicked.addListener((tab) => {
        captureAndOpenEditor(tab).catch((err) => {
            console.error('ScreenshotSafe capture failed:', err);
            openSettings('capture-error');
        });
    });
}

if (api && api.contextMenus && api.contextMenus.onClicked) {
    api.contextMenus.onClicked.addListener((info, tab) => {
        if (info.menuItemId === settingsMenuId) {
            openSettings('manual');
            return;
        }

        if (info.menuItemId === delayedCapture3MenuId) {
            captureAfterDelay(tab, 3000);
            return;
        }

        if (info.menuItemId === delayedCapture10MenuId) {
            captureAfterDelay(tab, 10000);
        }
    });
}

async function captureAndOpenEditor(tab) {
    const settings = await getSettings();
    const validation = await validateSettings(settings);
    if (!validation.ok) {
        await openSettings(validation.reason);
        return;
    }

    const activeTab = tab || (await queryActiveTab());
    if (!activeTab || !activeTab.id) {
        throw new Error('No active tab found');
    }

    const dataUrl = await call(api.tabs, 'captureVisibleTab', [
        activeTab.windowId,
        { format: 'png' },
    ]);
    const id = makeDraftId();

    saveDraft(id, {
        dataUrl,
        title: activeTab.title || 'Screenshot',
        sourceUrl: activeTab.url || '',
    });

    await call(api.tabs, 'create', [{
        url: api.runtime.getURL(`editor.html?id=${encodeURIComponent(id)}`),
    }]);
}

function captureAfterDelay(tab, delayMs) {
    setTimeout(() => {
        captureAndOpenEditor(tab).catch((err) => {
            console.error('ScreenshotSafe delayed capture failed:', err);
            openSettings('capture-error');
        });
    }, delayMs);
}

async function validateSettings(settings) {
    if (!settings.serverUrl || !settings.apiToken) {
        return { ok: false, reason: 'missing' };
    }

    try {
        const resp = await fetch(`${settings.serverUrl}/api/ping`, {
            headers: { 'Authorization': `Bearer ${settings.apiToken}` },
        });

        if (resp.ok) {
            return { ok: true };
        }

        if (resp.status === 401) {
            return { ok: false, reason: 'invalid-token' };
        }

        return { ok: false, reason: 'server-error' };
    } catch (_) {
        return { ok: false, reason: 'cannot-reach-server' };
    }
}

async function getSettings() {
    const values = await call(api.storage && api.storage.local, 'get', [['serverUrl', 'apiToken']]);
    return {
        serverUrl: values.serverUrl || '',
        apiToken: values.apiToken || '',
    };
}

async function queryActiveTab() {
    const tabs = await call(api.tabs, 'query', [{ active: true, currentWindow: true }]);
    return tabs && tabs[0];
}

async function openSettings(reason) {
    await call(api.tabs, 'create', [{
        url: api.runtime.getURL(`options.html?reason=${encodeURIComponent(reason)}`),
    }]);
}

async function createSettingsMenu() {
    if (!api || !api.contextMenus) return;

    if (api.contextMenus.removeAll) {
        await call(api.contextMenus, 'removeAll', []);
    }

    const create = async (contexts) => {
        try {
            await call(api.contextMenus, 'create', [{
                id: delayedCapture3MenuId,
                title: 'Take Screenshot in 3 Seconds',
                contexts,
            }]);
            await call(api.contextMenus, 'create', [{
                id: delayedCapture10MenuId,
                title: 'Take Screenshot in 10 Seconds',
                contexts,
            }]);
            await call(api.contextMenus, 'create', [{
                id: 'screenshotsafe-menu-separator',
                type: 'separator',
                contexts,
            }]);
            await call(api.contextMenus, 'create', [{
                id: settingsMenuId,
                title: 'ScreenshotSafe Settings',
                contexts,
            }]);
            return true;
        } catch (_) {
            if (api.contextMenus.removeAll) {
                await call(api.contextMenus, 'removeAll', []);
            }
            return false;
        }
    };

    if (await create(['action'])) return;
    await create(['browser_action']);
}

function saveDraft(id, draft) {
    drafts.set(id, {
        draft,
        expiresAt: Date.now() + draftTtlMs,
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

function call(target, method, args) {
    if (!target || typeof target[method] !== 'function') {
        return Promise.reject(new Error(`Extension API unavailable: ${method}`));
    }

    if (!usesChromeCallbackApi) {
        try {
            const result = target[method](...args);
            if (result && typeof result.then === 'function') {
                return result;
            }
            if (result !== undefined) {
                return Promise.resolve(result);
            }
        } catch (_) {
            // Some implementations require callbacks and throw without one.
        }
    }

    return new Promise((resolve, reject) => {
        target[method](...args, (result) => {
            const err = globalThis.chrome && globalThis.chrome.runtime &&
                globalThis.chrome.runtime.lastError;
            if (err) {
                reject(new Error(err.message));
                return;
            }
            resolve(result);
        });
    });
}

function makeDraftId() {
    if (globalThis.crypto && globalThis.crypto.randomUUID) {
        return globalThis.crypto.randomUUID();
    }
    return `${Date.now()}-${Math.random().toString(16).slice(2)}`;
}
