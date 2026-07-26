/**
 * ScreenshotSafe Safari Extension — native configuration status.
 */

(function () {
    'use strict';

    const checkBtn = document.getElementById('check-btn');
    const closeBtn = document.getElementById('close-btn');
    const status = document.getElementById('status');
    const notice = document.getElementById('notice');
    const ext = window.sssWebExt;

    checkBtn.addEventListener('click', checkConnection);
    closeBtn.addEventListener('click', () => window.close());

    init();

    async function init() {
        const reason = new URLSearchParams(window.location.search).get('reason');
        if (reason) {
            showNotice(reasonMessage(reason));
        }

        await checkConnection();
    }

    async function checkConnection() {
        checkBtn.disabled = true;
        checkBtn.textContent = 'Checking...';
        setStatus('Asking the ScreenshotSafe app to check the server...', null);

        try {
            const response = await ext.runtime.sendNativeMessage({
                type: 'sss-verify-native-settings',
            });

            if (response && response.ok) {
                const destination = response.serverUrl ? ` ${response.serverUrl}` : '';
                setStatus(`Connected to${destination}. Safari is ready to upload.`, true);
                return;
            }

            setStatus(response && response.error
                ? response.error
                : 'The ScreenshotSafe app could not verify its configuration.', false);
        } catch (err) {
            setStatus(err.message, false);
        } finally {
            checkBtn.disabled = false;
            checkBtn.textContent = 'Check Connection';
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
                return 'Open the ScreenshotSafe app and log in to a server or scan a setup QR code.';
            case 'capture-error':
                return 'Safari could not start the capture. Check the app configuration and try again.';
            default:
                return 'Connected servers are managed by the ScreenshotSafe app.';
        }
    }
})();
