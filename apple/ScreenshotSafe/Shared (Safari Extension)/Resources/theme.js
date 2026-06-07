(function () {
    'use strict';

    const cookieName = 'theme_preference';
    const cachedThemeKey = 'sss:themePreference';
    const explicitThemes = new Set(['light', 'dark']);
    const ext = window.sssWebExt;
    const deferAuthenticatedTheme = window.SCREENSHOTSAFE_THEME_DEFER_PING === true;

    if (!deferAuthenticatedTheme) applyCachedTheme();
    document.documentElement.dataset.themeLoading = 'true';
    window.sssTheme = {
        applyPreference: applyTheme,
    };
    init();

    async function init() {
        try {
            const settings = await ext.storage.get(['serverUrl']);
            if (!settings.serverUrl) {
                applyTheme(null);
                return;
            }

            if (deferAuthenticatedTheme) return;

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
            const resp = await fetch(`${serverUrl}/api/ping`, {
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
        } else {
            delete document.documentElement.dataset.theme;
        }

        cacheTheme(theme);
        revealPage();
    }

    function applyCachedTheme() {
        try {
            const theme = window.localStorage.getItem(cachedThemeKey);
            if (!explicitThemes.has(theme)) return;
            document.documentElement.dataset.theme = theme;
            setRootBackground(theme);
        } catch (_) {}
    }

    function cacheTheme(theme) {
        try {
            if (explicitThemes.has(theme)) {
                window.localStorage.setItem(cachedThemeKey, theme);
                setRootBackground(theme);
            } else {
                window.localStorage.removeItem(cachedThemeKey);
                setRootBackground(null);
            }
        } catch (_) {}
    }

    function setRootBackground(theme) {
        if (theme === 'dark') {
            document.documentElement.style.backgroundColor = '#0f0f13';
        } else if (theme === 'light') {
            document.documentElement.style.backgroundColor = '#f7f8fb';
        } else {
            document.documentElement.style.backgroundColor = '';
        }
    }

    function revealPage() {
        delete document.documentElement.dataset.themeLoading;
        document.documentElement.style.visibility = '';
    }
})();
