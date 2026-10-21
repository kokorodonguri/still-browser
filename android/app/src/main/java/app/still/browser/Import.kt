package app.still.browser

import org.json.JSONException
import org.json.JSONObject
import java.time.Instant
import java.time.OffsetDateTime
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter
import java.time.format.DateTimeParseException

// Moving from another browser. Android keeps every app's data to itself, so
// Still reads the files browsers export instead: a bookmarks file (HTML, from
// Chrome, Edge, Firefox or Safari on a computer) and Chrome's history from
// Google Takeout (History.json). The rules match desktop/core/src/import.rs.
// Passwords are filled by Android's autofill service, and cookies cannot be
// read from another app at all.

data class ImportedLink(val url: String, val title: String)

data class ImportedVisit(val url: String, val title: String, val visitedAt: Long)

sealed interface ImportedFile {
    data class Bookmarks(val links: List<ImportedLink>) : ImportedFile
    data class History(val visits: List<ImportedVisit>) : ImportedFile
    /** A password export: Android's autofill service holds passwords, not Still. */
    data object Passwords : ImportedFile
    data object Unrecognized : ImportedFile
}

object Import {
    /** Exported files larger than this are refused before parsing. */
    const val FILE_LIMIT = 64 * 1024 * 1024
    private const val TITLE_LIMIT = 300

    /** Reads at most [FILE_LIMIT] bytes; null when the file is larger. */
    @Throws(java.io.IOException::class)
    fun readLimited(input: java.io.InputStream): ByteArray? {
        val out = java.io.ByteArrayOutputStream()
        val buffer = ByteArray(64 * 1024)
        while (true) {
            val read = input.read(buffer)
            if (read < 0) return out.toByteArray()
            out.write(buffer, 0, read)
            if (out.size() > FILE_LIMIT) return null
        }
    }

    /** Recognizes the file by its contents rather than its name. */
    fun read(bytes: ByteArray): ImportedFile {
        val text = String(bytes, Charsets.UTF_8).trimStart('\uFEFF').trimStart()
        val head = text.take(512).lowercase()
        if (head.startsWith("{")) {
            return takeoutHistory(text)?.let { ImportedFile.History(it) } ?: ImportedFile.Unrecognized
        }
        val html = "netscape-bookmark" in head || listOf("<!doctype", "<html", "<dl", "<meta", "<title").any { head.startsWith(it) }
        if (html) {
            val links = bookmarksHtml(text)
            return if (links.isEmpty()) ImportedFile.Unrecognized else ImportedFile.Bookmarks(links)
        }
        val header = head.lineSequence().firstOrNull().orEmpty()
        return if ("password" in header && ("url" in header || "login_uri" in header)) ImportedFile.Passwords else ImportedFile.Unrecognized
    }

    /** Links from a bookmark file in the Netscape format; one per address, in order. */
    fun bookmarksHtml(text: String): List<ImportedLink> {
        val lower = text.lowercase()
        val links = LinkedHashMap<String, ImportedLink>()
        var at = 0
        while (true) {
            val start = lower.indexOf("<a ", at).takeIf { it >= 0 } ?: break
            val tagEnd = lower.indexOf('>', start).takeIf { it >= 0 } ?: break
            val close = lower.indexOf("</a>", tagEnd).let { if (it < 0) lower.length else it }
            val href = attribute(text.substring(start, tagEnd), "href")
            if (href != null) {
                link(decodeEntities(href), decodeEntities(stripTags(text.substring(tagEnd + 1, close))))?.let { links.putIfAbsent(it.url, it) }
            }
            at = maxOf(close, tagEnd + 1)
        }
        return links.values.toList()
    }

    /** `{"Browser History": [{"title", "url", "time_usec"}]}`; newest first, one per address. */
    fun takeoutHistory(json: String): List<ImportedVisit>? {
        val entries = try {
            JSONObject(json).optJSONArray("Browser History") ?: return null
        } catch (e: JSONException) {
            return null
        }
        val visits = (0 until entries.length()).mapNotNull { i ->
            val entry = entries.optJSONObject(i) ?: return@mapNotNull null
            val l = link(entry.optString("url"), entry.optString("title")) ?: return@mapNotNull null
            ImportedVisit(l.url, l.title, entry.optLong("time_usec") / 1_000_000)
        }
        return visits.sortedByDescending { it.visitedAt }.distinctBy { it.url }
    }

