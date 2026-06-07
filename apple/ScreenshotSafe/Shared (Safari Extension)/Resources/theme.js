(function () {
    'use strict';

    const cookieName = 'theme_preference';
    const explicitThemes = new Set(['light', 'dark']);
    const ext = window.sssWebExt;

    init();

    async function init() {
        try {
            const settings = await ext.storage.get(['serverUrl']);
            if (!settings.serverUrl) return;

            const cookieTheme = await getCookieTheme(settings.serverUrl);
            if (cookieTheme) {
                applyTheme(cookieTheme);
                return;
            }

            applyTheme(await getAuthenticatedTheme(settings.serverUrl));
        } catch (_) {
            applyTheme(null);
        }
    }

    async function getCookieTheme(serverUrl) {
        if (!ext.cookies) return null;

        try {
            const cookie = await ext.cookies.get({
                url: serverUrl,
                name: cookieName,
            });
            return cookie && cookie.value;
        } catch (_) {
            return null;
        }
    }

    async function getAuthenticatedTheme(serverUrl) {
        try {
            const resp = await fetch(`${serverUrl}/api/user/preferences`, {
                cache: 'no-store',
                mode: 'cors',
                credentials: 'include',
            });
            if (!resp.ok) return null;

            const data = await resp.json();
            return data && data.theme_preference;
        } catch (_) {
            return null;
        }
    }

    function applyTheme(theme) {
        if (explicitThemes.has(theme)) {
            document.documentElement.dataset.theme = theme;
            return;
        }

        delete document.documentElement.dataset.theme;
    }
})();
