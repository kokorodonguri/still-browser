package app.still.browser

import org.json.JSONException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class UpdaterTest {
    private val base = "https://still.dongurihub.com/update.json"
    private val sha = "ab".repeat(32)
    private fun manifest(code: Long, url: String = "dl/StillBrowser-Android.apk", hash: String = sha) =
        """{"android": {"version": "0.3.0", "code": $code, "url": "$url", "sha256": "$hash", "size": 1234}, "linux": {}}"""

    @Test
    fun offersOnlyAHigherVersionCode() {
        val release = Updater.parse(manifest(2), installedCode = 1, base)
        assertEquals(Updater.Release("0.3.0", 2, "https://still.dongurihub.com/dl/StillBrowser-Android.apk", sha, 1234), release)
        assertNull(Updater.parse(manifest(1), installedCode = 1, base))
        assertNull(Updater.parse(manifest(1), installedCode = 5, base))
    }

    @Test
    fun offersOnlyAnApkThisDeviceCanRun() {
        val arm = """{"android": {"code": 2, "url": "a.apk", "sha256": "$sha", "abis": ["arm64-v8a"]}}"""
        assertEquals(2L, Updater.parse(arm, installedCode = 1, base, listOf("arm64-v8a", "armeabi-v7a"))?.code)
        assertNull(Updater.parse(arm, installedCode = 1, base, listOf("armeabi-v7a", "armeabi")))
        assertEquals("entries without abis are for every device", 2L, Updater.parse(manifest(2), installedCode = 1, base, listOf("x86"))?.code)
    }

    @Test
    fun rejectsIncompleteOrUnsafeEntries() {
        assertNull(Updater.parse("{}", installedCode = 1, base))
        assertNull(Updater.parse("""{"android": {"code": 2}}""", installedCode = 1, base))
        assertNull(Updater.parse(manifest(2, hash = "not-a-hash"), installedCode = 1, base))
        assertNull(Updater.parse(manifest(2, url = "file:///sdcard/evil.apk"), installedCode = 1, base))
    }

    @Test(expected = JSONException::class)
    fun aBrokenManifestIsAnError() {
        Updater.parse("<html>", installedCode = 1, base)
    }
}
