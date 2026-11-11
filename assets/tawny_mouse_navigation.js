const key = Symbol.for("tawny.mouse-navigation");
window[key]?.dispose();
const abort = new AbortController();
let finish;
const closed = new Promise((resolve) => {
  finish = resolve;
});
const options = { capture: true, signal: abort.signal };
const sideButton = (event) => event.button === 3 || event.button === 4;
const consume = (event) => {
  if (!sideButton(event)) return;
  // The WebView's browser history is separate from the native router's.
  // Suppress its default traversal and send exactly once, on release.
  event.preventDefault();
  event.stopPropagation();
  if (event.type === "mouseup") dioxus.send(event.button === 3 ? -1 : 1);
};
for (const type of ["mousedown", "mouseup", "auxclick"]) {
  document.addEventListener(type, consume, options);
}
document.addEventListener(
  "keydown",
  (event) => {
    if (event.key !== "BrowserBack" && event.key !== "BrowserForward") return;
    event.preventDefault();
    event.stopPropagation();
    if (!event.repeat) dioxus.send(event.key === "BrowserBack" ? -1 : 1);
  },
  options,
);
window[key] = {
  dispose() {
    abort.abort();
    finish();
    delete window[key];
  },
};
await closed;
