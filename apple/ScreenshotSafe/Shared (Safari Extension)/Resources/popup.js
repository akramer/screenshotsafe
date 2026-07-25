/**
 * ScreenshotSafe Extension — Popup Logic
 */

(function () {
    'use strict';

    const captureBtn = document.getElementById('capture-btn');
    const settingsBtn = document.getElementById('settings-btn');
    const statusDot = document.getElementById('status-dot');
    const statusText = document.getElementById('status-text');
    const errorMsg = document.getElementById('error-msg');

    const ext = window.sssWebExt;
    let openedSettings = false;

    init();

    captureBtn.addEventListener('click', captureAndOpenEditor);
    settingsBtn.addEventListener('click', () => openSettings('manual'));

    async function init() {
        try {
            await checkConnection();
        } catch (err) {
            markInvalid(err.message);
            openSettings('load-error');
        }
    }

    async function checkConnection() {
        try {
            const response = await ext.runtime.sendNativeMessage({
                type: 'sss-verify-native-settings',
            });

            if (response && response.ok) {
                statusDot.classList.add('connected');
                statusText.textContent = 'Connected';
                captureBtn.disabled = false;
                return;
            }

            markInvalid(response && response.error
                ? response.error
                : 'App configuration unavailable');
        } catch (err) {
            markInvalid(err.message);
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

    function markInvalid(message) {
        statusDot.classList.remove('connected');
        statusText.textContent = message;
        captureBtn.disabled = true;
    }

    async function openSettings(reason) {
        if (openedSettings && reason !== 'manual') return;
        openedSettings = true;
        await ext.tabs.create({
            url: ext.runtime.getURL(`options.html?reason=${encodeURIComponent(reason)}`),
        });
    }

    function makeDraftId() {
        if (crypto && crypto.randomUUID) {
            return crypto.randomUUID();
        }
        return `${Date.now()}-${Math.random().toString(16).slice(2)}`;
    }

    function hideError() {
        errorMsg.classList.remove('show');
    }

    function showError(message) {
        errorMsg.textContent = message;
        errorMsg.classList.add('show');
    }
})();
