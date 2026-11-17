// Touch events keep scrolling natural until the hold has armed. Pointer events
// cover mouse/pen; touch has its own path to avoid native pointercancel on pan.
window.__calendarGesture?.abort();
const controller = new AbortController(), signal = controller.signal;
let pending = null, timer = null, selected = null, drag = null, quietUntil = 0, raf = 0;
const grids = () => [...document.querySelectorAll('.grid[data-calendar-day]')];
const clearHold = () => { clearTimeout(timer); timer = null; pending = null; };
function deselect() {
    clearHold(); cancelAnimationFrame(raf); raf = 0; drag = null;
    if (!selected) return;
    selected.source.classList.remove('gesture-source');
    selected.preview.remove(); selected = null;
}
function select(source) {
    deselect();
    const preview = source.cloneNode(true);
    preview.removeAttribute('disabled');
    preview.removeAttribute('data-calendar-event');
    preview.classList.add('calendar-selection');
    preview.setAttribute('aria-label', 'Selected event. Drag to move, or drag the start and end handles to resize.');
    for (const [edge, label] of [['start', 'Adjust start'], ['end', 'Adjust end']]) {
        const handle = document.createElement('span');
        handle.className = 'calendar-resize ' + edge;
        handle.dataset.resize = edge;
        handle.setAttribute('aria-label', label);
        preview.append(handle);
    }
    const when = document.createElement('span');
    when.className = 'calendar-drag-time'; when.setAttribute('role', 'status');
    preview.append(when);
    const grid = source.closest('.grid');
    selected = { source, preview, grid, when, start: +source.dataset.startMin,
        end: +source.dataset.endMin,
        id: source.dataset.calendarEvent, calendar: source.dataset.eventCalendar,
        before_start: +source.dataset.beforeStart, before_end: +source.dataset.beforeEnd,
        duration: +source.dataset.duration,
        startDelta: 0, endDelta: 0, saving: false };
    source.classList.add('gesture-source'); grid.append(preview);
    render();
    navigator.vibrate?.(20);
}
const clock = minutes => {
    const total = ((minutes + +selected.grid.dataset.hourStart * 60) % 1440 + 1440) % 1440;
    const date = new Date(2000, 0, 1, Math.floor(total / 60), total % 60);
    return date.toLocaleTimeString([], {hour: 'numeric', minute: '2-digit', hour12: selected.grid.dataset.clock === '12'});
};
function render() {
    const s = selected;
    s.preview.style.top = (s.start + s.startDelta) / 1440 * s.grid.clientHeight + 'px';
    s.preview.style.height = Math.max(22, (s.end - s.start + s.endDelta - s.startDelta) / 1440 * s.grid.clientHeight) + 'px';
    s.when.classList[s.start + s.startDelta < 45 ? 'add' : 'remove']('below');
    s.when.textContent = clock(s.start + s.startDelta) + ' – ' + clock(s.end + s.endDelta);
}
function begin(target, x, y, id) {
    if (selected?.saving || document.querySelector('.modal-backdrop')) return;
    const handle = target.closest('[data-resize]');
    if (selected && selected.preview.contains(target)) {
        drag = { x, y, id, mode: handle?.dataset.resize || 'move', grid: selected.grid,
            startDelta: selected.startDelta, endDelta: selected.endDelta, scroll: selected.grid.getBoundingClientRect().top };
        quietUntil = Date.now() + 600;
        return;
    }
    deselect();
    const source = target.closest('[data-calendar-event][data-draggable="true"]');
    if (!source || source.disabled) return;
    pending = { source, x, y, id };
    timer = setTimeout(() => {
        const p = pending;
        if (!p?.source.isConnected) return clearHold();
        select(p.source);
        drag = { x: p.x, y: p.y, id: p.id, mode: 'move', grid: selected.grid,
            startDelta: 0, endDelta: 0, scroll: selected.grid.getBoundingClientRect().top };
        quietUntil = Date.now() + 600;
    }, 400);
}
function update(x, y, id) {
    if (selected && !selected.source.isConnected) return deselect();
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
        const shift = Math.max(-s.start, Math.min(1440 - s.end, delta));
        s.startDelta = shift; s.endDelta = shift;
        s.dayDelta = days * 1440;
        if (s.preview.parentNode !== target) target.append(s.preview);
    } else if (drag.mode === 'start') {
        s.startDelta = Math.max(-s.start, Math.min(s.duration + s.endDelta - 5, drag.startDelta + delta));
    } else {
        s.endDelta = Math.min(1440 - s.end, Math.max(5 - s.duration + s.startDelta, drag.endDelta + delta));
    }
    render();
    if (!raf) raf = requestAnimationFrame(autoScroll);
}
function autoScroll() {
    raf = 0;
    if (!drag || !selected || drag.lastY === undefined) return;
    const scroller = selected.grid.closest('.timeline-scroll'), r = scroller.getBoundingClientRect();
    const speed = drag.lastY < r.top + 44 ? -8 : drag.lastY > r.bottom - 44 ? 8 : 0;
    if (speed) {
        scroller.scrollTop += speed;
        update(drag.lastX, drag.lastY, drag.id);
    }
}
function finish(cancelled, id) {
    clearHold();
    if (!drag || drag.id !== id) return;
    quietUntil = Date.now() + 600;
    cancelAnimationFrame(raf); raf = 0; drag = null;
    if (cancelled) return deselect();
    const s = selected;
    const start_delta = s.startDelta + (s.dayDelta || 0), end_delta = s.endDelta + (s.dayDelta || 0);
    if (!start_delta && !end_delta) return; // Keep handles after a stationary hold.
    s.saving = true; s.preview.classList.add('saving'); s.when.textContent = 'Saving…';
    dioxus.send({ id: s.id, calendar: s.calendar, before_start: s.before_start, before_end: s.before_end, start_delta, end_delta });
}
document.addEventListener('touchstart', e => {
    if (e.touches.length !== 1) { deselect(); return; }
    const t = e.changedTouches[0]; begin(e.target, t.clientX, t.clientY, t.identifier);
}, {signal, passive: true});
document.addEventListener('touchmove', e => {
    const t = [...e.touches].find(t => t.identifier === (drag?.id ?? pending?.id));
    if (!t) return;
    if (drag) e.preventDefault();
    update(t.clientX, t.clientY, t.identifier);
}, {signal, passive: false});
for (const type of ['touchend', 'touchcancel']) document.addEventListener(type, e => {
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
    if (Date.now() < quietUntil || e.target.closest('.calendar-selection')) { e.preventDefault(); e.stopImmediatePropagation(); }
}, {signal, capture: true});
document.addEventListener('contextmenu', e => { if (pending || selected) e.preventDefault(); }, {signal});
document.addEventListener('keydown', e => { if (e.key === 'Escape') deselect(); }, {signal});
window.addEventListener('blur', deselect, {signal});
window.__calendarGesture = { saved: deselect, abort: () => { deselect(); controller.abort(); } };
await new Promise(() => {});
