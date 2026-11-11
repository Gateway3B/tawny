import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

const source = readFileSync(
  new URL("../assets/tawny_mouse_navigation.js", import.meta.url),
  "utf8",
);
function setup() {
  const document = new EventTarget();
  const window = {};
  const sent = [];
  const context = {
    document,
    window,
    AbortController,
    dioxus: { send: (value) => sent.push(value) },
  };
  const install = () => vm.runInNewContext(`(async () => { ${source} })()`, context);
  const emit = (type, properties) => {
    const event = new Event(type, { cancelable: true });
    for (const [key, value] of Object.entries(properties))
      Object.defineProperty(event, key, { value });
    document.dispatchEvent(event);
    return event;
  };
  const dispose = () => window[Symbol.for("tawny.mouse-navigation")]?.dispose();
  return { sent, install, emit, dispose };
}

test("side buttons navigate once per release and suppress WebView navigation", async () => {
  const fixture = setup();
  const closed = fixture.install();
  for (const button of [3, 4]) {
    for (const type of ["mousedown", "mouseup", "auxclick"]) {
      assert.equal(fixture.emit(type, { button }).defaultPrevented, true);
    }
  }
  assert.deepEqual(fixture.sent, [-1, 1]);
  fixture.dispose();
  await closed;
});

test("ordinary mouse buttons are left alone", async () => {
  const fixture = setup();
  const closed = fixture.install();
  for (const button of [0, 1, 2])
    assert.equal(fixture.emit("mouseup", { button }).defaultPrevented, false);
  assert.deepEqual(fixture.sent, []);
  fixture.dispose();
  await closed;
});

test("browser keys work and reinstalling or disposing does not duplicate listeners", async () => {
  const fixture = setup();
  const first = fixture.install();
  const second = fixture.install();
  await first;
  fixture.emit("keydown", { key: "BrowserBack", repeat: false });
  fixture.emit("keydown", { key: "BrowserForward", repeat: false });
  fixture.emit("keydown", { key: "BrowserForward", repeat: true });
  assert.deepEqual(fixture.sent, [-1, 1]);
  fixture.dispose();
  await second;
  fixture.emit("mouseup", { button: 3 });
  assert.deepEqual(fixture.sent, [-1, 1]);
});
