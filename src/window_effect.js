// Temporary effect, loaded before the side panel maps a window. The app ID
// carries the settings with the Wayland map, avoiding asynchronous D-Bus races.
(function () {
    function duration(window) {
        if (window.pid !== __PID__) return null;
        const match = /(?:^| )time-slay-(instant|fade-([0-9]+))(?: |$)/.exec(window.windowClass);
        if (!match) return null; // Desktop default belongs to the desktop effect.
        return match[1] === "instant" ? 0 : Math.min(Number(match[2]), 5000);
    }
    effects.windowAdded.connect(function (window) {
        const ms = duration(window);
        if (ms === null) return;
        // Force also cancels Scale/Fade if their signal handler ran first.
        effect.grab(window, Effect.WindowAddedGrabRole, true);
        if (ms > 0) {
            window.timeSlayOpeningAnimation = animate({
                window: window,
                duration: ms,
                curve: QEasingCurve.Linear,
                type: Effect.Opacity,
                from: 0,
                to: 1
            });
        }
    });
    effects.windowClosed.connect(function (window) {
        const ms = duration(window);
        if (ms === null) return;
        if (window.timeSlayOpeningAnimation) {
            cancel(window.timeSlayOpeningAnimation);
            delete window.timeSlayOpeningAnimation;
        }
        // Force also cancels Scale/Fade's close animation.
        effect.grab(window, Effect.WindowClosedGrabRole, true);
        if (ms > 0) {
            animate({
                window: window,
                duration: ms,
                curve: QEasingCurve.Linear,
                type: Effect.Opacity,
                from: 1,
                to: 0
            });
        }
    });
})();
