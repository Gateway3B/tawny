"""yt-dlp's JavaScript challenge solver, run in Android's JavaScript sandbox.

yt-dlp solves YouTube's signature and `n` challenges by running its EJS solver
script in an external runtime (Deno, Node, Bun or QuickJS) as a subprocess.
An Android app cannot ship such an executable: the platform refuses to run
binaries from app storage, and native libraries are not extracted to disk
unless the app's own Gradle file asks, which `dx` generates. Android does have
V8, though, through `androidx.javascriptengine`, so this provider hands the
same script to that instead.

`evaluate` is set by the host (`LocalStreams.kt` on Android, a Node stand-in in
the tests): it takes a script and returns the value of its last expression as
a string.
"""

from __future__ import annotations

import time

from yt_dlp.extractor.youtube.jsc._builtin.ejs import EJSBaseJCP
from yt_dlp.extractor.youtube.jsc.provider import register_preference, register_provider
from yt_dlp.extractor.youtube.pot._provider import BuiltinIEContentProvider
from yt_dlp.utils._jsruntime import JsRuntimeInfo

evaluate = None

# The solver script reports through `console.log`, which the sandbox does not
# have. Capture it, and answer with it as the script's last expression. The
# preprocessed player it also returns is megabytes yt-dlp then ignores (its
# cache of it is off), and would only cross the binder for nothing.
_HEAD = (
    "var __tawnyOut = null;"
    "var console = { log: function (s) { __tawnyOut = s; },"
    " info: function () {}, warn: function () {}, error: function () {}, debug: function () {} };\n"
)
_TAIL = (
    "\n;(function () { var o = JSON.parse(__tawnyOut);"
    " delete o.preprocessed_player; return JSON.stringify(o); })()"
)


@register_provider
class AndroidSandboxJCP(EJSBaseJCP, BuiltinIEContentProvider):
    PROVIDER_NAME = "tawny-android"
    JS_RUNTIME_NAME = "tawny-android"

    @property
    def runtime_info(self):
        if evaluate is None:
            return None
        return JsRuntimeInfo(
            name="tawny-android",
            path="androidx.javascriptengine",
            version="1",
            version_tuple=(1,),
        )

    def _run_js_runtime(self, stdin: str, /) -> str:
        started = time.monotonic()
        try:
            return evaluate(_HEAD + stdin + _TAIL)
        finally:
            print(f"tawny: solved challenges in {time.monotonic() - started:.1f}s ({len(stdin) // 1024} KB script)", flush=True)


@register_preference(AndroidSandboxJCP)
def preference(provider, requests) -> int:
    # Ahead of every built-in runtime; none of them exists on a phone anyway.
    return 2000
