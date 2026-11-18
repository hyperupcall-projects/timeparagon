// Exercise the actual webview gesture controller without credentials or writes.
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import assert from 'node:assert/strict';
const script = await readFile(new URL('../dioxus/src/calendar_gestures.js', import.meta.url), 'utf8');
function setup() {
    const listeners = {}, timers = new Map(), writes = [], options = {};
    const properties = new Map();
    const rootStyle = {getPropertyValue: name => properties.get(name) || '', setProperty: (name, value) => properties.set(name, String(value))};
    let serial = 0;
    class Node {
        constructor(classes = '', dataset = {}) {
            this.classes = new Set(classes.split(' ')); this.dataset = {...dataset};
            this.style = {}; this.children = []; this.isConnected = true;
            this.classList = {add: c => this.classes.add(c), remove: c => this.classes.delete(c)};
        }
        set className(value) { this.classes = new Set(value.split(' ')); }
        setAttribute() {}
        removeAttribute(name) { if (name === 'disabled') this.disabled = false; if (name === 'data-calendar-event') delete this.dataset.calendarEvent; }
        cloneNode() { const n = new Node([...this.classes].join(' '), this.dataset); n.style = {...this.style}; return n; }
        append(n) { n.remove(); this.children.push(n); n.parentNode = this; }
        remove() { if (this.parentNode) this.parentNode.children = this.parentNode.children.filter(n => n !== this); this.parentNode = null; }
        contains(n) { return n === this || this.children.some(c => c.contains(n)); }
        querySelector(selector) {
            if (selector === '.timeline') return timeline;
            for (const child of this.children) {
                if (child.classes.has(selector.slice(1))) return child;
                const nested = child.querySelector(selector);
                if (nested) return nested;
            }
            return null;
        }
        get clientHeight() { return 1440 * +(properties.get('--calendar-zoom') || 1); }
        closest(selector) {
            const matches = selector === '[data-resize]' ? this.dataset.resize
                : selector === '.grid[data-calendar-day]' ? this.classes.has('grid') && this.dataset.calendarDay
                : selector === '[data-calendar-event][data-draggable="true"]' ? this.dataset.calendarEvent && this.dataset.draggable === 'true'
                : selector === '.grid-event, .block-task' ? this.classes.has('grid-event') || this.classes.has('block-task')
                : selector === '.placement-bar' ? this.classes.has('placement-bar')
                : selector.startsWith('.') && this.classes.has(selector.slice(1));
            return matches ? this : this.parentNode?.closest(selector) || null;
        }
        getBoundingClientRect() {
            if (this.classes.has('timeline-scroll')) return {top: 0, bottom: 800};
            if (this.classes.has('calendar-draft')) {
                const top = parseFloat(this.style.top) - scroller.scrollTop;
                return {top, bottom: top + parseFloat(this.style.height)};
            }
            return {top: -scroller.scrollTop, left: this === first ? 0 : 200, right: this === first ? 200 : 400};
        }
    }
    const scroller = new Node('timeline-scroll'); scroller.scrollTop = 0;
    const timeline = new Node('timeline'); scroller.append(timeline);
    const first = new Node('grid', {calendarDay: '2026-11-17', hourStart: '0', clock: '24'}), second = new Node('grid', {calendarDay: '2026-11-18', hourStart: '0', clock: '24'});
    timeline.append(first); timeline.append(second);
    const source = new Node('grid-event', {calendarEvent: 'event', eventCalendar: 'work', draggable: 'true', startMin: '540', endMin: '600', duration: '60', beforeStart: '1000', beforeEnd: '4600'});
    first.append(source);
    const doc = {documentElement: {style: rootStyle}, querySelector: selector => selector === '.modal-backdrop' ? (doc.modalOpen ? {} : null)
            : selector === '.alarm-placement-bar' ? doc.alarmBar || null : timeline.querySelector(selector),
        querySelectorAll: selector => selector === '.timeline-scroll' ? [scroller] : [first, second], createElement: () => new Node(),
        addEventListener: (type, fn, opts) => { listeners[type] = fn; options[type] = opts; }};
    const context = vm.createContext({ document: doc, window: {addEventListener() {}}, navigator: {vibrate() {}}, AbortController,
        setTimeout: fn => {timers.set(++serial, fn); return serial;}, clearTimeout: id => timers.delete(id),
        requestAnimationFrame: () => ++serial, cancelAnimationFrame() {},
        dioxus: {send: msg => writes.push(JSON.parse(JSON.stringify(msg)))}, Date });
    vm.runInContext(`(async () => { ${script}\n })()`, context);
    const down = (target = source, x = 100, y = 550) => listeners.pointerdown({target, clientX: x, clientY: y, pointerId: 1, pointerType: 'mouse', button: 0});
    const move = (x, y) => listeners.pointermove({clientX: x, clientY: y, pointerId: 1, pointerType: 'mouse'});
    const up = cancel => listeners[cancel ? 'pointercancel' : 'pointerup']({pointerId: 1, pointerType: 'mouse'});
    const hold = () => { for (const fn of [...timers.values()]) fn(); timers.clear(); };
    const preview = () => [...first.children, ...second.children].find(n => n.classes.has('calendar-selection'));
    const finger = (identifier, x, y, target = source) => ({identifier, clientX: x, clientY: y, target});
    const touch = (type, touches, changedTouches = touches) => {
        let prevented = false;
        listeners[type]({target: changedTouches[0]?.target || source, touches, changedTouches,
            preventDefault() {prevented = true;}});
        return prevented;
    };
    const zoom = () => +(properties.get('--calendar-zoom') || 1);
    const reload = () => vm.runInContext(`(async () => { ${script}\n })()`, context);
    const alarm = (minute = 600) => {
        doc.alarmBar = new Node('placement-bar alarm-placement-bar');
        const needle = new Node('alarm-needle', {minute: String(minute)});
        needle.append(new Node('alarm-needle-time')); first.append(needle);
        return needle;
    };
    return {source, first, second, scroller, timeline, doc, options, listeners, writes, context, down, move, up, hold, preview, finger, touch, zoom, reload, alarm};
}
{
    const s = setup();
    s.scroller.scrollTop = 480;
    s.context.window.__calendarGesture.setZoom(2);
    assert.equal(s.zoom(), 2, 'the View menu changes the same scale as pinch zoom');
    assert.ok(Math.abs(s.scroller.scrollTop - 1360) < 0.01, 'menu zoom keeps the time at the center in place');
    assert.deepEqual(s.writes, [], 'menu zoom never edits calendar data or echoes to Rust');
    const a = s.finger(7, 100, 200), b = s.finger(8, 100, 400);
    s.touch('touchstart', [a, b]);
    s.touch('touchmove', [s.finger(7, 100, 250), s.finger(8, 100, 350)]);
    assert.equal(s.zoom(), 1, 'pinching continues from the scale chosen in the menu');
    s.touch('touchend', [], [a, b]);
    const beforeReset = s.scroller.scrollTop;
    s.context.window.__calendarGesture.setZoom(3);
    s.context.window.__calendarGesture.setZoom(1);
    assert.equal(s.zoom(), 1, 'Reset zoom restores the default');
    assert.ok(Math.abs(s.scroller.scrollTop - beforeReset) < 0.01, 'zooming back restores the scroll position');
    s.context.window.__calendarGesture.setZoom(100);
    assert.equal(s.zoom(), 3, 'menu zoom respects the upper bound');
    s.context.window.__calendarGesture.setZoom(-1);
    assert.equal(s.zoom(), 0.5, 'menu zoom respects the lower bound');
}
{
    const s = setup(); s.down(); s.up(); s.hold();
    assert.equal(s.preview(), undefined, 'ordinary tap opens normally');
    assert.equal(s.writes.length, 0);
}
{
    const s = setup(); s.down(); s.move(100, 570); s.hold();
    assert.equal(s.preview(), undefined, 'moving before the hold keeps native scrolling');
}
{
    const s = setup(); s.down(); s.hold(); s.up();
    assert.ok(s.preview(), 'stationary hold leaves resize handles visible');
    assert.equal(s.writes.length, 0, 'selection alone never saves');
    let suppressed = false;
    s.listeners.click({target: s.source, preventDefault() {suppressed = true;}, stopImmediatePropagation() {}});
    assert.ok(suppressed, 'release does not open the editor');
    s.down(s.preview()); s.move(100, 582); s.up();
    assert.deepEqual(s.writes[0], {id: 'event', calendar: 'work', before_start: 1000, before_end: 4600, start_delta: 30, end_delta: 30}, 'moving snaps and preserves duration');
}
for (const edge of ['start', 'end']) {
    const s = setup(); s.down(); s.hold(); s.up();
    const handle = s.preview().children.find(n => n.dataset.resize === edge);
    s.down(handle); s.move(100, 580); s.up();
    assert.deepEqual(s.writes[0], {id: 'event', calendar: 'work', before_start: 1000, before_end: 4600, start_delta: edge === 'start' ? 30 : 0, end_delta: edge === 'end' ? 30 : 0});
}
for (const [start, end, delta, edge] of [[1380, 1500, -30, 'start'], [-60, 60, 30, 'end'], [-60, 1500, 0, null]]) {
    const s = setup();
    Object.assign(s.source.dataset, {startMin: String(start), endMin: String(end), duration: String(end - start)});
    s.down(); s.hold(); s.up();
    const preview = s.preview();
    assert.equal(preview.style.top, Math.max(0, start) + 'px', 'overnight preview clips only its drawing');
    assert.equal(preview.style.height, (Math.min(1440, end) - Math.max(0, start)) + 'px');
    assert.deepEqual(preview.children.filter(n => n.dataset.resize).map(n => n.dataset.resize), edge ? [edge] : [], 'only real event edges have resize handles');
    s.down(preview); s.move(100, 550 + delta); s.up();
    if (delta) {
        assert.equal(s.writes[0].start_delta, delta, 'moving keeps the hidden portion of an overnight event');
        assert.equal(s.writes[0].end_delta, delta);
    } else {
        assert.equal(s.writes.length, 0, 'holding an event longer than a day never moves it');
    }
}
for (const [start, end, edge, delta] of [[1380, 1500, 'start', -30], [-60, 60, 'end', 30]]) {
    const s = setup();
    Object.assign(s.source.dataset, {startMin: String(start), endMin: String(end), duration: String(end - start)});
    s.down(); s.hold(); s.up();
    s.down(s.preview().children.find(n => n.dataset.resize === edge)); s.move(100, 550 + delta); s.up();
    assert.equal(s.writes[0].start_delta, edge === 'start' ? delta : 0, 'resizing preserves the opposite overnight endpoint');
    assert.equal(s.writes[0].end_delta, edge === 'end' ? delta : 0);
}
{
    const s = setup(); s.down(); s.hold(); s.up();
    s.down(s.preview().children.find(n => n.dataset.resize === 'start'));
    s.move(100, 900); s.up();
    assert.equal(s.writes[0].start_delta, 55, 'resize cannot cross the other edge');
}
{
    const s = setup(); s.down(); s.hold(); s.move(300, 580); s.up();
    assert.equal(s.writes[0].start_delta, 1470, 'move across columns changes day and time');
    assert.equal(s.writes[0].end_delta, 1470);
}
{
    const s = setup(); s.down(); s.hold(); s.scroller.scrollTop = 60; s.move(100, 550); s.up();
    assert.equal(s.writes[0].start_delta, 60, 'scrolling contributes to the target time');
}
{
    const s = setup(); s.down(); s.hold(); s.move(100, 600); s.up(true);
    assert.equal(s.writes.length, 0, 'cancellation never persists a preview');
    assert.equal(s.preview(), undefined);
}
{
    const s = setup(); const event = (x, y) => ({target: s.source, touches: [{identifier: 7, clientX: x, clientY: y}], changedTouches: [{identifier: 7, clientX: x, clientY: y}]});
    s.listeners.touchstart(event(100, 550)); s.hold();
    let prevented = false;
    s.listeners.touchmove({...event(100, 575), preventDefault() {prevented = true;}});
    s.listeners.touchend(event(100, 575));
    assert.ok(prevented, 'armed touch gestures stop native scrolling');
    assert.equal(s.writes[0].start_delta, 25);
}
{
    const s = setup(); s.source.disabled = true; s.down(); s.hold();
    assert.equal(s.preview(), undefined, 'events with a write in flight stay locked');
}
{
    const s = setup(); s.down(); s.hold(); s.move(100, 650); s.up();
    s.context.window.__calendarGesture.saved();
    assert.equal(s.preview(), undefined, 'a save completion clears the lifted preview');
    assert.ok(!s.source.classes.has('gesture-source'), 'the saved event becomes fully visible');
}
{
    const s = setup(); s.down(); s.hold(); s.move(100, -500); s.up();
    assert.equal(s.writes[0].start_delta, -540, 'moving clamps to the day boundary');
    assert.equal(s.writes[0].end_delta, -540, 'boundary clamping preserves duration');
}
{
    const s = setup(), a = s.finger(7, 100, 200), b = s.finger(8, 100, 400);
    s.scroller.scrollTop = 480;
    assert.equal(s.options.touchstart.passive, false, 'two fingers can claim native zoom');
    assert.equal(s.touch('touchstart', [a]), false, 'one finger retains native scrolling');
    assert.equal(s.touch('touchstart', [a, b], [b]), true, 'two fingers claim the timeline');
    s.hold();
    assert.equal(s.preview(), undefined, 'a pinch cancels a pending long press');
    assert.equal(s.touch('touchmove', [s.finger(8, 100, 500), s.finger(7, 100, 100)]), true);
    assert.equal(s.zoom(), 2, 'spreading the same fingers doubles the timeline spacing');
    assert.equal(s.scroller.scrollTop, 1260, 'the time at the midpoint stays under the fingers');
    s.touch('touchmove', [s.finger(7, 100, 150), s.finger(8, 100, 550)]);
    assert.equal(s.scroller.scrollTop, 1210, 'moving the midpoint pans the same anchored time');
    assert.deepEqual(s.writes, [], 'animation stays in the webview');
    s.touch('touchend', [a], [b]);
    s.touch('touchmove', [s.finger(7, 100, 600)]);
    s.hold();
    assert.equal(s.preview(), undefined, 'the remaining finger cannot turn a pinch into an edit');
    assert.equal(s.zoom(), 2);
    s.touch('touchend', [], [a]);
    assert.deepEqual(s.writes, [{zoom: 2}], 'finishing refits labels without writing calendar data');
    let suppressed = false;
    s.listeners.click({target: s.source, preventDefault() {suppressed = true;}, stopImmediatePropagation() {}});
    assert.ok(suppressed, 'pinch release cannot open an event');
    s.reload();
    s.touch('touchstart', [a, b]);
    s.touch('touchmove', [s.finger(7, 100, 250), s.finger(8, 100, 350)]);
    assert.equal(s.zoom(), 1, 'zooming out starts from the retained scale after changing views');
    s.touch('touchend', [], [a, b]);
}
{
    const s = setup(), a = s.finger(7, 100, 200), b = s.finger(8, 300, 200);
    s.touch('touchstart', [a, b]);
    s.touch('touchmove', [s.finger(7, 10, 200), s.finger(8, 1010, 200)]);
    assert.equal(s.zoom(), 3, 'zooming in is bounded even with a horizontal pinch');
    s.touch('touchmove', [s.finger(7, 100, 200), s.finger(8, 101, 200)]);
    assert.equal(s.zoom(), 0.5, 'zooming out is bounded');
    s.touch('touchcancel', [], [a, b]);
    assert.deepEqual(s.writes, [{zoom: 0.5}], 'cancelling retains the displayed scale without editing');
    s.down(); s.hold(); s.move(100, 565); s.up();
    assert.equal(s.writes[1].start_delta, 30, 'dragging after zoom still uses five-minute time units');
    assert.equal(s.writes[1].end_delta, 30, 'dragging after zoom preserves the event duration');
}
{
    const s = setup(), a = s.finger(7, 100, 200), b = s.finger(8, 100, 400);
    s.down(); s.hold(); s.move(100, 580);
    assert.ok(s.preview());
    s.touch('touchstart', [a, b]);
    assert.equal(s.preview(), undefined, 'a pinch discards an unsaved drag preview');
    s.touch('touchmove', [s.finger(7, 100, 100), s.finger(8, 100, 500)]);
    s.touch('touchend', [], [a, b]);
    assert.deepEqual(s.writes, [{zoom: 2}]);
}
{
    const s = setup(), a = s.finger(7, 100, 200), b = s.finger(8, 100, 400), c = s.finger(9, 200, 300);
    s.touch('touchstart', [a, b]);
    s.touch('touchmove', [s.finger(7, 100, 100), s.finger(8, 100, 500)]);
    s.touch('touchstart', [a, b, c], [c]);
    s.touch('touchend', [a, b], [c]);
    s.touch('touchmove', [a, b]);
    assert.equal(s.zoom(), 2, 'adding or removing a third finger cannot jump the zoom');
    s.touch('touchend', [], [a, b]);
    assert.deepEqual(s.writes, [{zoom: 2}]);
}
for (const blocked of ['outside', 'modal']) {
    const s = setup(), a = s.finger(7, 100, 200);
    const b = s.finger(8, 100, 400, blocked === 'outside' ? {closest: () => null} : s.source);
    s.doc.modalOpen = blocked === 'modal';
    assert.equal(s.touch('touchstart', [a, b]), false);
    assert.equal(s.touch('touchmove', [a, b]), false);
    assert.equal(s.zoom(), 1, 'pinching outside the timeline or over a sheet leaves the calendar alone');
    assert.deepEqual(s.writes, []);
}
{
    const s = setup(); s.alarm(); s.down(); s.up(); s.hold();
    assert.deepEqual(s.writes, [{alarm_day: '2026-11-17', hour_start: 0, minute: 550}], 'an event tap selects an alarm time without moving or opening the event');
    assert.equal(s.preview(), undefined, 'alarm mode never selects an event');
    let suppressed = false;
    s.listeners.click({target: s.source, preventDefault() {suppressed = true;}, stopImmediatePropagation() {}});
    assert.ok(suppressed, 'a tap cannot open the event behind the alarm needle');
    suppressed = false;
    s.listeners.click({target: s.doc.alarmBar, preventDefault() {suppressed = true;}, stopImmediatePropagation() {}});
    assert.equal(suppressed, false, 'confirm and cancel work immediately after adjusting the needle');
}
{
    const s = setup(), needle = s.alarm();
    s.down(needle, 100, 612); s.move(100, 643); s.up();
    assert.deepEqual(s.writes, [{alarm_day: '2026-11-17', hour_start: 0, minute: 631}], 'dragging uses exact minutes and preserves the finger grab offset');
    assert.equal(needle.dataset.minute, '631');
    assert.equal(needle.style.top, (631 / 1440 * 100) + '%', 'the needle previews the chosen time before confirmation');
}
{
    const s = setup(), needle = s.alarm();
    s.scroller.scrollTop = 300;
    s.down(needle, 100, 300); s.scroller.scrollTop = 360; s.move(100, 300); s.up();
    assert.equal(s.writes[0].minute, 660, 'needle dragging accounts for scrolling');
    s.down(needle, 100, 300); s.move(100, -5000); s.up();
    assert.equal(s.writes.at(-1).minute, 0, 'needle clamps to the top of the day');
    s.down(needle, 100, -360); s.move(100, 5000); s.up();
    assert.equal(s.writes.at(-1).minute, 1439, 'needle clamps to the final minute');
}
{
    const s = setup(); s.alarm();
    const a = s.finger(7, 100, 300);
    assert.equal(s.touch('touchstart', [a]), false);
    assert.equal(s.touch('touchmove', [s.finger(7, 100, 200)]), false, 'swiping away from the needle retains native scrolling');
    s.touch('touchend', [], [a]); s.hold();
    assert.deepEqual(s.writes, [], 'a scroll never changes the selected alarm time');
    s.down(s.second, 300, 570); s.up();
    assert.deepEqual(s.writes, [{alarm_day: '2026-11-18', hour_start: 0, minute: 570}], 'tapping another visible day selects that date');
}
{
    const s = setup(), needle = s.alarm();
    const a = s.finger(7, 100, 600, needle), b = s.finger(8, 100, 800);
    s.touch('touchstart', [a]);
    assert.equal(s.touch('touchmove', [s.finger(7, 100, 617, needle)]), true, 'only dragging the needle claims one finger');
    assert.equal(s.writes[0].minute, 617);
    s.touch('touchstart', [a, b], [b]);
    s.touch('touchmove', [s.finger(7, 100, 500), s.finger(8, 100, 900)]);
    s.touch('touchend', [], [a, b]);
    assert.equal(s.zoom(), 2, 'pinching during alarm placement zooms normally');
    assert.deepEqual(s.writes.at(-1), {zoom: 2});
    assert.equal(s.writes.filter(w => w.alarm_day).length, 1, 'pinching leaves the chosen alarm time alone');
    s.touch('touchstart', [a, b]);
    s.touch('touchmove', [s.finger(7, 100, 650), s.finger(8, 100, 750)]);
    s.touch('touchend', [], [a, b]);
    assert.equal(s.zoom(), 1, 'pinching back zooms out while setting an alarm');
}
{
    const s = setup(), needle = s.alarm();
    s.listeners.keydown({target: needle, key: 'ArrowDown', preventDefault() {}});
    assert.equal(s.writes[0].minute, 601, 'the needle supports keyboard minute adjustments');
    s.listeners.keydown({target: needle, key: 'Escape'});
    assert.deepEqual(s.writes.at(-1), {cancel_alarm: true}, 'Escape cancels without saving');
}
{
    const s = setup(); s.alarm(); s.down(); s.up(true);
    assert.deepEqual(s.writes, [], 'a cancelled tap does not select an alarm time');
    s.doc.modalOpen = true; s.down(); s.up();
    assert.deepEqual(s.writes, [], 'a sheet blocks alarm gestures underneath it');
}
const ranges = s => s.writes.filter(w => w.range_day);
const lastRange = s => ranges(s).at(-1);
const span = s => [lastRange(s).range_start, lastRange(s).range_end];
const cancels = s => s.writes.filter(w => w.cancel_range).length;
const draft = (s, grid = s.first, x = 100, y = 550) => { s.down(grid, x, y); s.hold(); s.up(); return s.preview(); };
const knob = (preview, edge) => preview.children.find(n => n.dataset.resize === edge);
{
    const s = setup(); s.down(s.first, 100, 550); s.up(); s.hold();
    assert.equal(s.preview(), undefined, 'a tap on the empty grid draws nothing');
    s.down(s.first, 100, 550); s.move(100, 600); s.hold();
    assert.equal(s.preview(), undefined, 'scrolling the empty grid draws nothing');
    assert.deepEqual(s.writes, []);
}
{
    const s = setup(), preview = draft(s);
    assert.ok(preview.classes.has('calendar-draft'), 'holding the empty grid draws a gray range');
    assert.deepEqual(preview.children.filter(n => n.dataset.resize).map(n => n.dataset.resize), ['start', 'end'], 'a range has the handles of a held event');
    assert.deepEqual(s.writes, [{range_day: '2026-11-17', hour_start: 0, range_start: 540, range_end: 600}], 'it starts on the quarter hour held and lasts an hour');
    assert.equal(preview.style.top, '540px');
    assert.equal(preview.style.height, '60px');
}
for (const [y, start] of [[5, 0], [1435, 1380]]) {
    const s = setup(); draft(s, s.first, 100, y);
    assert.deepEqual(span(s), [start, start + 60], 'a range held at the edge of the day stays inside it');
}
{
    const s = setup(); s.down(s.first, 100, 550); s.hold(); s.move(100, 610); s.up();
    assert.deepEqual(span(s), [600, 660], 'the same press can drag the new range');
    s.down(s.preview(), 100, 620); s.move(100, 650); s.up();
    assert.deepEqual(span(s), [630, 690], 'a second drag continues from where the first ended');
    assert.equal(s.writes.filter(w => w.id).length, 0, 'adjusting a range never writes an event');
}
{
    const s = setup(), preview = draft(s);
    s.down(knob(preview, 'start'), 100, 540); s.move(100, 510); s.up();
    assert.deepEqual(span(s), [510, 600], 'the top handle changes the start');
    s.down(knob(preview, 'end'), 100, 600); s.move(100, 645); s.up();
    assert.deepEqual(span(s), [510, 645], 'the bottom handle changes the end');
    s.down(knob(preview, 'start'), 100, 510); s.move(100, 900); s.up();
    assert.deepEqual(span(s), [640, 645], 'a range keeps at least five minutes');
}
{
    const s = setup(), preview = draft(s);
    s.down(preview, 100, 550); s.move(300, 550); s.up();
    assert.equal(lastRange(s).range_day, '2026-11-18', 'a range moves between visible days');
    assert.equal(preview.parentNode, s.second);
    s.down(knob(preview, 'end'), 300, 600); s.move(300, 630); s.up();
    assert.deepEqual([lastRange(s).range_day, lastRange(s).range_end], ['2026-11-18', 630], 'resizing stays on the new day');
}
{
    const s = setup(), preview = draft(s), sent = s.writes.length;
    s.down(preview, 100, 550); s.move(100, 551); s.move(100, 552);
    assert.equal(s.writes.length, sent, 'a range only reports whole five-minute changes');
    s.move(100, 600); s.up(true);
    assert.equal(s.preview(), preview, 'a cancelled drag keeps the range');
    assert.deepEqual(span(s), [540, 600], 'and puts it back');
    assert.equal(cancels(s), 0);
}
{
    const s = setup(), preview = draft(s), a = s.finger(7, 100, 300, s.first);
    assert.equal(s.touch('touchstart', [a]), false, 'a finger beside the range still scrolls natively');
    s.touch('touchmove', [s.finger(7, 100, 200, s.first)]);
    s.touch('touchend', [], [s.finger(7, 100, 200, s.first)]);
    assert.equal(s.preview(), preview, 'scrolling keeps the range');
    const b = s.finger(8, 100, 500, s.first);
    s.touch('touchstart', [a, b]);
    s.touch('touchmove', [s.finger(7, 100, 100, s.first), s.finger(8, 100, 700, s.first)]);
    s.touch('touchend', [], [a, b]);
    assert.equal(s.preview(), preview, 'pinching keeps the range');
    assert.equal(preview.style.top, 540 * s.zoom() + 'px', 'the range follows the zoomed grid');
    s.context.window.__calendarGesture.setZoom(1);
    assert.equal(preview.style.top, '540px', 'menu zoom also keeps the range in place');
    assert.equal(cancels(s), 0);
}
{
    const s = setup(); draft(s);
    s.down(s.first, 100, 900); s.up();
    assert.equal(s.preview(), undefined, 'a tap on the grid dismisses the range');
    assert.equal(cancels(s), 1, 'and clears the confirmation bar');
    let suppressed = false;
    s.listeners.click({target: s.first, preventDefault() {suppressed = true;}, stopImmediatePropagation() {}});
    assert.ok(suppressed, 'that tap opens nothing underneath');
}
{
    const s = setup(), first = draft(s);
    draft(s, s.first, 100, 900);
    assert.notEqual(s.preview(), first, 'holding elsewhere replaces the range');
    assert.equal(lastRange(s).range_start, 900);
    assert.equal(cancels(s), 0, 'replacing never flashes the bar away');
}
{
    const s = setup(); draft(s);
    s.down(s.source, 100, 550); s.hold(); s.up();
    assert.ok(s.preview() && !s.preview().classes.has('calendar-draft'), 'holding an event selects that event instead');
    assert.equal(cancels(s), 1);
}
{
    const s = setup(); s.source.dataset.draggable = 'false';
    s.down(s.source, 100, 550); s.hold();
    assert.equal(s.preview(), undefined, 'an event that cannot move is not empty grid');
    assert.deepEqual(s.writes, []);
}
{
    const s = setup(), bar = new s.first.constructor('placement-bar range-draft-bar'), preview = draft(s);
    s.down(preview, 100, 550); s.move(100, 560); s.up();
    s.down(bar, 100, 790); s.up();
    assert.equal(s.preview(), preview, 'pressing the bar leaves the range for its own buttons');
    let suppressed = false;
    s.listeners.click({target: bar, preventDefault() {suppressed = true;}, stopImmediatePropagation() {}});
    assert.equal(suppressed, false, 'Confirm time responds immediately after a drag');
}
{
    const s = setup(); draft(s); const sent = s.writes.length;
    s.context.window.__calendarGesture.dismissDraft();
    assert.equal(s.preview(), undefined, 'Rust can take the range off the grid');
    assert.equal(s.writes.length, sent, 'without being told what it already knows');
}
{
    const s = setup(); s.down(s.source, 100, 550); s.hold(); s.up();
    s.context.window.__calendarGesture.dismissDraft();
    assert.ok(s.preview(), 'dismissing a range leaves a selected event alone');
}
{
    const s = setup(); s.scroller.scrollTop = 300; draft(s, s.first, 100, 200);
    assert.equal(s.scroller.scrollTop, 300, 'a range in clear view does not scroll');
    s.down(s.preview(), 100, 200); s.move(100, 760); s.up();
    assert.ok(s.scroller.scrollTop > 300, 'a range left under the bar scrolls into view');
}
{
    const s = setup(); s.alarm(); s.down(s.first, 100, 550); s.hold(); s.up();
    assert.deepEqual(ranges(s), [], 'alarm placement keeps holding for the needle');
}
{
    const s = setup(); s.doc.modalOpen = true; s.down(s.first, 100, 550); s.hold();
    assert.equal(s.preview(), undefined, 'a sheet blocks drawing underneath it');
}
console.log('Calendar drag, resize, pinch zoom, alarm placement, new range, scrolling and cancellation checks passed');
