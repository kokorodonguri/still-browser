package app.still.browser

import org.json.JSONException
import java.io.File
import java.io.FileOutputStream
import java.io.IOException

/**
 * Reads and writes `state.json`. A file that cannot be understood (corrupt, or
 * written by a newer version) is copied aside before the first overwrite, and no
 * overwrite happens until that copy has succeeded. Writes go to a temporary file
 * that is synced and then renamed over the old one, so a killed process never
 * leaves a half-written file.
 *
 * Not thread-safe: the activity calls [save] from one writer thread only.
 */
class StateStore(private val file: File) {
    enum class LoadProblem { Corrupt, Newer, Unreadable }

    /** The original that must be preserved before [file] may be replaced, and its backup name. */
    private var pendingBackup: String? = null

    fun load(): Pair<BrowserState, LoadProblem?> {
        val text = try {
            if (!file.exists()) return BrowserState() to null
            file.readText()
        } catch (e: IOException) {
            pendingBackup = "unreadable-${System.currentTimeMillis()}"
            return BrowserState() to LoadProblem.Unreadable
        }
        val state = try {
            BrowserState.parse(text)
        } catch (e: JSONException) {
            pendingBackup = "invalid-${System.currentTimeMillis()}"
            return BrowserState() to LoadProblem.Corrupt
        }
        if (state.version > BrowserState.SCHEMA_VERSION) {
            pendingBackup = "v${state.version}-backup"
            return state to LoadProblem.Newer
        }
        return state to null
    }

    /** Returns false when nothing was written; the caller retries on the next change. */
    fun save(json: String): Boolean {
        pendingBackup?.let { suffix ->
            if (file.exists() && !backup(suffix)) return false
            pendingBackup = null
        }
        val temp = File(file.parentFile, file.name + ".tmp")
        return try {
            file.parentFile?.mkdirs()
            FileOutputStream(temp).use { out ->
                out.write(json.toByteArray(Charsets.UTF_8))
                out.fd.sync()
            }
            if (!temp.renameTo(file)) throw IOException("rename failed")
            true
        } catch (e: IOException) {
            temp.delete()
            false
        }
    }

    /** Copies the current file to a name that does not exist yet. */
    private fun backup(suffix: String): Boolean {
        val dir = file.parentFile ?: return false
        var target = File(dir, "${file.name}.$suffix")
        var n = 1
        while (target.exists() && n < 10_000) target = File(dir, "${file.name}.$suffix-${n++}")
        if (target.exists()) return false
        return try {
            file.copyTo(target)
            FileOutputStream(target, true).use { it.fd.sync() }
            true
        } catch (e: IOException) {
            target.delete()
            false
        }
    }
}
