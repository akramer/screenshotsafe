/**
 * Tiny WebExtension API adapter for Chrome callback APIs and Safari/Firefox
 * promise APIs. Keep this intentionally narrow: it only wraps the APIs used by
 * the ScreenshotSafe popup.
 */
(function () {
    'use strict';

    const api = window.browser || window.chrome;
    const localStoragePrefix = 'sss:';
    const usesChromeCallbackApi = Boolean(window.chrome && api === window.chrome);

    function getLastError() {
        return window.chrome && window.chrome.runtime && window.chrome.runtime.lastError;
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
                const err = getLastError();
                if (err) {
                    reject(new Error(err.message));
                    return;
                }
                resolve(result);
            });
        });
    }

    function getLocal(keys) {
        const result = {};
        keys.forEach((key) => {
            const value = window.localStorage.getItem(`${localStoragePrefix}${key}`);
            if (value !== null) {
                result[key] = value;
            }
        });
        return result;
    }

    function setLocal(values) {
        Object.entries(values).forEach(([key, value]) => {
            window.localStorage.setItem(`${localStoragePrefix}${key}`, value);
        });
    }

    function hasNativeSettingsBridge() {
        return Boolean(api && api.runtime && typeof api.runtime.sendMessage === 'function');
    }

    async function sendNativeMessage(message) {
        if (!api || !api.runtime || typeof api.runtime.sendMessage !== 'function') {
            return null;
        }

        try {
            const response = await call(api.runtime, 'sendMessage', [{
                type: 'sss-native-message',
                message,
            }]);
            if (response && response.ok) return response;
            if (response && response.error) return response;
        } catch (err) {
            return { ok: false, error: err.message };
        }

        return {
            ok: false,
            error: 'Native ScreenshotSafe settings are unavailable. Rebuild and re-enable the Safari extension.',
        };
    }

    async function getNativeSettings() {
        const response = await sendNativeMessage({ type: 'sss-get-native-settings' });
        if (!response || !response.ok) {
            throw new Error(response && response.error ? response.error : 'Native ScreenshotSafe settings are unavailable.');
        }
        if (!response.settings) return {};

        const settings = {};
        if (response.settings.serverUrl) settings.serverUrl = response.settings.serverUrl;
        if (response.settings.apiToken) settings.apiToken = response.settings.apiToken;
        if (response.settings.defaultExpiry) settings.defaultExpiry = response.settings.defaultExpiry;
        return settings;
    }

    async function setNativeSettings(values) {
        const response = await sendNativeMessage({
            type: 'sss-set-native-settings',
            settings: values,
        });
        if (!response || !response.ok) {
            throw new Error(response && response.error ? response.error : 'Native ScreenshotSafe settings are unavailable.');
        }
    }

    window.sssWebExt = {
        storage: {
            async get(keys) {
                return getNativeSettings();
            },
            async set(values) {
                if (hasNativeSettingsBridge()) {
                    await setNativeSettings(values);
                    return;
                }

                throw new Error('Native ScreenshotSafe settings are unavailable. Rebuild and re-enable the Safari extension.');
            },
        },
        runtime: {
            getURL(path) {
                return api && api.runtime && api.runtime.getURL(path);
            },
            sendMessage(message) {
                return call(api && api.runtime, 'sendMessage', [message]);
            },
            onMessage(handler) {
                if (!api || !api.runtime || !api.runtime.onMessage) return;
                api.runtime.onMessage.addListener(handler);
            },
        },
        tabs: {
            query(queryInfo) {
                return call(api && api.tabs, 'query', [queryInfo]);
            },
            captureVisibleTab(windowId, options) {
                return call(api && api.tabs, 'captureVisibleTab', [windowId, options]);
            },
            create(createProperties) {
                return call(api && api.tabs, 'create', [createProperties]);
            },
        },
    };
})();
