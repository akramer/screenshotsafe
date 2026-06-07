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

    async function requestOrigin(serverUrl) {
        if (!api || !api.permissions || typeof api.permissions.request !== 'function') {
            return true;
        }

        const origin = new URL(serverUrl).origin + '/*';
        return call(api.permissions, 'request', [{ origins: [origin] }]);
    }

    window.sssWebExt = {
        storage: {
            async get(keys) {
                if (api && api.storage && api.storage.local) {
                    return call(api.storage.local, 'get', [keys]);
                }

                return getLocal(keys);
            },
            async set(values) {
                if (api && api.storage && api.storage.local) {
                    await call(api.storage.local, 'set', [values]);
                    return;
                }

                setLocal(values);
            },
        },
        permissions: {
            requestOrigin,
        },
        cookies: {
            get(details) {
                return call(api && api.cookies, 'get', [details]);
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
