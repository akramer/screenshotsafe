(function () {
    'use strict';

    const themes = new Set(['light', 'dark', 'os_default']);
    const cachedThemeKey = 'sss:themePreference';

    document.documentElement.dataset.themeLoading = 'true';
    applyTheme(readCachedTheme());

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