    /** Bookmarks after the existing ones, skipping saved addresses; titles fill in until a visit names the page. */
    fun mergeBookmarks(bookmarks: List<String>, titles: Map<String, String>, links: List<ImportedLink>): Pair<List<String>, Map<String, String>> {
        val known = bookmarks.toHashSet()
        val added = links.filter { it.url.length <= Address.URL_LIMIT && known.add(it.url) }.map { it.url }
        val named = titles.toMutableMap()
        for (l in links) if (l.title.isNotEmpty() && l.url in known) named.putIfAbsent(l.url, l.title)
        return (bookmarks + added) to named
    }

    /**
     * Merges visits into history by time, skipping pages already visited as
     * recently, and keeps the newest [BrowserState.HISTORY_LIMIT] entries.
     * Returns the new history and how many imported visits it holds.
     */
    fun mergeHistory(history: List<HistoryEntry>, visits: List<ImportedVisit>): Pair<List<HistoryEntry>, Int> {
        var time = 0L
        val latest = HashMap<String, Long>()
        val dated = ArrayList<Triple<Long, Boolean, HistoryEntry>>(history.size + visits.size)
        for (entry in history) {
            // An unreadable time keeps the entry where it was.
            time = parseTime(entry.visitedAt) ?: time
            latest[entry.url] = maxOf(latest[entry.url] ?: time, time)
            dated += Triple(time, false, entry)
        }
        for (v in visits) {
            if (v.url.length > Address.URL_LIMIT || (latest[v.url] ?: Long.MIN_VALUE) >= v.visitedAt) continue
            val at = maxOf(0, v.visitedAt)
            dated += Triple(at, true, HistoryEntry(v.url, isoTime(at), v.title))
        }
        val kept = dated.sortedBy { it.first }.takeLast(BrowserState.HISTORY_LIMIT)
        val count = kept.count { it.second }
        return (if (count == 0) history else kept.map { it.third }) to count
    }

    fun parseTime(text: String): Long? = try {
        OffsetDateTime.parse(text.trim(), DateTimeFormatter.ISO_OFFSET_DATE_TIME).toEpochSecond()
    } catch (e: DateTimeParseException) {
        null
    }

    private fun isoTime(seconds: Long) = OffsetDateTime.ofInstant(Instant.ofEpochSecond(seconds), ZoneOffset.UTC)
        .format(DateTimeFormatter.ISO_OFFSET_DATE_TIME).replace("Z", "+00:00")

    private fun link(url: String, title: String): ImportedLink? {
        val web = webUrl(url) ?: return null
        val clean = title.split(Regex("\\s+")).filter { it.isNotEmpty() }.joinToString(" ")
        return ImportedLink(web, if (clean.length > TITLE_LIMIT) clean.substring(0, TITLE_LIMIT) else clean)
    }

    /** Only web pages are imported; `javascript:`, `file:` and the like are dropped. */
    private fun webUrl(raw: String): String? {
        val text = raw.trim()
        if (!text.startsWith("http://", ignoreCase = true) && !text.startsWith("https://", ignoreCase = true)) return null
        return Address.webUrl(text)
    }

    /** The value of [name] in a tag. */
    private fun attribute(tag: String, name: String): String? {
        val lower = tag.lowercase()
        var search = 0
        while (true) {
            val i = lower.indexOf(name, search).takeIf { it >= 0 } ?: return null
            search = i + name.length
            if (i == 0 || !lower[i - 1].isWhitespace()) continue
            val rest = lower.substring(search).trimStart()
            if (!rest.startsWith("=")) continue
            val value = tag.substring(tag.length - rest.length + 1).trimStart()
            return when (val quote = value.firstOrNull()) {
                '"', '\'' -> value.substring(1).substringBefore(quote)
                else -> value.takeWhile { !it.isWhitespace() && it != '>' }
            }
        }
    }

    private fun stripTags(text: String): String {
        val out = StringBuilder(text.length)
        var inside = false
        for (c in text) {
            when {
                c == '<' -> inside = true
                c == '>' -> inside = false
                !inside -> out.append(c)
            }
        }
        return out.toString()
    }

    private val ENTITY = Regex("&(#[xX][0-9a-fA-F]{1,6}|#[0-9]{1,7}|amp|lt|gt|quot|apos|nbsp);")

    private fun decodeEntities(text: String): String = ENTITY.replace(text) { m ->
        when (val e = m.groupValues[1]) {
            "amp" -> "&"
            "lt" -> "<"
            "gt" -> ">"
            "quot" -> "\""
            "apos" -> "'"
            "nbsp" -> " "
            else -> {
                val code = if (e[1] == 'x' || e[1] == 'X') e.substring(2).toIntOrNull(16) else e.substring(1).toIntOrNull()
                if (code != null && Character.isValidCodePoint(code)) String(Character.toChars(code)) else m.value
            }
        }
    }
}
