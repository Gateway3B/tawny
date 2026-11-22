package net.gateway3.tawny.localstreams

import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.os.ParcelFileDescriptor
import android.provider.MediaStore
import java.io.File
import org.json.JSONObject

/// Writes an exported download into the phone's Downloads folder, where any
/// app's file picker finds it. Rust (`src/video_export.rs`) asks for a file,
/// writes the bytes the page sends through the local relay straight into the
/// descriptor it gets back, then finishes or abandons it. Each call answers
/// JSON: the result, or `{"error": "why"}`.
class VideoExportPlugin(private val app: Context) {
    /// Opens `Download/Tawny/<displayName>` for writing:
    /// `{"uri": "...", "fd": 42}`. The descriptor is Rust's to close.
    fun beginFromRust(displayName: String, mimeType: String): String {
        return try {
            val (uri, descriptor) =
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) openShared(displayName, mimeType)
                else openPrivate(displayName)
            JSONObject().put("uri", uri.toString()).put("fd", descriptor.detachFd()).toString()
        } catch (error: Throwable) {
            failure(error)
        }
    }

    /// Publishes the file (`keep`) or deletes what was written. Answers
    /// `{"path": "Download/Tawny/name.mp4"}`, the name as it ended up: the
    /// system numbers a name that is already taken.
    fun finishFromRust(uriText: String, keep: String): String {
        return try {
            val uri = Uri.parse(uriText)
            val keepIt = keep == "true"
            val path =
                if (uri.scheme == "file") finishPrivate(uri, keepIt)
                else finishShared(uri, keepIt)
            JSONObject().put("path", path).toString()
        } catch (error: Throwable) {
            failure(error)
        }
    }

    /// Android 10 and later: the shared Downloads collection, with no storage
    /// permission. Pending until finished, so no app sees half a file.
    private fun openShared(displayName: String, mimeType: String): Pair<Uri, ParcelFileDescriptor> {
        val resolver = app.contentResolver
        val values = ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, displayName)
            put(MediaStore.MediaColumns.MIME_TYPE, mimeType)
            put(MediaStore.MediaColumns.RELATIVE_PATH, "${Environment.DIRECTORY_DOWNLOADS}/$FOLDER")
            put(MediaStore.MediaColumns.IS_PENDING, 1)
        }
        val uri = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
            ?: error("Android would not create the file in Downloads")
        val descriptor = try {
            resolver.openFileDescriptor(uri, "w") ?: error("Android would not open the new file")
        } catch (error: Throwable) {
            resolver.delete(uri, null, null)
            throw error
        }
        return uri to descriptor
    }

    private fun finishShared(uri: Uri, keep: Boolean): String {
        val resolver = app.contentResolver
        if (!keep) {
            resolver.delete(uri, null, null)
            return ""
        }
        resolver.update(
            uri,
            ContentValues().apply { put(MediaStore.MediaColumns.IS_PENDING, 0) },
            null,
            null,
        )
        resolver.query(
            uri,
            arrayOf(MediaStore.MediaColumns.RELATIVE_PATH, MediaStore.MediaColumns.DISPLAY_NAME),
            null,
            null,
            null,
        )?.use { cursor ->
            if (cursor.moveToFirst()) return cursor.getString(0).trimEnd('/') + "/" + cursor.getString(1)
        }
        return "${Environment.DIRECTORY_DOWNLOADS}/$FOLDER"
    }

    /// Android 8 and 9: the shared folder needs a storage permission Tawny
    /// does not ask for, so the file goes in the app's own Downloads folder,
    /// which file managers show under Android/data.
    private fun openPrivate(displayName: String): Pair<Uri, ParcelFileDescriptor> {
        val folder = File(app.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS), FOLDER)
        folder.mkdirs()
        val file = uniqueFile(folder, displayName)
        val descriptor = ParcelFileDescriptor.open(
            file,
            ParcelFileDescriptor.MODE_WRITE_ONLY or
                ParcelFileDescriptor.MODE_CREATE or
                ParcelFileDescriptor.MODE_TRUNCATE,
        )
        return Uri.fromFile(file) to descriptor
    }

    private fun finishPrivate(uri: Uri, keep: Boolean): String {
        val file = File(uri.path ?: error("The exported file has no path"))
        if (!keep) {
            file.delete()
            return ""
        }
        return file.path
    }

    private fun uniqueFile(folder: File, displayName: String): File {
        val stem = displayName.substringBeforeLast('.')
        val extension = displayName.substringAfterLast('.', "")
        var candidate = File(folder, displayName)
        var number = 1
        while (candidate.exists()) {
            val numbered = if (extension.isEmpty()) "$stem ($number)" else "$stem ($number).$extension"
            candidate = File(folder, numbered)
            number += 1
        }
        return candidate
    }

    private fun failure(error: Throwable): String =
        JSONObject().put("error", error.message ?: error.toString()).toString()

    companion object {
        private const val FOLDER = "Tawny"
    }
}
