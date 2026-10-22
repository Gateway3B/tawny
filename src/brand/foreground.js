// Timers can pause in a background webview. Check immediately when it returns.
const report = () => {
  if (document.visibilityState === "visible") dioxus.send(true);
};
document.addEventListener("visibilitychange", report);
window.addEventListener("pageshow", report);
window.addEventListener("online", report);
const timer = setInterval(report, 60000);
try {
  await dioxus.recv();
} finally {
  clearInterval(timer);
  document.removeEventListener("visibilitychange", report);
  window.removeEventListener("pageshow", report);
  window.removeEventListener("online", report);
}
