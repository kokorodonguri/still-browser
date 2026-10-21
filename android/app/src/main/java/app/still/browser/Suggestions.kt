package app.still.browser

import org.json.JSONArray
import org.json.JSONException
import java.time.Duration
import java.time.Instant
import java.time.OffsetDateTime
import java.time.format.DateTimeParseException
import java.util.Locale

/** A row under the address bar while typing. */
data class Suggestion(val kind: Kind, val title: String, val url: String, val tabId: Long = 0) {
    enum class Kind { Tab, Bookmark, History, Search }
}

/**
 * Suggestions while typing in the address bar: open tabs, bookmarks and
 * history that contain every typed word, and the search engine's own
 * suggestions. Plain logic, so the ranking is unit-tested on the JVM.
 */
object Suggestions {
    const val LOCAL_LIMIT = 4
    const val SEARCH_LIMIT = 4
    /** Longer input is not sent to the search engine. */
    private const val SEARCH_INPUT_LIMIT = 100

    /** The address without scheme and `www.`, as typed most of the time. */
    fun bare(url: String): String = url.substringAfter("://").removePrefix("www.").removeSuffix("/")

    /**
     * Pages whose address or title matches every word of [input], best first:
     * address prefixes beat host labels, then title and path words. Within each
     * tier, visits weighted by age and shared with the host favor familiar sites.
     */
    fun local(
        input: String,
        tabs: List<Tab>,
        activeId: Long,
        bookmarks: List<String>,
        history: List<HistoryEntry>,
        titleFor: (String) -> String,
        limit: Int = LOCAL_LIMIT,
    ): List<Suggestion> {
        val words = input.trim().lowercase(Locale.ROOT).split(Regex("\\s+")).filter { it.isNotEmpty() }
        if (words.isEmpty()) return emptyList()
        val typed = words.joinToString(" ")
        val now = Instant.now()
        val frecency = HashMap<String, Long>()
        val hostFrecency = LinkedHashMap<String, Long>()
        val hostVisits = HashMap<String, Int>()
        for (entry in history) {
            val weight = visitWeight(entry.visitedAt, now)
            frecency[entry.url] = (frecency[entry.url] ?: 0) + weight
            val host = matchingAddress(entry.url).substringBefore('/')
            if (Address.isWeb(entry.url) && host.isNotEmpty()) {
                hostFrecency[host] = (hostFrecency[host] ?: 0) + weight
                hostVisits[host] = (hostVisits[host] ?: 0) + 1
            }
        }
        val candidates = LinkedHashMap<String, Suggestion>()
        for (tab in tabs) {
            if (tab.id != activeId && Address.isWeb(tab.url)) {
                // A tab released before it loaded has no title yet; history may know it.
                candidates.putIfAbsent(tab.url, Suggestion(Suggestion.Kind.Tab, tab.title.ifBlank { titleFor(tab.url) }, tab.url, tab.id))
            }
        }
        for (url in bookmarks) candidates.putIfAbsent(url, Suggestion(Suggestion.Kind.Bookmark, titleFor(url), url))
        val roots = HashMap<String, Long>()
        for ((host, score) in hostFrecency) {
            if (host.startsWith(typed) && (hostVisits[host] ?: 0) > 1) {
                val root = "https://$host/"
                roots[root] = score
                // Insert before history so a root wins ties with its only visited page.
                candidates.putIfAbsent(root, Suggestion(Suggestion.Kind.History, titleFor(root).ifBlank { host }, root))
            }
        }
        for (entry in history.asReversed()) {
            candidates.putIfAbsent(entry.url, Suggestion(Suggestion.Kind.History, titleFor(entry.url), entry.url))
        }
        return candidates.values
            .mapNotNull { s ->
                val address = matchingAddress(s.url)
                val host = address.substringBefore('/')
                val title = s.title.lowercase(Locale.ROOT)
                if (!words.all { matchesWord(address, it) || matchesWord(title, it) }) return@mapNotNull null
                val tier = when {
                    address.startsWith(typed) -> 3
                    host.split('.').any { label -> words.any { label.startsWith(it) } } -> 2
                    else -> 1
                }
                var score = (roots[s.url] ?: frecency[s.url] ?: 0) + (hostFrecency[host] ?: 0) / 4
                score += when (s.kind) {
                    Suggestion.Kind.Tab -> 150
                    Suggestion.Kind.Bookmark -> 100
                    else -> 0
                }
                Triple(s, tier, score)
            }
            .sortedWith(compareByDescending<Triple<Suggestion, Int, Long>> { it.second }.thenByDescending { it.third })
            .take(limit)
            .map { it.first }
    }

    private fun matchingAddress(url: String): String =
        bare(url).substringBefore('?').substringBefore('#').lowercase(Locale.ROOT)

    private fun matchesWord(text: String, word: String): Boolean {
        if (word.any { it.code > 127 }) return word in text
        var at = text.indexOf(word)
        while (at >= 0) {
            if (at == 0 || !isAsciiAlphanumeric(text[at - 1])) return true
            at = text.indexOf(word, at + 1)
        }
        return false
    }

    private fun isAsciiAlphanumeric(c: Char): Boolean = c in 'a'..'z' || c in '0'..'9'

    private fun visitWeight(visitedAt: String, now: Instant): Long = try {
        val days = Duration.between(OffsetDateTime.parse(visitedAt).toInstant(), now).toDays()
        when {
            days <= 4 -> 100L
            days <= 14 -> 70L
            days <= 31 -> 50L
            days <= 90 -> 30L
            else -> 10L
        }
    } catch (e: DateTimeParseException) {
        10L
    }

    /** True when [input] may go to the search engine: not a full web address, which could hold private paths or tokens. */
    fun mayAskEngine(input: String): Boolean {
        val text = input.trim()
        if (text.isEmpty() || text.length > SEARCH_INPUT_LIMIT) return false
        if ("://" in text) return false
        return !(Address.webUrl(text) != null && '/' in text)
    }

    /** Where the engine answers with suggestions (OpenSearch JSON); null when it has no public address for them. */
    fun engineUrl(engine: SearchEngine, input: String): String? {
        val q = Address.encodeComponent(input.trim())
        return when (engine) {
            SearchEngine.Google -> "https://suggestqueries.google.com/complete/search?client=firefox&ie=utf-8&oe=utf-8&q=$q"
            SearchEngine.Bing -> "https://api.bing.com/osjson.aspx?query=$q"
            SearchEngine.DuckDuckGo -> "https://duckduckgo.com/ac/?type=list&q=$q"
            // Yahoo! JAPAN offers suggestions only through a keyed developer API.
            SearchEngine.YahooJapan -> null
        }
    }

    /** The phrases in an OpenSearch answer (`["query", ["one", "two"]]`), without the input itself. */
    fun parseEngine(body: String, input: String, limit: Int = SEARCH_LIMIT): List<String> = try {
        val list = JSONArray(body).getJSONArray(1)
        val typed = input.trim().lowercase(Locale.ROOT)
        (0 until list.length())
            .mapNotNull { list.opt(it) as? String }
            .map { it.trim() }
            .filter { it.isNotEmpty() && it.length <= 200 && it.lowercase(Locale.ROOT) != typed }
            .distinct()
            .take(limit)
    } catch (e: JSONException) {
        emptyList()
    }
}
