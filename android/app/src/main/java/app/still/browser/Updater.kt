package app.still.browser

import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.os.Build
import android.os.Handler
import android.os.Looper
import org.json.JSONObject
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.security.MessageDigest
import java.util.concurrent.Executors

/**
 * Updates Still from the download site (`site/build.py`) at the `update_url`
 * resource (set in build.gradle.kts). update.json names the
 * newest APK with its versionCode, SHA-256 and size. The APK is verified before
 * it is handed to the system installer, which also refuses an APK signed with a
 * different key than the installed app. The installer's answer arrives at
 * [UpdateStatusReceiver].
 */
class Updater(private val context: Context) {
    data class Release(val version: String, val code: Long, val url: String, val sha256: String, val size: Long)

    sealed class Check {
        object UpToDate : Check()
        data class Available(val release: Release) : Check()
        object Failed : Check()
    }

    private val worker = Executors.newSingleThreadExecutor()
    private val main = Handler(Looper.getMainLooper())
    private val apk = File(context.cacheDir, "update.apk")

    /** Reads update.json off the main thread; [onResult] runs on the main thread. */
    fun check(onResult: (Check) -> Unit) = background(Check.Failed, onResult) {
        val url = context.getString(R.string.update_url)
        val text = fetch(URL(url)) { it.readLimited(MANIFEST_LIMIT).decodeToString() }
        parse(text, installedCode(context), url, Build.SUPPORTED_ABIS.toList())?.let { Check.Available(it) } ?: Check.UpToDate
    }

    /** Downloads and verifies [release], then opens the system installer. [onResult] gets false on failure. */
    fun install(release: Release, onResult: (Boolean) -> Unit) = background(false, onResult) {
        if (release.size !in 1..SIZE_LIMIT) return@background false
        val digest = MessageDigest.getInstance("SHA-256")
        var received = 0L
        fetch(URL(release.url)) { input ->
            apk.outputStream().use { output ->
                val buffer = ByteArray(1 shl 16)
                while (true) {
                    val n = input.read(buffer)
                    if (n < 0) break
                    received += n
                    if (received > release.size) break
                    digest.update(buffer, 0, n)
                    output.write(buffer, 0, n)
                }
            }
        }
        val hex = digest.digest().joinToString("") { "%02x".format(it) }
        if (received != release.size || !hex.equals(release.sha256, ignoreCase = true)) {
            apk.delete()
            return@background false
        }
        commit()
        true
    }

    fun shutdown() = worker.shutdownNow()

    /** Hands the verified APK to PackageInstaller. */
    private fun commit() {
        val installer = context.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL).apply {
            setAppPackageName(context.packageName)
            setSize(apk.length())
            if (Build.VERSION.SDK_INT >= 31) setRequireUserAction(PackageInstaller.SessionParams.USER_ACTION_NOT_REQUIRED)
        }
        val id = installer.createSession(params)
        try {
            installer.openSession(id).use { session ->
                session.openWrite("base.apk", 0, apk.length()).use { out ->
                    apk.inputStream().use { it.copyTo(out) }
                    session.fsync(out)
                }
                val status = Intent(context, UpdateStatusReceiver::class.java)
                // Mutable: the installer adds the status and its confirmation intent.
                val flags = PendingIntent.FLAG_UPDATE_CURRENT or (if (Build.VERSION.SDK_INT >= 31) PendingIntent.FLAG_MUTABLE else 0)
                session.commit(PendingIntent.getBroadcast(context, 0, status, flags).intentSender)
            }
        } catch (e: Exception) {
            installer.abandonSession(id)
            throw e
        } finally {
            apk.delete()
        }
    }

    /** Runs [block] on the worker; any error (network, bad manifest, installer) becomes [failure]. */
    private fun <T> background(failure: T, onResult: (T) -> Unit, block: () -> T) {
        if (worker.isShutdown) return
        worker.execute {
            val result = try {
                block()
            } catch (e: Exception) {
                failure
            }
            main.post { onResult(result) }
        }
    }

    private fun <T> fetch(url: URL, read: (java.io.InputStream) -> T): T {
        val connection = url.openConnection() as HttpURLConnection
        try {
            connection.connectTimeout = 10_000
            connection.readTimeout = 30_000
            connection.useCaches = false
            if (connection.responseCode != HttpURLConnection.HTTP_OK) throw IOException("HTTP ${connection.responseCode}")
            return connection.inputStream.use(read)
        } finally {
            connection.disconnect()
        }
    }

    companion object {
        /** Set by the visible activity to report an install that failed after the download. */
        var onFailure: (() -> Unit)? = null
        private const val MANIFEST_LIMIT = 1 shl 20
        private const val SIZE_LIMIT = 256L shl 20

        /**
         * The Android entry of update.json when its versionCode is above [installedCode]
         * and its native code (`abis`, when listed) runs on this device; URLs resolve against [base].
         */
        fun parse(json: String, installedCode: Long, base: String, deviceAbis: List<String> = emptyList()): Release? {
            val entry = JSONObject(json).optJSONObject("android") ?: return null
            val abis = entry.optJSONArray("abis")?.let { list -> (0 until list.length()).map { list.optString(it) } }
            if (abis != null && deviceAbis.isNotEmpty() && abis.none { it in deviceAbis }) return null
            val code = entry.optLong("code", -1)
            val sha = entry.optString("sha256")
            val url = URL(URL(base), entry.optString("url").ifEmpty { return null })
            if (code <= installedCode || !sha.matches(Regex("[0-9a-fA-F]{64}")) || url.protocol !in setOf("http", "https")) return null
            return Release(entry.optString("version", code.toString()), code, url.toString(), sha, entry.optLong("size", -1))
        }

        fun installedCode(context: Context): Long =
            context.packageManager.getPackageInfo(context.packageName, 0).longVersionCode

    }
}

/** The whole stream, refusing more than [limit] bytes (an IOException). */
fun java.io.InputStream.readLimited(limit: Int): ByteArray {
    val out = java.io.ByteArrayOutputStream()
    val buffer = ByteArray(8192)
    while (true) {
        val n = read(buffer)
        if (n < 0) return out.toByteArray()
        out.write(buffer, 0, n)
        if (out.size() > limit) throw IOException("response is larger than $limit bytes")
    }
}

/**
 * Receives PackageInstaller's result. Not exported, so only the installer (through
 * Updater's PendingIntent) can reach it and the confirmation intent is the system's.
 */
class UpdateStatusReceiver : android.content.BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)) {
            PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                val confirm = if (Build.VERSION.SDK_INT >= 33) {
                    intent.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java)
                } else {
                    @Suppress("DEPRECATION") intent.getParcelableExtra(Intent.EXTRA_INTENT)
                }
                try {
                    context.startActivity(confirm?.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK) ?: return)
                } catch (e: Exception) {
                    Updater.onFailure?.invoke()
                }
            }
            // Success replaces the process; Aborted means the user said no.
            PackageInstaller.STATUS_SUCCESS, PackageInstaller.STATUS_FAILURE_ABORTED -> Unit
            else -> Updater.onFailure?.invoke()
        }
    }
}
