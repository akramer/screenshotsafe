(function () {
    'use strict';

    const themes = new Set(['light', 'dark', 'os_default']);
    const cachedThemeKey = 'sss:themePreference';
    const ext = window.sssWebExt;

    document.documentElement.dataset.themeLoading = 'true';
    applyTheme(readCachedTheme());
    init();

    async function init() {
        try {
            const settings = await ext.storage.get(['serverUrl']);
            if (!settings.serverUrl) {
                return;
            }

            const theme = await getAuthenticatedTheme(settings.serverUrl);
            if (theme) applyTheme(theme);
        } catch (_) {}
    }

    async function getAuthenticatedTheme(serverUrl) {
        try {
            const resp = await fetch(`${serverUrl}/api/ping`, {
                cache: 'no-store',
                mode: 'cors',
                credentials: 'include',
            });
            if (!resp.ok) return null;

            const data = await resp.json();
            return data && themes.has(data.theme_preference)
                ? data.theme_preference
                : null;
        } catch (_) {
            return null;
        }
    }

    function applyTheme(theme) {
        const resolvedTheme = themes.has(theme) ? theme : 'os_default';
        document.documentElement.dataset.theme = resolvedTheme;
        cacheTheme(resolvedTheme);
        revealPage();
    }

    function readCachedTheme() {
        try {
            const theme = window.localStorage.getItem(cachedThemeKey);
            return themes.has(theme) ? theme : 'os_default';
        } catch (_) {
            return 'os_default';
        }
    }

    function cacheTheme(theme) {
        try {
            window.localStorage.setItem(cachedThemeKey, theme);
        } catch (_) {}
    }

    function revealPage() {
        delete document.documentElement.dataset.themeLoading;
        document.documentElement.style.visibility = '';
    }
})();
