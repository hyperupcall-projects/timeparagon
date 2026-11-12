// Temporary effect, loaded before this process maps a window. KWin keeps a
// layer surface's namespace from scripts, so the setting is written in here and
// the effect is reloaded when it changes.
(function () {
    const DURATION = __DURATION__;
    function applies(window) {
        if (window.pid !== __PID__) return false;
        // The flash overlay fills the screen and fades itself.
        const screen = window.screen.geometry;
        return window.width < screen.width || window.height < screen.height;
    }
    effects.windowAdded.connect(function (window) {
        if (!applies(window)) return;
        // Force also cancels Scale/Fade if their signal handler ran first.
        effect.grab(window, Effect.WindowAddedGrabRole, true);
        if (DURATION > 0) {
            window.timeParagonOpeningAnimation = animate({
                window: window,
                duration: DURATION,
                curve: QEasingCurve.Linear,
                type: Effect.Opacity,
                from: 0,
                to: 1
            });
        }
    });
    effects.windowClosed.connect(function (window) {
        if (!applies(window)) return;
        if (window.timeParagonOpeningAnimation) {
            cancel(window.timeParagonOpeningAnimation);
            delete window.timeParagonOpeningAnimation;
        }
        // Force also cancels Scale/Fade's close animation.
        effect.grab(window, Effect.WindowClosedGrabRole, true);
        if (DURATION > 0) {
            animate({
                window: window,
                duration: DURATION,
                curve: QEasingCurve.Linear,
                type: Effect.Opacity,
                from: 1,
                to: 0
            });
        }
    });
})();
