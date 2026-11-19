// Touch events keep scrolling natural until a hold or pinch arms. Pointer events
// cover mouse/pen; touch has its own path to avoid native pointercancel on pan.
window.__calendarGesture?.abort();
const controller = new AbortController(), signal = controller.signal;
let pending = null, timer = null, selected = null, drag = null, quietUntil = 0, raf = 0;
let pinch = null, pinching = false;
let alarmDrag = null, alarmTap = null;
let zoom = +(document.documentElement.style.getPropertyValue('--calendar-zoom') || 1);
// Only the timeline's vertical geometry scales; text and controls stay readable.
const MIN_ZOOM = 0.5, MAX_ZOOM = 3;
// A new range starts on a quarter hour and lasts an hour until it is adjusted.
const DRAFT_SNAP = 15, DRAFT_MINUTES = 60;
function setZoom(value) {
    if (!Number.isFinite(value)) return;
    value = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, value));
    if (value === zoom) return;
    settle();
    const anchors = [...document.querySelectorAll('.timeline-scroll')].map(scroller => {
        const timeline = scroller.querySelector('.timeline');
        const rect = scroller.getBoundingClientRect();
        const center = (rect.top + rect.bottom) / 2;
        return timeline && { scroller, timeline, center, fraction: (center - timeline.getBoundingClientRect().top) / timeline.clientHeight };
    }).filter(Boolean);
    zoom = value;
    document.documentElement.style.setProperty('--calendar-zoom', zoom);
    if (selected) render();
    for (const {scroller, timeline, center, fraction} of anchors) {
        scroller.scrollTop += timeline.getBoundingClientRect().top + fraction * timeline.clientHeight - center;
    }
}
const grids = () => [...document.querySelectorAll('.grid[data-calendar-day]')];
const clearHold = () => { clearTimeout(timer); timer = null; pending = null; };
// `quiet` is for when Rust already knows the draft is gone or being replaced.
function deselect(quiet) {
    clearHold(); cancelAnimationFrame(raf); raf = 0; drag = null;
    if (!selected) return;
    const draft = selected.draft;
    selected.source?.classList.remove('gesture-source');
    selected.preview.remove(); selected = null;
    if (draft && !quiet) dioxus.send({cancel_range: true});
}
// Scrolling, zooming and leaving the app keep a draft; only its unfinished drag is dropped.
function settle() {
    if (!selected?.draft) return deselect();
    clearHold(); cancelAnimationFrame(raf); raf = 0;
    if (drag) { drag = null; revertDraft(); }
}
function revertDraft() {
    const s = selected;
    s.startDelta = 0; s.endDelta = 0;
    if (s.preview.parentNode !== s.grid) s.grid.append(s.preview);
    render(); publishDraft();
}
function publishDraft() {
    const s = selected, grid = s.preview.parentNode;
    const range = {range_day: grid.dataset.calendarDay, hour_start: +grid.dataset.hourStart,
        range_start: s.start + s.startDelta, range_end: s.end + s.endDelta};
    const sent = JSON.stringify(range);
    if (sent !== s.sent) { s.sent = sent; dioxus.send(range); }
}
// The confirmation bar shortens the grid, which can cover a range held near the bottom.
function revealDraft() {
    if (!selected?.draft || drag) return;
    const scroller = selected.grid.closest('.timeline-scroll');
    const view = scroller.getBoundingClientRect(), box = selected.preview.getBoundingClientRect();
    // Leave room for the handles and time label outside the range.
    const above = view.top + 44 - box.top, below = box.bottom + 44 - view.bottom;
    if (above > 0) scroller.scrollTop -= above;
    else if (below > 0) scroller.scrollTop += Math.min(below, -above);
}
function select(source) {
    deselect();
    const preview = source.cloneNode(true);
    preview.removeAttribute('disabled');
    preview.removeAttribute('data-calendar-event');
    source.classList.add('gesture-source');
    lift(preview, source.closest('.grid'), 'Selected event. Drag to move, or drag the start and end handles to resize.', {
        source, start: +source.dataset.startMin, end: +source.dataset.endMin,
        id: source.dataset.calendarEvent, calendar: source.dataset.eventCalendar,
        before_start: +source.dataset.beforeStart, before_end: +source.dataset.beforeEnd,
        duration: +source.dataset.duration });
}
function drawDraft(grid, y) {
    deselect(true);
    const held = (y - grid.getBoundingClientRect().top) / grid.clientHeight * 1440;
    const start = Math.max(0, Math.min(1440 - DRAFT_MINUTES, Math.floor(held / DRAFT_SNAP) * DRAFT_SNAP));
    const preview = document.createElement('div');
    preview.className = 'grid-event calendar-draft';
    lift(preview, grid, 'New time range. Drag to move, or drag the start and end handles to resize.',
        {draft: true, start, end: start + DRAFT_MINUTES});
    publishDraft();
}
function lift(preview, grid, description, fields) {
    preview.classList.add('calendar-selection');
    preview.setAttribute('aria-label', description);
    for (const [edge, label] of [['start', 'Adjust start'], ['end', 'Adjust end']]) {
        // A day boundary is not an event edge.
        if (edge === 'start' ? fields.start < 0 : fields.end > 1440) continue;
        const handle = document.createElement('span');
        handle.className = 'calendar-resize ' + edge;
        handle.dataset.resize = edge;
        handle.setAttribute('aria-label', label);
        preview.append(handle);
    }
    const when = document.createElement('span');
    when.className = 'calendar-drag-time'; when.setAttribute('role', 'status');
    preview.append(when);
    selected = { ...fields, preview, grid, when, startDelta: 0, endDelta: 0, saving: false };
    grid.append(preview);
    render();
    navigator.vibrate?.(20);
}
const clock = (minutes, grid = selected.grid) => {
    const total = ((minutes + +grid.dataset.hourStart * 60) % 1440 + 1440) % 1440;
    const date = new Date(2000, 0, 1, Math.floor(total / 60), total % 60);
    return date.toLocaleTimeString([], {hour: 'numeric', minute: '2-digit', hour12: grid.dataset.clock === '12'});
};
const placingAlarm = () => document.querySelector('.alarm-placement-bar');
function alarmAt(grid, y) {
    const minute = Math.max(0, Math.min(1439, Math.round((y - grid.getBoundingClientRect().top) / grid.clientHeight * 1440)));
    const needle = document.querySelector('.alarm-needle');
    if (needle?.parentNode === grid) {
        needle.style.top = minute / 1440 * 100 + '%';
        needle.dataset.minute = String(minute);
        needle.setAttribute('aria-valuenow', String(minute));
        needle.querySelector('.alarm-needle-time').textContent = clock(minute, grid);
    }
    dioxus.send({alarm_day: grid.dataset.calendarDay, hour_start: +grid.dataset.hourStart, minute});
}
function render() {
    const s = selected;
    const start = Math.max(0, s.start + s.startDelta), end = Math.min(1440, s.end + s.endDelta);
    s.preview.style.top = start / 1440 * s.grid.clientHeight + 'px';
    s.preview.style.height = Math.max(22, (end - start) / 1440 * s.grid.clientHeight) + 'px';
    s.when.classList[s.start + s.startDelta < 45 ? 'add' : 'remove']('below');
    s.when.textContent = clock(s.start + s.startDelta) + ' – ' + clock(s.end + s.endDelta);
}
function begin(target, x, y, id) {
    if (selected?.saving || document.querySelector('.modal-backdrop')) return;
    if (placingAlarm()) {
        deselect();
        const grid = target.closest('.grid[data-calendar-day]');
        if (!grid) return;
        const needle = target.closest('.alarm-needle');
        if (needle) {
            // Preserve where the finger grabbed the 44px target to avoid a jump.
            alarmDrag = {id, grid, offset: y - grid.getBoundingClientRect().top - +needle.dataset.minute / 1440 * grid.clientHeight};
        } else {
            alarmTap = {id, grid, x, y};
        }
        return;
    }
    const handle = target.closest('[data-resize]');
    if (selected && selected.preview.contains(target)) {
        drag = { x, y, id, mode: handle?.dataset.resize || 'move', grid: selected.grid,
            startDelta: selected.startDelta, endDelta: selected.endDelta, scroll: selected.grid.getBoundingClientRect().top };
        quietUntil = Date.now() + 600;
        return;
    }
    // A draft stays while the grid scrolls under it; a tap on the grid dismisses it.
    const draft = selected?.draft;
    if (!draft) deselect();
    const grid = target.closest('.grid[data-calendar-day]');
    const movable = target.closest('[data-calendar-event][data-draggable="true"]');
    const source = movable?.disabled ? null : movable;
    const empty = grid && !target.closest('.grid-event') && !target.closest('.alarm-tap');
    if (!source && !empty && !(draft && grid)) return;
    pending = { source, grid: empty ? grid : null, x, y, id, dismiss: draft && !!grid };
    timer = setTimeout(() => {
        const p = pending;
        if (!(p?.source || p?.grid)?.isConnected) return clearHold();
        if (p.source) select(p.source); else drawDraft(p.grid, p.y);
        drag = { x: p.x, y: p.y, id: p.id, mode: 'move', grid: selected.grid,
            startDelta: 0, endDelta: 0, scroll: selected.grid.getBoundingClientRect().top };
        quietUntil = Date.now() + 600;
    }, 400);
}
function update(x, y, id) {
    if (alarmDrag?.id === id) {
        if (!placingAlarm() || !alarmDrag.grid.isConnected) { alarmDrag = null; return; }
        alarmDrag.lastX = x; alarmDrag.lastY = y;
        // Stay within this needle's day; tap another column to select its day.
        alarmAt(alarmDrag.grid, y - alarmDrag.offset);
        if (!raf) raf = requestAnimationFrame(autoScroll);
        return;
    }
    if (alarmTap?.id === id && Math.hypot(x - alarmTap.x, y - alarmTap.y) > 8) alarmTap = null;
    if (selected && !(selected.source || selected.grid).isConnected) return deselect();
    if (!drag) {
        if (pending && Math.hypot(x - pending.x, y - pending.y) > 8) clearHold();
        return;
    }
    if (drag.id !== id || !selected) return;
    drag.lastX = x; drag.lastY = y;
    const s = selected;
    const delta = Math.round((y - drag.y + drag.scroll - drag.grid.getBoundingClientRect().top) / drag.grid.clientHeight * 1440 / 5) * 5;
    if (drag.mode === 'move') {
        const columns = grids();
        const target = columns.find(grid => { const r = grid.getBoundingClientRect(); return x >= r.left && x <= r.right; }) || s.grid;
        const days = columns.indexOf(target) - columns.indexOf(drag.grid);
        const shift = Math.max(-Math.max(0, s.start), Math.min(1440 - Math.min(1440, s.end), delta));
        s.startDelta = shift; s.endDelta = shift;
        s.dayDelta = days * 1440;
        if (s.preview.parentNode !== target) target.append(s.preview);
    } else if (drag.mode === 'start') {
        s.startDelta = Math.max(-Math.max(0, s.start), Math.min(s.end - s.start + s.endDelta - 5, drag.startDelta + delta));
    } else {
        s.endDelta = Math.min(1440 - Math.min(1440, s.end), Math.max(5 - (s.end - s.start) + s.startDelta, drag.endDelta + delta));
    }
    render();
    if (s.draft) publishDraft();
    if (!raf) raf = requestAnimationFrame(autoScroll);
}
function autoScroll() {
    raf = 0;
    const active = alarmDrag || (selected && drag);
    if (!active || active.lastY === undefined) return;
    const scroller = active.grid.closest('.timeline-scroll'), r = scroller.getBoundingClientRect();
    const speed = active.lastY < r.top + 44 ? -8 : active.lastY > r.bottom - 44 ? 8 : 0;
    if (speed) {
        scroller.scrollTop += speed;
        update(active.lastX, active.lastY, active.id);
    }
}
function finish(cancelled, id) {
    const dismissed = !cancelled && pending?.dismiss && pending.id === id;
    clearHold();
    if (dismissed) { quietUntil = Date.now() + 600; return deselect(); }
    if (alarmTap?.id === id) {
        if (!cancelled && placingAlarm()) alarmAt(alarmTap.grid, alarmTap.y);
        alarmTap = null;
        quietUntil = Date.now() + 600;
    }
    if (alarmDrag?.id === id) {
        alarmDrag = null; cancelAnimationFrame(raf); raf = 0;
        quietUntil = Date.now() + 600;
    }
    if (!drag || drag.id !== id) return;
    quietUntil = Date.now() + 600;
    cancelAnimationFrame(raf); raf = 0; drag = null;
    const s = selected;
    if (s?.draft) {
        if (cancelled) return revertDraft();
        // Later drags measure from here, as nothing is saved between them.
        s.start += s.startDelta; s.end += s.endDelta;
        s.startDelta = 0; s.endDelta = 0;
        s.grid = s.preview.parentNode;
        return revealDraft();
    }
    if (cancelled) return deselect();
    const start_delta = s.startDelta + (s.dayDelta || 0), end_delta = s.endDelta + (s.dayDelta || 0);
    if (!start_delta && !end_delta) return; // Keep handles after a stationary hold.
    s.saving = true; s.preview.classList.add('saving'); s.when.textContent = 'Saving…';
    dioxus.send({ id: s.id, calendar: s.calendar, before_start: s.before_start, before_end: s.before_end, start_delta, end_delta });
}
const distance = (a, b) => Math.hypot(a.clientX - b.clientX, a.clientY - b.clientY);
function startPinch(e) {
    if (e.touches.length !== 2 || selected?.saving || document.querySelector('.modal-backdrop')) return;
    const [a, b] = e.touches;
    const scroller = a.target.closest('.timeline-scroll');
    if (!scroller || b.target.closest('.timeline-scroll') !== scroller) return;
    const timeline = scroller.querySelector('.timeline');
    const span = distance(a, b);
    if (!timeline || span < 1) return;
    alarmDrag = null; alarmTap = null;
    settle();
    pinching = true;
    const midpoint = (a.clientY + b.clientY) / 2;
    pinch = { scroller, timeline, ids: [a.identifier, b.identifier], span, zoom,
        // A fraction of the day survives both scaling and midpoint movement.
        anchor: (midpoint - timeline.getBoundingClientRect().top) / timeline.clientHeight };
}
function movePinch(e) {
    if (!pinch || e.touches.length !== 2) return;
    const [a, b] = pinch.ids.map(id => [...e.touches].find(t => t.identifier === id));
    if (!a || !b || !pinch.timeline.isConnected) { pinch = null; return; }
    zoom = Math.max(MIN_ZOOM, Math.min(MAX_ZOOM, pinch.zoom * distance(a, b) / pinch.span));
    document.documentElement.style.setProperty('--calendar-zoom', zoom);
    if (selected) render();
    const {scroller, timeline, anchor} = pinch;
    const midpoint = (a.clientY + b.clientY) / 2;
    scroller.scrollTop += timeline.getBoundingClientRect().top + anchor * timeline.clientHeight - midpoint;
}
function finishPinch() {
    pinch = null;
    if (!pinching) return;
    pinching = false;
    quietUntil = Date.now() + 600;
    // Refit event labels once, keeping the native bridge out of animation frames.
    dioxus.send({zoom});
}
document.addEventListener('touchstart', e => {
    if (e.touches.length > 1) {
        alarmDrag = null; alarmTap = null;
        startPinch(e);
        if (pinching) { e.preventDefault(); pinch = e.touches.length === 2 ? pinch : null; }
        settle(); return;
    }
    if (pinching) { e.preventDefault(); return; }
    if (e.touches.length !== 1) { settle(); return; }
    const t = e.changedTouches[0]; begin(e.target, t.clientX, t.clientY, t.identifier);
}, {signal, passive: false});
document.addEventListener('touchmove', e => {
    if (pinching) { e.preventDefault(); movePinch(e); return; }
    const t = [...e.touches].find(t => t.identifier === (alarmDrag?.id ?? alarmTap?.id ?? drag?.id ?? pending?.id));
    if (!t) return;
    if (drag || alarmDrag) e.preventDefault();
    update(t.clientX, t.clientY, t.identifier);
}, {signal, passive: false});
for (const type of ['touchend', 'touchcancel']) document.addEventListener(type, e => {
    if (pinching) {
        pinch = null;
        if (!e.touches.length) finishPinch();
        return;
    }
    for (const t of e.changedTouches) finish(type === 'touchcancel', t.identifier);
}, {signal});
document.addEventListener('pointerdown', e => {
    if (e.pointerType !== 'touch' && e.button === 0) begin(e.target, e.clientX, e.clientY, e.pointerId);
}, {signal});
document.addEventListener('pointermove', e => { if (e.pointerType !== 'touch') update(e.clientX, e.clientY, e.pointerId); }, {signal});
for (const type of ['pointerup', 'pointercancel']) document.addEventListener(type, e => {
    if (e.pointerType !== 'touch') finish(type === 'pointercancel', e.pointerId);
}, {signal});
document.addEventListener('click', e => {
    // Confirm and Cancel must respond immediately, including just after a drag.
    if (e.target.closest('.placement-bar')) return;
    if (placingAlarm() && !document.querySelector('.modal-backdrop') && e.target.closest('.grid[data-calendar-day]')) {
        e.preventDefault(); e.stopImmediatePropagation(); return;
    }
    if (pinching || Date.now() < quietUntil || e.target.closest('.calendar-selection')) { e.preventDefault(); e.stopImmediatePropagation(); }
}, {signal, capture: true});
document.addEventListener('contextmenu', e => { if (pending || selected || alarmDrag) e.preventDefault(); }, {signal});
document.addEventListener('keydown', e => {
    if (document.querySelector('.modal-backdrop')) return;
    if (e.key === 'Escape') {
        deselect(); alarmDrag = null; alarmTap = null;
        if (placingAlarm()) dioxus.send({cancel_alarm: true});
    }
    const needle = e.target.closest('.alarm-needle');
    if (!needle || !placingAlarm()) return;
    const delta = {ArrowUp: -1, ArrowDown: 1, PageUp: -60, PageDown: 60}[e.key];
    if (delta === undefined) return;
    e.preventDefault();
    const grid = needle.closest('.grid[data-calendar-day]');
    alarmAt(grid, grid.getBoundingClientRect().top + (+needle.dataset.minute + delta) / 1440 * grid.clientHeight);
}, {signal});
window.addEventListener('blur', () => { finishPinch(); settle(); alarmDrag = null; alarmTap = null; }, {signal});
window.__calendarGesture = { setZoom, saved: () => deselect(), revealDraft,
    dismissDraft: () => { if (selected?.draft) deselect(true); },
    abort: () => { finishPinch(); deselect(true); alarmDrag = null; alarmTap = null; controller.abort(); } };
await new Promise(() => {});
