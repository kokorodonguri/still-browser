package app.still.browser

import android.app.DownloadManager
import android.content.ActivityNotFoundException
import android.content.ContentUris
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Environment
import android.provider.MediaStore
import android.webkit.MimeTypeMap
import android.webkit.URLUtil
import org.mozilla.geckoview.WebResponse
import java.io.IOException
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors

/**
 * The engine hands a download over as a response stream (with the page's
 * cookies already applied). It is written into the public Downloads folder
 * through MediaStore, which needs no storage permission, never overwrites a
 * file (it picks a free name) and hides the file until it is complete.
 *
 * [SavedDownload.id] is negative for these: minus the MediaStore row id.
 * Positive ids are DownloadManager downloads from the WebView builds (≤ 0.4).
 */
class Downloads(private val context: Context) {
    private val resolver = context.contentResolver
    private val io = Executors.newCachedThreadPool()
    /** Downloads this process is writing, by MediaStore row id. */
    private val live = ConcurrentHashMap<Long, Progress>()

    private class Progress {
        @Volatile var bytes = 0L
        @Volatile var state = DownloadState.InProgress
    }

    /** Starts saving [response]; null when the file cannot be created. */
    fun start(response: WebResponse): SavedDownload? {
        val body = response.body ?: return null
        val header = { name: String -> response.headers.entries.firstOrNull { it.key.equals(name, ignoreCase = true) }?.value }
        val mime = header("Content-Type")?.substringBefore(';')?.trim()?.lowercase()?.takeIf { it.isNotEmpty() }
        val name = URLUtil.guessFileName(response.uri, header("Content-Disposition"), mime)
            .replace(Regex("[\\\\/:*?\"<>|\\u0000-\\u001f]"), "_")
            .ifBlank { "download" }
        val type = mime?.takeIf { it != "application/octet-stream" }
            ?: MimeTypeMap.getSingleton().getMimeTypeFromExtension(name.substringAfterLast('.', "").lowercase())
            ?: "application/octet-stream"
        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, name)
            put(MediaStore.Downloads.MIME_TYPE, type)
            put(MediaStore.Downloads.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS)
            put(MediaStore.Downloads.IS_PENDING, 1)
        }
        val uri = try {
            resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
        } catch (e: RuntimeException) {
            null
        }
        if (uri == null) {
            closeQuietly(body)
            return null
        }
        val row = ContentUris.parseId(uri)
        val stored = displayName(uri) ?: name
        val progress = Progress()
        live[row] = progress
        io.execute { write(uri, body, progress) }
        val total = header("Content-Length")?.toLongOrNull()?.coerceAtLeast(0) ?: 0
        return SavedDownload(stored, DownloadState.InProgress, 0, total, -row)
    }

    private fun write(uri: Uri, body: java.io.InputStream, progress: Progress) {
        val ok = try {
            resolver.openOutputStream(uri)?.use { out ->
                body.use { input ->
                    val buffer = ByteArray(64 * 1024)
                    while (true) {
                        val read = input.read(buffer)
                        if (read < 0) break
                        out.write(buffer, 0, read)
                        progress.bytes += read
                    }
                }
                true
            } ?: false
        } catch (e: IOException) {
            false
        } catch (e: RuntimeException) {
            false
        }
        try {
            if (ok) {
                resolver.update(uri, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
            } else {
                // A partial file is not left behind.
                resolver.delete(uri, null, null)
            }
        } catch (e: RuntimeException) {
            // The entry stays pending; Android removes pending files after a week.
        }
        progress.state = if (ok) DownloadState.Completed else DownloadState.Failed
    }

    private fun displayName(uri: Uri): String? = try {
        resolver.query(uri, arrayOf(MediaStore.Downloads.DISPLAY_NAME), null, null, null)?.use { c ->
            if (c.moveToFirst()) c.getString(0) else null
        }
    } catch (e: RuntimeException) {
        null
    }

    private fun closeQuietly(stream: java.io.InputStream) {
        try {
            stream.close()
        } catch (e: IOException) {
            // Nothing to keep.
        }
    }

    /** Current progress of in-progress entries; others are returned unchanged. */
    fun refresh(list: List<SavedDownload>): List<SavedDownload> = list.map { d ->
        if (d.state != DownloadState.InProgress) return@map d
        // Not written by this process: it ended while the download ran.
        val now = live[-d.id] ?: return@map d.copy(state = DownloadState.Interrupted)
        if (now.state != DownloadState.InProgress) live.remove(-d.id)
        d.copy(state = now.state, bytes = now.bytes, total = maxOf(d.total, if (now.state == DownloadState.Completed) now.bytes else 0))
    }

    /** Opens a finished file in another app; false when none can. */
    fun open(download: SavedDownload): Boolean {
        val (uri, type) = when {
            download.id < 0 -> {
                val uri = ContentUris.withAppendedId(MediaStore.Downloads.EXTERNAL_CONTENT_URI, -download.id)
                uri to (resolver.getType(uri) ?: "*/*")
            }
            download.id > 0 -> {
                val manager = context.getSystemService(DownloadManager::class.java)
                val uri = try {
                    manager.getUriForDownloadedFile(download.id)
                } catch (e: RuntimeException) {
                    null
                } ?: return false
                uri to (manager.getMimeTypeForDownloadedFile(download.id) ?: "*/*")
            }
            else -> return false
        }
        val intent = Intent(Intent.ACTION_VIEW)
            .setDataAndType(uri, type)
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        return try {
            context.startActivity(intent)
            true
        } catch (e: ActivityNotFoundException) {
            false
        }
    }

    fun shutdown() {
        io.shutdown()
    }
}
