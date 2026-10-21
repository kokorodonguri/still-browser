package app.still.browser

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.ByteArrayInputStream
import java.io.File

/** Mirrors desktop/core/src/new_tab_image.rs's tests. */
class NewTabImageTest {
    @get:Rule
    val tmp = TemporaryFolder()

    private val png = byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A) + ByteArray(32)

    @Test
    fun sniffReadsTheContentNotTheName() {
        val samples = mapOf(
            "png" to png,
            "jpg" to byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 0xFF.toByte(), 0xE0.toByte()),
            "gif" to "GIF89a....".toByteArray(),
            "webp" to "RIFF\u0000\u0000\u0000\u0000WEBPVP8 ".toByteArray(),
            "avif" to "\u0000\u0000\u0000\u001cftypavif".toByteArray(),
            "bmp" to "BM".toByteArray() + ByteArray(30),
        )
        for ((extension, bytes) in samples) assertEquals(extension, NewTabImage.sniff(bytes))
        for (bad in listOf("", "<svg/>", "BM", "RIFF\u0000\u0000\u0000\u0000WAVE")) assertNull(bad, NewTabImage.sniff(bad.toByteArray()))
    }

    @Test
    fun storedNamesAreTheOnlyAcceptedNames() {
        for (good in listOf("newtab-1.png", "newtab-1760400000000.jpeg", "newtab-12345678901234567890.avif")) {
            assertEquals(good, true, NewTabImage.isStoredName(good))
        }
        for (bad in listOf("newtab-.png", "newtab-1.svg", "newtab-123456789012345678901.png", "x/newtab-1.png", "newtab-1", "newtab-1a.png", ".newtab-1.png.tmp")) {
            assertEquals(bad, false, NewTabImage.isStoredName(bad))
        }
    }

    @Test
    fun storeWritesAFreshNameAndRejectsOtherFiles() {
        val dir = File(tmp.root, "NewTab")
        val first = NewTabImage.store(dir, png, nowMs = 42)!!
        assertEquals("newtab-42.png", first)
        assertArrayEquals(png, File(dir, first).readBytes())
        val second = NewTabImage.store(dir, png, nowMs = 42)!!
        assertNotEquals("the same millisecond still gets a new name", first, second)
        assertNull(NewTabImage.store(dir, "not a picture".toByteArray()))
        assertNull(NewTabImage.store(dir, png + ByteArray(NewTabImage.MAX_BYTES)))
        assertEquals(setOf(first, second), dir.list()!!.toSet())
    }

    @Test
    fun removeOthersKeepsTheCurrentPictureAndUnrelatedFiles() {
        val dir = tmp.newFolder("NewTab")
        val keep = NewTabImage.store(dir, png, nowMs = 1)!!
        NewTabImage.store(dir, png, nowMs = 2)
        File(dir, "notes.txt").writeText("not Still's")
        assertEquals(1, NewTabImage.removeOthers(dir, keep))
        assertEquals(setOf(keep, "notes.txt"), dir.list()!!.toSet())
        assertEquals(1, NewTabImage.removeOthers(dir, ""))
        assertEquals(0, NewTabImage.removeOthers(File(tmp.root, "missing"), ""))
    }

    @Test
    fun readLimitedRefusesLongStreams() {
        assertArrayEquals(png, NewTabImage.readLimited(ByteArrayInputStream(png)))
        assertNull(NewTabImage.readLimited(ByteArrayInputStream(ByteArray(NewTabImage.MAX_BYTES + 1))))
    }
}
