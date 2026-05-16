/**
 * ScreenshotSafe Extension — Popup Logic
 */

(function () {
    'use strict';

    const serverUrlInput = document.getElementById('server-url');
    const apiTokenInput = document.getElementById('api-token');
    const saveSettingsBtn = document.getElementById('save-settings-btn');
    const captureBtn = document.getElementById('capture-btn');
    const statusDot = document.getElementById('status-dot');
    const statusText = document.getElementById('status-text');
    const errorMsg = document.getElementById('error-msg');
    const savedToast = document.getElementById('saved-toast');

    const ext = window.sssWebExt;

    ext.storage.get(['serverUrl', 'apiToken']).then((data) => {
        if (data.serverUrl) serverUrlInput.value = data.serverUrl;
        if (data.apiToken) apiTokenInput.value = data.apiToken;
        checkConnection();
    }).catch((err) => {
        statusDot.classList.remove('connected');
        statusText.textContent = err.message;
        captureBtn.disabled = true;
    });

    saveSettingsBtn.addEventListener('click', () => {
        const serverUrl = normalizedServerUrl();
        const apiToken = apiTokenInput.value.trim();

        ext.storage.set({ serverUrl, apiToken }).then(() => {
            savedToast.classList.add('show');
            setTimeout(() => savedToast.classList.remove('show'), 2000);
            checkConnection();
        }).catch((err) => showError(err.message));
    });

    captureBtn.addEventListener('click', captureAndOpenEditor);

    async function checkConnection() {
        const serverUrl = normalizedServerUrl();
        const apiToken = apiTokenInput.value.trim();

        if (!serverUrl || !apiToken) {
            statusDot.classList.remove('connected');
            statusText.textContent = 'Not configured';
            captureBtn.disabled = true;
            return;
        }

        try {
            const resp = await fetch(`${serverUrl}/api/ping`, {
                headers: { 'Authorization': `Bearer ${apiToken}` },
            });

            if (resp.ok) {
                statusDot.classList.add('connected');
                statusText.textContent = 'Connected';
                captureBtn.disabled = false;
            } else if (resp.status === 401) {
                statusDot.classList.remove('connected');
                statusText.textContent = 'Invalid token';
                captureBtn.disabled = true;
            } else {
                statusDot.classList.remove('connected');
                statusText.textContent = 'Server error';
                captureBtn.disabled = true;
            }
        } catch (_) {
            statusDot.classList.remove('connected');
            statusText.textContent = 'Cannot reach server';
            captureBtn.disabled = true;
        }
    }

    async function captureAndOpenEditor() {
        hideError();
        captureBtn.disabled = true;
        captureBtn.textContent = '📷 Capturing...';

        try {
            const [tab] = await ext.tabs.query({ active: true, currentWindow: true });
            if (!tab || !tab.id) {
                throw new Error('No active tab found');
            }

            const dataUrl = await ext.tabs.captureVisibleTab(tab.windowId, { format: 'png' });
            const id = makeDraftId();

            await ext.runtime.sendMessage({
                type: 'sss-save-draft',
                id,
                draft: {
                    dataUrl,
                    title: tab.title || 'Screenshot',
                    sourceUrl: tab.url || '',
                },
            });

            captureBtn.textContent = 'Opening editor...';
            await ext.tabs.create({
                url: ext.runtime.getURL(`editor.html?id=${encodeURIComponent(id)}`),
            });
        } catch (err) {
            showError(err.message);
            captureBtn.disabled = false;
            captureBtn.textContent = '📷 Capture and Edit';
        }
    }

    function makeDraftId() {
        if (crypto && crypto.randomUUID) {
            return crypto.randomUUID();
        }
        return `${Date.now()}-${Math.random().toString(16).slice(2)}`;
    }

    function normalizedServerUrl() {
        return serverUrlInput.value.trim().replace(/\/+$/, '');
    }

    function hideError() {
        errorMsg.classList.remove('show');
    }

    function showError(message) {
        errorMsg.textContent = message;
        errorMsg.classList.add('show');
    }
})();
