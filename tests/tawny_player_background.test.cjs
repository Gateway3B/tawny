"use strict";

// With the screen off in the Android app, the WebView keeps running and the
// page stays "visible", so playback that paused or stuck there was only
// recovered once the screen came back on. The decisions are pure functions in
// the controls IIFE, lifted out of the shipped source as the timeline test does.

const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { test } = require("node:test");

const SOURCE = fs.readFileSync(
  path.join(__dirname, "..", "assets", "tawny_player_controls.js"),
  "utf8",
);

function lift(name) {
  const at = SOURCE.indexOf(`function ${name}(`);
  assert.notEqual(at, -1, `${name} is gone from tawny_player_controls.js`);
  let depth = 0;
  let index = SOURCE.indexOf("{", at);
  for (; index < SOURCE.length; index++) {
    if (SOURCE[index] === "{") depth++;
    else if (SOURCE[index] === "}") {
      depth--;
      if (depth === 0) break;
    }
  }
  return SOURCE.slice(at, index + 1);
}

function liftConstants(prefix) {
  const lines = SOURCE.split("\n").filter((line) =>
    new RegExp(`^\\s*const ${prefix}\\w* = `).test(line),
  );
  assert.ok(lines.length > 0, `no ${prefix} constants`);
  return lines.join("\n");
}

const { playerBackgrounded, backgroundRecovery } = new Function(
  `${liftConstants("BACKGROUND_")}\n${lift("playerBackgrounded")}\n${lift("backgroundRecovery")}\n` +
    "return { playerBackgrounded, backgroundRecovery };",
)();

const androidDocument = ({ visible = true, focused = true, app = true } = {}) => ({
  visibilityState: visible ? "visible" : "hidden",
  documentElement: { dataset: app ? { androidApp: "true" } : {} },
  hasFocus: () => focused,
});

test("a hidden page is backgrounded everywhere", () => {
  assert.equal(playerBackgrounded(androidDocument({ visible: false, app: false })), true);
});

test("the Android app with its window unfocused is backgrounded though still visible", () => {
  assert.equal(playerBackgrounded(androidDocument({ focused: false })), true);
  assert.equal(playerBackgrounded(androidDocument({ focused: true })), false);
});

test("a browser tab that only lost focus is not backgrounded", () => {
  assert.equal(playerBackgrounded(androidDocument({ focused: false, app: false })), false);
});

const sample = (overrides = {}) => ({
  unfocusedVisible: true,
  intent: true,
  ended: false,
  busy: false,
  kicks: 0,
  hasMetadata: true,
  paused: false,
  seeking: false,
  stalledFor: 0,
  bufferedAhead: 0,
  ...overrides,
});

test("an unasked-for pause with the screen off is resumed after a moment", () => {
  assert.equal(backgroundRecovery(sample({ paused: true, stalledFor: 1 })), null);
  assert.equal(backgroundRecovery(sample({ paused: true, stalledFor: 2 })), "play");
});

test("a clock stopped with data buffered is nudged, one waiting for the network is not", () => {
  assert.equal(backgroundRecovery(sample({ stalledFor: 6, bufferedAhead: 10 })), "nudge");
  assert.equal(backgroundRecovery(sample({ stalledFor: 6, bufferedAhead: 1 })), null);
  assert.equal(backgroundRecovery(sample({ stalledFor: 3, bufferedAhead: 10 })), null);
  assert.equal(
    backgroundRecovery(sample({ stalledFor: 6, bufferedAhead: 10, seeking: true })),
    null,
  );
});

test("nothing is done for a viewer's pause, in the foreground, or once kicks run out", () => {
  const stuck = { paused: true, stalledFor: 10 };
  assert.equal(backgroundRecovery(sample({ ...stuck, intent: false })), null);
  assert.equal(backgroundRecovery(sample({ ...stuck, unfocusedVisible: false })), null);
  assert.equal(backgroundRecovery(sample({ ...stuck, ended: true })), null);
  assert.equal(backgroundRecovery(sample({ ...stuck, busy: true })), null);
  assert.equal(backgroundRecovery(sample({ ...stuck, hasMetadata: false })), null);
  assert.equal(backgroundRecovery(sample({ ...stuck, kicks: 5 })), null);
});
