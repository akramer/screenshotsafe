/**
 * ScreenshotSafe Safari Extension - Background Worker
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
const nativeApplicationId = 'application.id';

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

        if (message.type === 'sss-native-message') {
            sendNativeMessageToApp(message.message)
                .then(sendResponse)
                .catch((err) => sendResponse({ ok: false, error: err.message }));
            return true;
        }

        if (message.type === 'sss-login-required') {
            handleLoginRequired(message.settings || {}, message.reason || 'login-required', _sender.tab)
                .then(() => sendResponse({ ok: true }))
                .catch((err) => sendResponse({ ok: false, error: err.message }));
            return true;
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
    const activeTab = tab || (await queryActiveTab());
    const validation = await validateSettings(settings);
    if (!validation.ok) {
        if (validation.reason === 'missing') {
            await openSettings(validation.reason);
        } else {
            await handleLoginRequired(settings, validation.reason, activeTab);
        }
        return;
    }

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
        viewportWidth: activeTab.width || null,
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
    if (!settings.serverUrl) {
        return { ok: false, reason: 'missing' };
    }

    try {
        const resp = await fetch(`${settings.serverUrl}/api/ping`, {
            cache: 'no-store',
            mode: 'cors',
            credentials: 'include',
        });

        if (resp.ok) {
            return { ok: true };
        }

        if (resp.status === 401) {
            return { ok: false, reason: 'login-required' };
        }

        return { ok: false, reason: 'server-error' };
    } catch (_) {
        return { ok: false, reason: 'cannot-reach-server' };
    }
}

async function getSettings() {
    if (!api || !api.storage || !api.storage.local) {
        return { serverUrl: '' };
    }

    try {
        const settings = await call(api.storage.local, 'get', [['serverUrl']]);
        return { serverUrl: settings.serverUrl || '' };
    } catch (err) {
        console.error('ScreenshotSafe settings load failed:', err.message);
        return { serverUrl: '' };
    }
}

async function sendNativeMessageToApp(message) {
    const response = await call(api.runtime, 'sendNativeMessage', [nativeApplicationId, message]);
    if (!response || response.ok !== false) {
        return response || { ok: false, error: 'Native ScreenshotSafe settings are unavailable.' };
    }
    return response;
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

async function handleLoginRequired(settings, reason, tab) {
    if (settings && settings.serverUrl) {
        await showLoginRequiredDialog(reason, tab);
        await call(api.tabs, 'create', [{ url: settings.serverUrl }]);
    } else {
        await openSettings('missing');
    }
}

async function showLoginRequiredDialog(reason, tab) {
    const targetTab = tab || (await queryActiveTab().catch(() => null));
    if (!targetTab || !targetTab.id) {
        return;
    }

    const message = loginRequiredMessage(reason);

    try {
        if (api.scripting && typeof api.scripting.executeScript === 'function') {
            await call(api.scripting, 'executeScript', [{
                target: { tabId: targetTab.id },
                func: (dialogMessage) => alert(dialogMessage),
                args: [message],
            }]);
            return;
        }

        if (api.tabs && typeof api.tabs.executeScript === 'function') {
            await call(api.tabs, 'executeScript', [
                targetTab.id,
                { code: `alert(${JSON.stringify(message)});` },
            ]);
        }
    } catch (err) {
        console.warn('ScreenshotSafe sign-in dialog failed:', err.message);
    }
}

function loginRequiredMessage(reason) {
    if (reason === 'server-error') {
        return 'ScreenshotSafe responded with an error. Check the site, then try your capture again.';
    }

    if (reason === 'cannot-reach-server') {
        return 'ScreenshotSafe could not be reached. Confirm the address and sign in if needed.';
    }

    return 'Please sign in to ScreenshotSafe in your browser, then try your capture again.';
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
