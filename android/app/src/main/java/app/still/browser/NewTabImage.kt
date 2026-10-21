package app.still.browser

import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.io.InputStream

/**
 * The picture shown behind a new tab, as in the desktop build
 * (desktop/core/src/new_tab_image.rs). The chosen file is copied into
 * `files/NewTab/` under a name Still generates, and settings keep only that
 * name, so a saved setting can never point at another file.
 */
object NewTabImage {
    /** Larger files are refused before anything is written. */
    const val MAX_BYTES = 20 * 1024 * 1024
    private val EXTENSIONS = setOf("png", "jpg", "jpeg", "webp", "gif", "avif", "bmp")

    /** True for the names [store] gives (`newtab-<digits>.<ext>`). Anything else, including a path, is not one. */
    fun isStoredName(name: String): Boolean {
        val stem = name.substringBeforeLast('.', "")
        val digits = stem.removePrefix("newtab-")
        return digits != stem && digits.length in 1..20 && digits.all { it in '0'..'9' } &&
            name.substringAfterLast('.') in EXTENSIONS
    }

    /** The format from the first bytes; the file name is not trusted. */
    fun sniff(bytes: ByteArray): String? {
        fun starts(vararg magic: Int, at: Int = 0) =
            bytes.size >= at + magic.size && magic.indices.all { bytes[at + it] == magic[it].toByte() }
        fun ascii(text: String, at: Int = 0) = starts(*text.map { it.code }.toIntArray(), at = at)
        return when {
            starts(0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A) -> "png"
            starts(0xFF, 0xD8, 0xFF) -> "jpg"
            ascii("GIF87a") || ascii("GIF89a") -> "gif"
            ascii("RIFF") && ascii("WEBP", at = 8) -> "webp"
            ascii("ftypavif", at = 4) || ascii("ftypavis", at = 4) -> "avif"
            ascii("BM") && bytes.size > 26 -> "bmp"
            else -> null
        }
    }

    /** Reads at most [MAX_BYTES]; null when the stream is longer. */
    @Throws(IOException::class)
    fun readLimited(input: InputStream): ByteArray? {
        val out = java.io.ByteArrayOutputStream()
        val buffer = ByteArray(64 * 1024)
        while (true) {
            val read = input.read(buffer)
            if (read < 0) return out.toByteArray()
            out.write(buffer, 0, read)
            if (out.size() > MAX_BYTES) return null
        }
    }

    /**
     * Writes [bytes] into [dir] under a fresh name and returns it, or null when
     * they are not a supported picture. The file is complete before the name is
     * returned; nothing else is touched.
     */
    @Throws(IOException::class)
    fun store(dir: File, bytes: ByteArray, nowMs: Long = System.currentTimeMillis()): String? {
        if (bytes.size > MAX_BYTES) return null
        val extension = sniff(bytes) ?: return null
        if (!dir.isDirectory && !dir.mkdirs()) throw IOException("Cannot create $dir")
        // Two pictures chosen within the same millisecond still get distinct names.
        var stamp = nowMs
        while (File(dir, "newtab-$stamp.$extension").exists()) stamp++
        val name = "newtab-$stamp.$extension"
        val temporary = File(dir, ".$name.tmp")
        try {
            FileOutputStream(temporary).use {
                it.write(bytes)
                it.fd.sync()
            }
            if (!temporary.renameTo(File(dir, name))) throw IOException("Cannot save $name")
        } finally {
            temporary.delete()
        }
        return name
    }

    /** Deletes stored pictures other than [keep] (empty keeps none); other files are left alone. */
    fun removeOthers(dir: File, keep: String): Int =
        dir.listFiles().orEmpty().count { it.name != keep && isStoredName(it.name) && it.delete() }
}
