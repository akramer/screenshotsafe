/**
 * ScreenshotSafe Chromium connection registry.
 *
 * Credentials stay in chrome.storage.local. This file is shared by extension
 * pages and the Manifest V3 service worker.
 */
(function (root) {
    'use strict';

    const api = root.browser || root.chrome;
    const usesChromeCallbackApi = Boolean(root.chrome && api === root.chrome);
    const schemaVersion = 2;
    const storageKeys = ['connectionSchemaVersion', 'activeServerOrigin', 'servers', 'serverUrl'];

    function call(target, method, args) {
        if (!target || typeof target[method] !== 'function') {
            return Promise.reject(new Error(`Extension API unavailable: ${method}`));
        }

        if (!usesChromeCallbackApi) {
            try {
                const result = target[method](...args);
                if (result && typeof result.then === 'function') return result;
                if (result !== undefined) return Promise.resolve(result);
            } catch (_) {}
        }

        return new Promise((resolve, reject) => {
            target[method](...args, (result) => {
                const error = root.chrome && root.chrome.runtime && root.chrome.runtime.lastError;
                if (error) {
                    reject(new Error(error.message));
                    return;
                }
                resolve(result);
            });
        });
    }

    async function loadState() {
        if (!api || !api.storage || !api.storage.local) {
            return emptyState();
        }

        const stored = await call(api.storage.local, 'get', [storageKeys]);
        if (stored.connectionSchemaVersion === schemaVersion && Array.isArray(stored.servers)) {
            return sanitizeState(stored);
        }

        const state = emptyState();
        if (stored.serverUrl) {
            try {
                const origin = normalizeServerUrl(stored.serverUrl);
                state.servers.push({
                    origin,
                    token: '',
                    displayName: '',
                    username: '',
                    lastPingStatus: 'login_required',
                    lastPingAt: null,
                });
                state.activeServerOrigin = origin;
            } catch (_) {}
        }
        await saveState(state);
        if (typeof api.storage.local.remove === 'function') {
            await call(api.storage.local, 'remove', [['serverUrl']]).catch(() => {});
        }
        return state;
    }

    function emptyState() {
        return {
            connectionSchemaVersion: schemaVersion,
            activeServerOrigin: '',
            servers: [],
        };
    }

    function sanitizeState(stored) {
        const seen = new Set();
        const servers = [];
        for (const candidate of stored.servers) {
            if (!candidate || typeof candidate !== 'object') continue;
            try {
                const origin = normalizeServerUrl(candidate.origin);
                if (seen.has(origin)) continue;
                seen.add(origin);
                servers.push({
                    origin,
                    token: typeof candidate.token === 'string' ? candidate.token : '',
                    displayName: typeof candidate.displayName === 'string' ? candidate.displayName : '',
                    username: typeof candidate.username === 'string' ? candidate.username : '',
                    lastPingStatus: validStatus(candidate.lastPingStatus),
                    lastPingAt: typeof candidate.lastPingAt === 'string' ? candidate.lastPingAt : null,
                });
            } catch (_) {}
        }
        const active = servers.some((server) => server.origin === stored.activeServerOrigin)
            ? stored.activeServerOrigin
            : '';
        return {
            connectionSchemaVersion: schemaVersion,
            activeServerOrigin: active,
            servers,
        };
    }

    function validStatus(status) {
        return [
            'connected',
            'login_required',
            'account_disabled',
            'unreachable',
            'incompatible',
            'server_error',
            'checking',
        ].includes(status) ? status : 'unreachable';
    }

    async function saveState(state) {
        const clean = sanitizeState({
            connectionSchemaVersion: schemaVersion,
            activeServerOrigin: state.activeServerOrigin,
            servers: state.servers,
        });
        await call(api.storage.local, 'set', [{
            connectionSchemaVersion: schemaVersion,
            activeServerOrigin: clean.activeServerOrigin,
            servers: clean.servers,
        }]);
        return clean;
    }

    async function getActiveConnection() {
        const state = await loadState();
        return state.servers.find((server) => server.origin === state.activeServerOrigin) || null;
    }

    async function setActive(origin) {
        const state = await loadState();
        if (!state.servers.some((server) => server.origin === origin)) {
            throw new Error('That ScreenshotSafe server is not configured.');
        }
        state.activeServerOrigin = origin;
        return saveState(state);
    }

    async function upsertConnection(connection, makeActive) {
        const state = await loadState();
        const origin = normalizeServerUrl(connection.origin);
        const index = state.servers.findIndex((server) => server.origin === origin);
        const next = {
            origin,
            token: connection.token || '',
            displayName: connection.displayName || '',
            username: connection.username || '',
            lastPingStatus: validStatus(connection.lastPingStatus || 'unreachable'),
            lastPingAt: connection.lastPingAt || null,
        };
        if (index >= 0) state.servers[index] = next;
        else state.servers.push(next);
        if (makeActive || !state.activeServerOrigin) state.activeServerOrigin = origin;
        return saveState(state);
    }

    async function removeConnection(origin) {
        const state = await loadState();
        state.servers = state.servers.filter((server) => server.origin !== origin);
        if (state.activeServerOrigin === origin) {
            const connected = state.servers.find((server) => server.lastPingStatus === 'connected');
            state.activeServerOrigin = connected ? connected.origin : (state.servers[0]?.origin || '');
        }
        return saveState(state);
    }

    async function ping(connection) {
        const checkedAt = new Date().toISOString();
        if (!connection || !connection.token) {
            return { status: 'login_required', checkedAt };
        }
        try {
            const response = await fetch(`${connection.origin}/api/ping`, {
                cache: 'no-store',
                mode: 'cors',
                credentials: 'omit',
                headers: authorizationHeaders(connection),
            });
            if (response.ok) {
                const data = await response.json().catch(() => ({}));
                return {
                    status: 'connected',
                    checkedAt,
                    displayName: data.display_name || connection.displayName || '',
                    username: data.username || connection.username || '',
                    themePreference: data.theme_preference || null,
                    retention: data.retention || null,
                };
            }
            if (response.status === 401) return { status: 'login_required', checkedAt };
            if (response.status === 403) return { status: 'account_disabled', checkedAt };
            if (response.status === 404) return { status: 'incompatible', checkedAt };
            return { status: 'server_error', checkedAt };
        } catch (_) {
            return { status: 'unreachable', checkedAt };
        }
    }

    function authorizationHeaders(connection) {
        return connection && connection.token
            ? { Authorization: `Bearer ${connection.token}` }
            : {};
    }

    function normalizeServerUrl(value) {
        const trimmed = String(value || '').trim();
        if (!trimmed) throw new Error('Enter your ScreenshotSafe server domain.');
        const withScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(trimmed)
            ? trimmed
            : `${looksLocal(trimmed) ? 'http' : 'https'}://${trimmed}`;

        let url;
        try {
            url = new URL(withScheme);
        } catch (_) {
            throw new Error('Enter a valid domain, like screenshots.example.com.');
        }
        if (!['http:', 'https:'].includes(url.protocol)
            || url.username
            || url.password
            || (url.protocol === 'http:' && !isLoopbackHostname(url.hostname))) {
            throw new Error('Use HTTPS. Plain HTTP is allowed only for localhost development.');
        }
        return url.origin.replace(/\/+$/, '');
    }

    function looksLocal(value) {
        return /^(localhost|127\.|0\.0\.0\.0|\[::1\]|::1)(?::\d+)?(?:\/|$)/i.test(value);
    }

    function isLoopbackHostname(hostname) {
        return hostname === 'localhost'
            || hostname === '[::1]'
            || hostname === '::1'
            || hostname === '0.0.0.0'
            || hostname.startsWith('127.');
    }

    root.sssConnections = {
        loadState,
        saveState,
        getActiveConnection,
        setActive,
        upsertConnection,
        removeConnection,
        ping,
        authorizationHeaders,
        normalizeServerUrl,
    };
})(globalThis);
