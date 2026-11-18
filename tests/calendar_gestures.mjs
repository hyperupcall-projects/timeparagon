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
        querySelector() { return timeline; }
        get clientHeight() { return 1440 * +(properties.get('--calendar-zoom') || 1); }
        closest(selector) {
            const matches = selector === '[data-resize]' ? this.dataset.resize
                : selector === '[data-calendar-event][data-draggable="true"]' ? this.dataset.calendarEvent && this.dataset.draggable === 'true'
                : selector.startsWith('.') && this.classes.has(selector.slice(1));
            return matches ? this : this.parentNode?.closest(selector) || null;
        }
        getBoundingClientRect() {
            if (this.classes.has('timeline-scroll')) return {top: 0, bottom: 800};
            return {top: -scroller.scrollTop, left: this === first ? 0 : 200, right: this === first ? 200 : 400};
        }
    }
    const scroller = new Node('timeline-scroll'); scroller.scrollTop = 0;
    const timeline = new Node('timeline'); scroller.append(timeline);
    const first = new Node('grid', {hourStart: '0', clock: '24'}), second = new Node('grid', {hourStart: '0', clock: '24'});
    timeline.append(first); timeline.append(second);
    const source = new Node('grid-event', {calendarEvent: 'event', eventCalendar: 'work', draggable: 'true', startMin: '540', endMin: '600', duration: '60', beforeStart: '1000', beforeEnd: '4600'});
    first.append(source);
    const doc = {documentElement: {style: rootStyle}, querySelector: () => doc.modalOpen ? {} : null,
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
    return {source, first, second, scroller, timeline, doc, options, listeners, writes, context, down, move, up, hold, preview, finger, touch, zoom, reload};
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
console.log('Calendar drag, resize, pinch zoom, anchoring, limits and cancellation checks passed');
