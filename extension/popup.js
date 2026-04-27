/**
 * ScreenshotSafe Chrome Extension — Popup Logic
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
    const resultDiv = document.getElementById('result');
    const shareUrlInput = document.getElementById('share-url');
    const rawUrlInput = document.getElementById('raw-url');
    const copyShareBtn = document.getElementById('copy-share-btn');
    const copyRawBtn = document.getElementById('copy-raw-btn');
    const openEditorBtn = document.getElementById('open-editor-btn');
    const savedToast = document.getElementById('saved-toast');

    let currentResult = null;

    // Load saved settings
    chrome.storage.local.get(['serverUrl', 'apiToken'], (data) => {
        if (data.serverUrl) serverUrlInput.value = data.serverUrl;
        if (data.apiToken) apiTokenInput.value = data.apiToken;
        checkConnection();
    });

    // Save settings
    saveSettingsBtn.addEventListener('click', () => {
        const serverUrl = serverUrlInput.value.trim().replace(/\/+$/, '');
        const apiToken = apiTokenInput.value.trim();

        chrome.storage.local.set({ serverUrl, apiToken }, () => {
            savedToast.classList.add('show');
            setTimeout(() => savedToast.classList.remove('show'), 2000);
            checkConnection();
        });
    });

    // Check connection to server
    async function checkConnection() {
        const serverUrl = serverUrlInput.value.trim().replace(/\/+$/, '');
        const apiToken = apiTokenInput.value.trim();

        if (!serverUrl || !apiToken) {
            statusDot.classList.remove('connected');
            statusText.textContent = 'Not configured';
            captureBtn.disabled = true;
            return;
        }

        try {
            const resp = await fetch(`${serverUrl}/api/screenshots?page=1&per_page=1`, {
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
        } catch (e) {
            statusDot.classList.remove('connected');
            statusText.textContent = 'Cannot reach server';
            captureBtn.disabled = true;
        }
    }

    // Capture screenshot
    captureBtn.addEventListener('click', async () => {
        errorMsg.classList.remove('show');
        resultDiv.classList.remove('show');
        captureBtn.disabled = true;
        captureBtn.textContent = '📷 Capturing...';

        try {
            // Get the active tab
            const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });

            if (!tab || !tab.id) {
                throw new Error('No active tab found');
            }

            // Capture the visible viewport
            const dataUrl = await chrome.tabs.captureVisibleTab(tab.windowId, {
                format: 'png',
            });

            captureBtn.textContent = '📷 Uploading...';

            // Convert data URL to blob
            const response = await fetch(dataUrl);
            const blob = await response.blob();

            // Upload to server
            const serverUrl = serverUrlInput.value.trim().replace(/\/+$/, '');
            const apiToken = apiTokenInput.value.trim();

            const formData = new FormData();
            formData.append('image', blob, 'screenshot.png');
            formData.append('title', tab.title || 'Screenshot');
            formData.append('source_url', tab.url || '');

            const uploadResp = await fetch(`${serverUrl}/api/screenshots`, {
                method: 'POST',
                headers: { 'Authorization': `Bearer ${apiToken}` },
                body: formData,
            });

            if (!uploadResp.ok) {
                const errData = await uploadResp.json().catch(() => ({}));
                throw new Error(errData.error || `Upload failed (${uploadResp.status})`);
            }

            const result = await uploadResp.json();
            currentResult = result;

            // Show result
            shareUrlInput.value = result.share_url;
            rawUrlInput.value = result.raw_url;
            resultDiv.classList.add('show');

            // Auto-copy share URL
            await navigator.clipboard.writeText(result.share_url);
            captureBtn.textContent = '✅ Copied to clipboard!';

        } catch (err) {
            errorMsg.textContent = err.message;
            errorMsg.classList.add('show');
            captureBtn.textContent = '📷 Capture Screenshot';
        } finally {
            captureBtn.disabled = false;
            setTimeout(() => {
                captureBtn.textContent = '📷 Capture Screenshot';
            }, 3000);
        }
    });

    // Copy buttons
    copyShareBtn.addEventListener('click', () => {
        navigator.clipboard.writeText(shareUrlInput.value);
        copyShareBtn.textContent = '✓';
        setTimeout(() => { copyShareBtn.textContent = 'Copy'; }, 1500);
    });

    copyRawBtn.addEventListener('click', () => {
        navigator.clipboard.writeText(rawUrlInput.value);
        copyRawBtn.textContent = '✓';
        setTimeout(() => { copyRawBtn.textContent = 'Copy'; }, 1500);
    });

    // Open editor button
    openEditorBtn.addEventListener('click', () => {
        if (currentResult) {
            const serverUrl = serverUrlInput.value.trim().replace(/\/+$/, '');
            chrome.tabs.create({
                url: `${serverUrl}/screenshots/${currentResult.id}/edit`,
            });
        }
    });
})();
