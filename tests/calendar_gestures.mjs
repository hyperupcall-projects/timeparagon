// Exercise the actual webview gesture controller without credentials or writes.
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';
import assert from 'node:assert/strict';
const script = await readFile(new URL('../dioxus/src/calendar_gestures.js', import.meta.url), 'utf8');
function setup() {
    const listeners = {}, timers = new Map(), writes = [];
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
    const first = new Node('grid', {hourStart: '0', clock: '24'}), second = new Node('grid', {hourStart: '0', clock: '24'});
    first.clientHeight = second.clientHeight = 1440; scroller.append(first); scroller.append(second);
    const source = new Node('grid-event', {calendarEvent: 'event', eventCalendar: 'work', draggable: 'true', startMin: '540', endMin: '600', duration: '60', beforeStart: '1000', beforeEnd: '4600'});
    first.append(source);
    const doc = {querySelector: () => null, querySelectorAll: () => [first, second], createElement: () => new Node(), addEventListener: (type, fn) => { listeners[type] = fn; }};
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
    return {source, first, second, scroller, listeners, writes, context, down, move, up, hold, preview};
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
console.log('Calendar gesture checks passed');
