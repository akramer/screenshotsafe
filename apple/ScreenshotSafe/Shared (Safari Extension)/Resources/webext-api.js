/**
 * Narrow Safari WebExtension API adapter used by extension pages.
 */
(function () {
    'use strict';

    const api = window.browser || window.chrome;
    const nativeApplicationId = 'application.id';
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

    window.sssWebExt = {
        runtime: {
            getURL(path) {
                return api && api.runtime && api.runtime.getURL(path);
            },
            sendMessage(message) {
                return call(api && api.runtime, 'sendMessage', [message]);
            },
            sendNativeMessage(message) {
                return call(api && api.runtime, 'sendNativeMessage', [
                    nativeApplicationId,
                    message,
                ]);
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
