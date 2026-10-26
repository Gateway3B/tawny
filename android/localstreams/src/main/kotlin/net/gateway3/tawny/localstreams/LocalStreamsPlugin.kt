package net.gateway3.tawny.localstreams

import android.app.Activity
import android.content.Context
import androidx.javascriptengine.IsolateStartupParameters
import androidx.javascriptengine.JavaScriptSandbox
import com.chaquo.python.PyException
import com.chaquo.python.Python
import com.chaquo.python.android.AndroidPlatform
import java.io.File
import java.util.concurrent.TimeUnit
import org.json.JSONObject

/// Resolves a video's streams on the phone. Rust calls `resolveFromRust` from
/// a worker thread (it blocks for the seconds yt-dlp takes) and gets back
/// `{"source": PlaybackSource}` or `{"error": "why"}`.
class LocalStreamsPlugin(private val activity: Activity) {
    fun resolveFromRust(videoId: String): String {
        return try {
            val context = activity.applicationContext
            JsSandbox.context = context
            if (!Python.isStarted()) Python.start(AndroidPlatform(context))
            val cacheDir = File(context.cacheDir, "yt-dlp").path
            val source = Python.getInstance()
                .getModule("tawny_local")
                .callAttr("resolve", videoId, cacheDir)
                .toString()
            "{\"source\":$source}"
        } catch (error: PyException) {
            JSONObject().put("error", error.message ?: "yt-dlp failed").toString()
        } catch (error: Throwable) {
            JSONObject().put("error", error.toString()).toString()
        }
    }
}

/// Android's V8, for yt-dlp's challenge solver (`tawny_jsc.py` calls
/// `evaluate` through Chaquopy). One sandbox process for the app's life, and a
/// fresh isolate per script, so one solve cannot leave state for the next.
object JsSandbox {
    @Volatile
    var context: Context? = null
    private var sandbox: JavaScriptSandbox? = null

    @JvmStatic
    fun evaluate(script: String): String {
        val box = connect()
        // The player script is megabytes; without this feature a script that
        // size has to cross a binder transaction, which holds about one.
        check(box.isFeatureSupported(JavaScriptSandbox.JS_FEATURE_EVALUATE_WITHOUT_TRANSACTION_LIMIT)) {
            "This device's WebView is too old to solve YouTube's challenges; update Android System WebView"
        }
        val parameters = IsolateStartupParameters()
        if (box.isFeatureSupported(JavaScriptSandbox.JS_FEATURE_ISOLATE_MAX_HEAP_SIZE)) {
            parameters.maxHeapSizeBytes = 512L * 1024 * 1024
        }
        val isolate = box.createIsolate(parameters)
        try {
            return isolate.evaluateJavaScriptAsync(script).get(60, TimeUnit.SECONDS)
        } finally {
            isolate.close()
        }
    }

    @Synchronized
    private fun connect(): JavaScriptSandbox {
        sandbox?.let { return it }
        val context = checkNotNull(context) { "The sandbox was asked for before the app set its context" }
        check(JavaScriptSandbox.isSupported()) { "This device has no JavaScript sandbox; update Android System WebView" }
        // Waits on the main thread's binder callback, so this must never be
        // called from the main thread. It is not: Rust calls in from a worker.
        val connected = JavaScriptSandbox.createConnectedInstanceAsync(context).get(30, TimeUnit.SECONDS)
        sandbox = connected
        return connected
    }
}
