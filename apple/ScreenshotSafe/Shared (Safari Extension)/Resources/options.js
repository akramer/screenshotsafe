/**
 * ScreenshotSafe Safari Extension - Options Page
 */

(function () {
    'use strict';

    const serverUrlInput = document.getElementById('server-url');
    const apiTokenInput = document.getElementById('api-token');
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
            const settings = await ext.storage.get(['serverUrl', 'apiToken']);
            if (settings.serverUrl) serverUrlInput.value = settings.serverUrl;
            if (settings.apiToken) apiTokenInput.value = settings.apiToken;
        } catch (err) {
            setStatus(err.message, false);
        }
    }

    async function saveAndVerify() {
        const serverUrl = serverUrlInput.value.trim().replace(/\/+$/, '');
        const apiToken = apiTokenInput.value.trim();

        if (!serverUrl || !apiToken) {
            setStatus('Enter both a server URL and an API token.', false);
            return;
        }

        saveBtn.disabled = true;
        saveBtn.textContent = 'Verifying...';
        setStatus('Checking connection...', null);

        try {
            await ext.storage.set({ serverUrl, apiToken });
            const result = await verifySettings(serverUrl, apiToken);
            setStatus(result.message, result.ok);
        } catch (err) {
            setStatus(err.message, false);
        } finally {
            saveBtn.disabled = false;
            saveBtn.textContent = 'Save and Verify';
        }
    }

    async function verifySettings(serverUrl, apiToken) {
        try {
            const resp = await fetch(`${serverUrl}/api/ping`, {
                headers: { 'Authorization': `Bearer ${apiToken}` },
            });

            if (resp.ok) {
                return { ok: true, message: 'Connected. You can capture screenshots now.' };
            }

            if (resp.status === 401) {
                return { ok: false, message: 'The API token was rejected.' };
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

    function reasonMessage(reason) {
        switch (reason) {
            case 'missing':
                return 'Add your ScreenshotSafe server URL and API token to start capturing.';
            case 'invalid-token':
                return 'The saved API token was rejected. Check or replace it here.';
            case 'cannot-reach-server':
                return 'The extension could not reach the saved server URL.';
            case 'server-error':
                return 'The saved server responded with an error.';
            default:
                return 'Check your ScreenshotSafe settings.';
        }
    }
})();
