/**
 * ScreenshotSafe Extension — Options Page
 */

(function () {
    'use strict';

    const serverUrlInput = document.getElementById('server-url');
    const saveBtn = document.getElementById('save-btn');
    const closeBtn = document.getElementById('close-btn');
    const status = document.getElementById('status');
    const notice = document.getElementById('notice');

    const ext = window.sssWebExt;

    init();

    saveBtn.addEventListener('click', saveAndVerify);
    closeBtn.addEventListener('click', () => window.close());

    async function init() {
        const reason = new URLSearchParams(window.location.search).get('reason');
        if (reason && reason !== 'manual') {
            showNotice(reasonMessage(reason));
        }

        try {
            const settings = await ext.storage.get(['serverUrl']);
            if (settings.serverUrl) serverUrlInput.value = settings.serverUrl;
        } catch (err) {
            setStatus(err.message, false);
        }
    }

    async function saveAndVerify() {
        let serverUrl;
        try {
            serverUrl = normalizeServerUrl(serverUrlInput.value);
        } catch (err) {
            setStatus(err.message, false);
            return;
        }

        if (!serverUrl) {
            setStatus('Enter your ScreenshotSafe server domain.', false);
            return;
        }

        saveBtn.disabled = true;
        saveBtn.textContent = 'Checking...';
        setStatus('Checking connection...', null);

        try {
            const hasOriginAccess = await ext.permissions.requestOrigin(serverUrl);
            if (!hasOriginAccess) {
                setStatus('Chrome needs permission for that server before ScreenshotSafe can connect.', false);
                return;
            }

            await ext.storage.set({ serverUrl });
            serverUrlInput.value = serverUrl;
            const result = await verifySettings(serverUrl);
            setStatus(result.message, result.ok);
        } catch (err) {
            setStatus(err.message, false);
        } finally {
            saveBtn.disabled = false;
            saveBtn.textContent = 'Save and Check';
        }
    }

    async function verifySettings(serverUrl) {
        try {
            const resp = await fetch(`${serverUrl}/api/ping`, {
                cache: 'no-store',
                mode: 'cors',
                credentials: 'include',
            });

            if (resp.ok) {
                return { ok: true, message: 'Connected. You can capture screenshots now.' };
            }

            if (resp.status === 401) {
                return { ok: false, message: 'Saved. Sign in to ScreenshotSafe in your browser, then try again.' };
            }

            return { ok: false, message: `Server returned ${resp.status}.` };
        } catch (_) {
            return { ok: false, message: 'Could not reach the ScreenshotSafe server.' };
        }
    }

    function showNotice(message) {
        notice.textContent = message;
        notice.classList.add('show');
    }

    function setStatus(message, ok) {
        status.textContent = message;
        status.classList.toggle('ok', ok === true);
        status.classList.toggle('bad', ok === false);
    }

    function normalizeServerUrl(value) {
        const trimmed = value.trim();
        if (!trimmed) return '';

        const withScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(trimmed)
            ? trimmed
            : `${looksLocal(trimmed) ? 'http' : 'https'}://${trimmed}`;

        let url;
        try {
            url = new URL(withScheme);
        } catch (_) {
            throw new Error('Enter a valid domain, like screenshots.example.com.');
        }

        if (!/^https?:$/.test(url.protocol)) {
            throw new Error('Use an http or https ScreenshotSafe server address.');
        }

        return url.origin.replace(/\/+$/, '');
    }

    function looksLocal(value) {
        return /^(localhost|127\.|0\.0\.0\.0|\[::1\]|::1)(?::\d+)?(?:\/|$)/i.test(value);
    }

    function reasonMessage(reason) {
        switch (reason) {
            case 'missing':
                return 'Add your ScreenshotSafe server domain to start capturing.';
            case 'login-required':
                return 'Sign in to ScreenshotSafe in your browser, then try your capture again.';
            case 'cannot-reach-server':
                return 'The extension could not reach the saved server URL.';
            case 'server-error':
                return 'The saved server responded with an error.';
            default:
                return 'Check your ScreenshotSafe settings.';
        }
    }
})();
