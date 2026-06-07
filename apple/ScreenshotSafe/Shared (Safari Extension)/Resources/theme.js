(function () {
    'use strict';

    const cookieName = 'theme_preference';
    const explicitThemes = new Set(['light', 'dark']);
    const ext = window.sssWebExt;

    init();

    async function init() {
        try {
            const settings = await ext.storage.get(['serverUrl']);
            if (!settings.serverUrl || !ext.cookies) return;

            const cookie = await ext.cookies.get({
                url: settings.serverUrl,
                name: cookieName,
            });
            applyTheme(cookie && cookie.value);
        } catch (_) {
            applyTheme(null);
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
