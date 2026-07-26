/**
 * ScreenshotSafe Chromium extension connection settings.
 */
(function () {
    'use strict';

    const ext = window.sssWebExt;
    const connections = window.sssConnections;
    const serverUrlInput = document.getElementById('server-url');
    const loginBtn = document.getElementById('login-btn');
    const closeBtn = document.getElementById('close-btn');
    const refreshBtn = document.getElementById('refresh-btn');
    const serverList = document.getElementById('server-list');
    const emptyState = document.getElementById('empty-state');
    const status = document.getElementById('status');
    const notice = document.getElementById('notice');
    let refreshing = false;
    let openingReason = new URLSearchParams(window.location.search).get('reason');
    let statusCheckedThisPage = false;

    init();

    loginBtn.addEventListener('click', () => loginServer(serverUrlInput.value));
    closeBtn.addEventListener('click', () => window.close());
    refreshBtn.addEventListener('click', refreshAll);
    serverUrlInput.addEventListener('keydown', (event) => {
        if (event.key === 'Enter') loginServer(serverUrlInput.value);
    });
    window.setInterval(refreshAll, 60 * 1000);

    async function init() {
        try {
            await render();
            await refreshAll();
        } catch (err) {
            setStatus(err.message, false);
        }
    }

    async function render(stateOverride) {
        const state = stateOverride || await connections.loadState();
        serverList.replaceChildren();
        emptyState.hidden = state.servers.length > 0;
        refreshBtn.disabled = state.servers.length === 0;

        for (const server of state.servers) {
            const row = document.createElement('div');
            row.className = 'server-row';
            if (server.origin === state.activeServerOrigin) row.classList.add('active');

            const radio = document.createElement('input');
            radio.type = 'radio';
            radio.name = 'active-server';
            radio.checked = server.origin === state.activeServerOrigin;
            radio.setAttribute('aria-label', `Use ${server.origin} for uploads`);
            radio.addEventListener('change', async () => {
                await connections.setActive(server.origin);
                await render();
                setStatus(`Uploads will use ${server.origin}.`, true);
            });

            const details = document.createElement('div');
            details.className = 'server-details';
            const origin = document.createElement('strong');
            origin.textContent = server.origin;
            const account = document.createElement('span');
            account.className = 'server-account';
            account.textContent = server.displayName
                ? `${server.displayName}${server.username ? ` (${server.username})` : ''}`
                : 'Not logged in';
            details.append(origin, account);

            const connectionStatus = document.createElement('div');
            connectionStatus.className = `connection-status status-${server.lastPingStatus}`;
            const dot = document.createElement('span');
            dot.className = 'connection-dot';
            const statusLabel = document.createElement('span');
            statusLabel.textContent = statusText(server);
            connectionStatus.append(dot, statusLabel);

            const actions = document.createElement('div');
            actions.className = 'server-actions';
            const retry = smallButton('Retry', 'secondary');
            retry.addEventListener('click', () => refreshOne(server.origin));
            actions.append(retry);

            if (!server.token || server.lastPingStatus === 'login_required') {
                const reconnect = smallButton(server.token ? 'Log in again' : 'Log in', 'primary');
                reconnect.addEventListener('click', () => loginServer(server.origin));
                actions.append(reconnect);
            }

            const remove = smallButton(server.token ? 'Log out' : 'Remove', 'danger');
            remove.addEventListener('click', () => logoutServer(server));
            actions.append(remove);

            row.append(radio, details, connectionStatus, actions);
            serverList.append(row);
        }
        updateNotice(state);
    }

    function smallButton(label, kind) {
        const button = document.createElement('button');
        button.type = 'button';
        button.className = `small-button ${kind}`;
        button.textContent = label;
        return button;
    }

    async function loginServer(input) {
        let origin;
        let issuedConnection = null;
        let savedConnection = false;
        try {
            origin = connections.normalizeServerUrl(input);
        } catch (err) {
            setStatus(err.message, false);
            return;
        }

        loginBtn.disabled = true;
        loginBtn.textContent = 'Opening login…';
        setStatus(`Connecting to ${origin}…`, null);
        try {
            const granted = await ext.permissions.requestOrigin(origin);
            if (!granted) {
                throw new Error('Chrome needs permission for that server before ScreenshotSafe can connect.');
            }

            const state = randomBase64Url(32);
            const verifier = randomBase64Url(64);
            const challenge = await sha256Base64Url(verifier);
            const redirectUri = ext.identity.getRedirectURL('screenshotsafe');
            const authorizeUrl = new URL('/extension/authorize', origin);
            authorizeUrl.searchParams.set('redirect_uri', redirectUri);
            authorizeUrl.searchParams.set('state', state);
            authorizeUrl.searchParams.set('code_challenge', challenge);
            authorizeUrl.searchParams.set('code_challenge_method', 'S256');

            await verifyAuthorizationPage(authorizeUrl, origin);
            const finalUrl = await ext.identity.launchWebAuthFlow({
                url: authorizeUrl.toString(),
                interactive: true,
            });
            if (!finalUrl) throw new Error('ScreenshotSafe login did not complete.');

            const callback = new URL(finalUrl);
            if (callback.searchParams.get('state') !== state) {
                throw new Error('ScreenshotSafe login returned an invalid state.');
            }
            if (callback.searchParams.get('error')) {
                throw new Error('ScreenshotSafe login was cancelled.');
            }
            const code = callback.searchParams.get('code');
            if (!code) throw new Error('ScreenshotSafe login did not return an authorization code.');

            const exchange = await fetch(`${origin}/api/auth/extension/token`, {
                method: 'POST',
                mode: 'cors',
                credentials: 'omit',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({
                    code,
                    code_verifier: verifier,
                    redirect_uri: redirectUri,
                }),
            });
            const data = await exchange.json().catch(() => ({}));
            if (!exchange.ok || !data.token) {
                throw new Error(data.error || `Token exchange failed (${exchange.status}).`);
            }

            const existingState = await connections.loadState();
            const previous = existingState.servers.find((server) => server.origin === origin);
            const connection = {
                origin,
                token: data.token,
                displayName: data.display_name || '',
                username: data.username || '',
                lastPingStatus: 'checking',
                lastPingAt: null,
            };
            issuedConnection = connection;
            const ping = await connections.ping(connection);
            connection.lastPingStatus = ping.status;
            connection.lastPingAt = ping.checkedAt;
            connection.displayName = ping.displayName || connection.displayName;
            connection.username = ping.username || connection.username;
            if (ping.status !== 'connected') {
                throw new Error('The server issued a token, but its connection check failed.');
            }

            await connections.upsertConnection(connection, true);
            savedConnection = true;
            statusCheckedThisPage = true;
            if (previous && previous.token && previous.token !== data.token) {
                revokeToken(previous).catch(() => {});
            }
            serverUrlInput.value = '';
            await render();
            setStatus(`Connected to ${origin}. It is now used for uploads.`, true);
        } catch (err) {
            if (issuedConnection && !savedConnection) {
                await revokeToken(issuedConnection).catch(() => {});
            }
            const message = /identity|web auth flow/i.test(err.message)
                ? 'Chrome could not complete the ScreenshotSafe login flow.'
                : err.message;
            setStatus(message, false);
        } finally {
            loginBtn.disabled = false;
            loginBtn.textContent = 'Log in';
        }
    }

    async function verifyAuthorizationPage(authorizeUrl, origin) {
        let response;
        try {
            response = await fetch(authorizeUrl.toString(), {
                cache: 'no-store',
                mode: 'cors',
                credentials: 'omit',
            });
        } catch (_) {
            throw new Error(
                `Could not load ${origin}/extension/authorize. Open the server in Chrome and check its HTTPS certificate and availability.`
            );
        }

        if (response.ok) return;
        if (response.status === 404) {
            throw new Error(
                'This ScreenshotSafe server does not have the extension authorization endpoint. Update and restart the server, then try again.'
            );
        }

        const data = await response.json().catch(() => ({}));
        throw new Error(
            data.error || `The ScreenshotSafe authorization endpoint returned ${response.status}.`
        );
    }

    async function refreshAll() {
        if (refreshing) return;
        refreshing = true;
        try {
            const state = await connections.loadState();
            if (state.servers.length === 0) return;
            state.servers.forEach((server) => { server.lastPingStatus = 'checking'; });
            await render(state);
            const results = await Promise.all(state.servers.map(async (server) => ({
                origin: server.origin,
                result: await connections.ping(server),
            })));
            for (const { origin, result } of results) {
                const server = state.servers.find((entry) => entry.origin === origin);
                if (!server) continue;
                server.lastPingStatus = result.status;
                server.lastPingAt = result.checkedAt;
                if (result.displayName) server.displayName = result.displayName;
                if (result.username) server.username = result.username;
            }
            await connections.saveState(state);
            statusCheckedThisPage = true;
            await render(state);
        } catch (err) {
            setStatus(err.message, false);
        } finally {
            refreshing = false;
        }
    }

    async function refreshOne(origin) {
        const state = await connections.loadState();
        const server = state.servers.find((entry) => entry.origin === origin);
        if (!server) return;
        server.lastPingStatus = 'checking';
        await render(state);
        const result = await connections.ping(server);
        server.lastPingStatus = result.status;
        server.lastPingAt = result.checkedAt;
        if (result.displayName) server.displayName = result.displayName;
        if (result.username) server.username = result.username;
        await connections.saveState(state);
        statusCheckedThisPage = true;
        await render(state);
    }

    async function logoutServer(server) {
        if (server.token) {
            const revoked = await revokeToken(server);
            if (!revoked && !window.confirm(
                'The server could not revoke this token. Forget it locally anyway? You should revoke it later from the server settings.'
            )) return;
        }
        await connections.removeConnection(server.origin);
        await ext.permissions.removeOrigin(server.origin).catch(() => {});
        await render();
        setStatus(`Removed ${server.origin}.`, true);
    }

    async function revokeToken(server) {
        try {
            const response = await fetch(`${server.origin}/api/auth/extension/token`, {
                method: 'DELETE',
                mode: 'cors',
                credentials: 'omit',
                headers: connections.authorizationHeaders(server),
            });
            return response.ok || response.status === 401;
        } catch (_) {
            return false;
        }
    }

    function randomBase64Url(byteLength) {
        const bytes = crypto.getRandomValues(new Uint8Array(byteLength));
        return bytesToBase64Url(bytes);
    }

    async function sha256Base64Url(value) {
        const bytes = new TextEncoder().encode(value);
        const digest = await crypto.subtle.digest('SHA-256', bytes);
        return bytesToBase64Url(new Uint8Array(digest));
    }

    function bytesToBase64Url(bytes) {
        let binary = '';
        bytes.forEach((byte) => { binary += String.fromCharCode(byte); });
        return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
    }

    function statusText(server) {
        const checked = server.lastPingAt
            ? ` · ${new Date(server.lastPingAt).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}`
            : '';
        const labels = {
            connected: 'Connected',
            login_required: server.token ? 'Login expired or revoked' : 'Login required',
            account_disabled: 'Account disabled',
            unreachable: 'Server unreachable',
            incompatible: 'Server needs an upgrade',
            server_error: 'Server error',
            checking: 'Checking…',
        };
        return `${labels[server.lastPingStatus] || 'Not checked'}${checked}`;
    }

    function showNotice(message) {
        notice.textContent = message;
        notice.classList.add('show');
    }

    function updateNotice(state) {
        if (!openingReason || openingReason === 'manual') {
            hideNotice(false);
            return;
        }

        const active = state.servers.find(
            (server) => server.origin === state.activeServerOrigin
        );
        let unresolved = false;
        switch (openingReason) {
            case 'missing':
                unresolved = !active || !active.token;
                break;
            case 'login-required':
                unresolved = !active
                    || !active.token
                    || !statusCheckedThisPage
                    || ['login_required', 'checking'].includes(active.lastPingStatus);
                break;
            case 'cannot-reach-server':
                unresolved = !active
                    || !statusCheckedThisPage
                    || ['unreachable', 'checking'].includes(active.lastPingStatus);
                break;
            case 'server-error':
                unresolved = !active
                    || !statusCheckedThisPage
                    || ['server_error', 'checking'].includes(active.lastPingStatus);
                break;
            default:
                unresolved = false;
        }

        if (unresolved) {
            showNotice(reasonMessage(openingReason));
        } else {
            hideNotice(true);
        }
    }

    function hideNotice(clearReason) {
        notice.classList.remove('show');
        notice.textContent = '';
        if (!clearReason || !openingReason) return;
        openingReason = null;
        window.history.replaceState(null, '', window.location.pathname);
    }

    function setStatus(message, ok) {
        status.textContent = message;
        status.classList.toggle('ok', ok === true);
        status.classList.toggle('bad', ok === false);
    }

    function reasonMessage(reason) {
        switch (reason) {
            case 'missing':
                return 'Log in to a ScreenshotSafe server to start capturing.';
            case 'login-required':
                return 'The selected server needs you to log in again.';
            case 'cannot-reach-server':
                return 'The extension could not reach the selected server.';
            case 'server-error':
                return 'The selected server responded with an error.';
            default:
                return 'Check your ScreenshotSafe connections.';
        }
    }
})();
