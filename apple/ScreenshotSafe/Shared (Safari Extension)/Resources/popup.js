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
    let settings = null;
    let openedSettings = false;

    init();

    captureBtn.addEventListener('click', captureAndOpenEditor);
    settingsBtn.addEventListener('click', () => openSettings('manual'));

    async function init() {
        try {
            settings = await ext.storage.get(['serverUrl']);
            await checkConnection();
        } catch (err) {
            markInvalid(err.message);
            openSettings('load-error');
        }
    }

    async function checkConnection() {
        if (!settings.serverUrl) {
            markInvalid('Settings required');
            openSettings('missing');
            return;
        }

        try {
            const resp = await fetch(`${settings.serverUrl}/api/ping`, {
                cache: 'no-store',
                mode: 'cors',
                credentials: 'include',
            });

            if (resp.ok) {
                statusDot.classList.add('connected');
                statusText.textContent = 'Connected';
                captureBtn.disabled = false;
                return;
            }

            if (resp.status === 401) {
                markInvalid('Sign-in needed');
                openLoginRequired('login-required');
                return;
            }

            markInvalid('Server error');
            openLoginRequired('server-error');
        } catch (_) {
            markInvalid('Cannot reach server');
            openLoginRequired('cannot-reach-server');
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

    async function openLoginRequired(reason) {
        if (openedSettings) return;
        openedSettings = true;
        await ext.runtime.sendMessage({
            type: 'sss-login-required',
            settings,
            reason,
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
