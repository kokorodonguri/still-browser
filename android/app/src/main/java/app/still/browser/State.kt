package app.still.browser

import org.json.JSONArray
import org.json.JSONException
import org.json.JSONObject
import java.time.OffsetDateTime
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter
import java.time.temporal.ChronoUnit

// The saved state uses the desktop build's schema (desktop/DESIGN.md §5) so the
// two stay recognisable; Android-only fields are additions the desktop ignores.

enum class Theme { System, Light, Dark }
enum class Accent { Blue, Purple, Pink, Orange, Green, Graphite }
enum class SidebarStartup { Remember, Open, Closed }
enum class Startup { Restore, NewTab, Home }

enum class SearchEngine(private val base: String) {
    Google("https://www.google.com/search?q="),
    Bing("https://www.bing.com/search?q="),
    DuckDuckGo("https://duckduckgo.com/?q="),
    YahooJapan("https://search.yahoo.co.jp/search?p=");

    /** Results page for an already percent-encoded query. */
    fun searchUrl(encodedQuery: String) = base + encodedQuery
}

data class Settings(
    val theme: Theme = Theme.System,
    val accent: Accent = Accent.Blue,
    val sidebar: SidebarStartup = SidebarStartup.Remember,
    val search: SearchEngine = SearchEngine.Google,
    val startup: Startup = Startup.Restore,
    val homePage: String = "",
    /** Desktop only; kept so the value survives a round trip. */
    val downloadFolder: String = "",
    val askDownloadFolder: Boolean = false,
    val keepHistory: Boolean = true,
    /** File name of the new tab picture in `files/NewTab/` ([NewTabImage]); empty = none. */
    val newTabImage: String = "",
    /** Ask the search engine for suggestions while typing (sends the typed text). */
    val searchSuggestions: Boolean = true,
) {
    /**
     * Applies the fields present in [patch]. Unknown or mistyped values are
     * ignored per field; a home page that is not a web address keeps the old one,
     * and an empty one clears it.
     */
    fun merge(patch: JSONObject): Settings {
        var s = this
        enumField<Theme>(patch, "Theme")?.let { s = s.copy(theme = it) }
        enumField<Accent>(patch, "Accent")?.let { s = s.copy(accent = it) }
        enumField<SidebarStartup>(patch, "Sidebar")?.let { s = s.copy(sidebar = it) }
        enumField<SearchEngine>(patch, "Search")?.let { s = s.copy(search = it) }
        enumField<Startup>(patch, "Startup")?.let { s = s.copy(startup = it) }
        (patch.opt("AskDownloadFolder") as? Boolean)?.let { s = s.copy(askDownloadFolder = it) }
        (patch.opt("KeepHistory") as? Boolean)?.let { s = s.copy(keepHistory = it) }
        (patch.opt("SearchSuggestions") as? Boolean)?.let { s = s.copy(searchSuggestions = it) }
        (patch.opt("HomePage") as? String)?.trim()?.let { home ->
            if (home.isEmpty()) {
                s = s.copy(homePage = "")
            } else {
                Address.webUrl(home)?.takeIf { it.length <= Address.URL_LIMIT }?.let { s = s.copy(homePage = it) }
            }
        }
        (patch.opt("NewTabImage") as? String)?.let { name ->
            if (name.isEmpty() || NewTabImage.isStoredName(name)) s = s.copy(newTabImage = name)
        }
        (patch.opt("DownloadFolder") as? String)?.trim()?.let { folder ->
            if (folder.length <= 1024 && '\u0000' !in folder) s = s.copy(downloadFolder = folder)
        }
        return s
    }

    fun toJson(): JSONObject = JSONObject()
        .put("Theme", theme.name)
        .put("Accent", accent.name)
        .put("Sidebar", sidebar.name)
        .put("Search", search.name)
        .put("Startup", startup.name)
        .put("HomePage", homePage)
        .put("DownloadFolder", downloadFolder)
        .put("AskDownloadFolder", askDownloadFolder)
        .put("KeepHistory", keepHistory)
        .put("NewTabImage", newTabImage)
        .put("SearchSuggestions", searchSuggestions)

    private inline fun <reified T : Enum<T>> enumField(patch: JSONObject, key: String): T? {
        val name = patch.opt(key) as? String ?: return null
        return enumValues<T>().firstOrNull { it.name == name }
    }
}

data class HistoryEntry(val url: String, val visitedAt: String, val title: String)

enum class DownloadState { InProgress, Completed, Failed, Interrupted }

/**
 * [path] is the file's display name inside the public Downloads folder.
 * [id] (Android only; 0 when unknown) is minus the MediaStore row id, or for
 * downloads made by the WebView builds (0.4 and earlier), the DownloadManager id.
 */
data class SavedDownload(
    val path: String,
    val state: DownloadState,
    val bytes: Long = 0,
    val total: Long = 0,
    val id: Long = 0,
)

data class BrowserState(
    val version: Int = SCHEMA_VERSION,
    val sidebarOpen: Boolean = false,
    val tabs: List<String> = emptyList(),
    val activeIndex: Int = 0,
    val bookmarks: List<String> = emptyList(),
    /** Titles of imported bookmarks, used until a visit names the page. */
    val bookmarkTitles: Map<String, String> = emptyMap(),
    val history: List<HistoryEntry> = emptyList(),
    val downloads: List<SavedDownload> = emptyList(),
    val settings: Settings = Settings(),
) {
    fun toJson(): String {
        val root = JSONObject()
            .put("Version", SCHEMA_VERSION)
            .put("SidebarOpen", sidebarOpen)
            .put("Tabs", JSONArray(tabs.map { JSONObject().put("Url", it) }))
            .put("ActiveIndex", activeIndex)
            .put("Bookmarks", JSONArray(bookmarks))
        // Absent unless something was imported, like the desktop build.
        val marked = bookmarks.toHashSet()
        val titles = bookmarkTitles.filterKeys { it in marked }
        if (titles.isNotEmpty()) root.put("BookmarkTitles", JSONObject(titles))
        root
            .put("History", JSONArray(history.map {
                JSONObject().put("Url", it.url).put("VisitedAt", it.visitedAt).put("Title", it.title)
            }))
            .put("Downloads", JSONArray(downloads.map {
                JSONObject().put("Path", it.path).put("State", it.state.name)
                    .put("Bytes", it.bytes).put("Total", it.total).put("Id", it.id)
            }))
            .put("Settings", settings.toJson())
        return root.toString(2)
    }

    companion object {
        const val SCHEMA_VERSION = 2
        const val HISTORY_LIMIT = 500
        const val DOWNLOAD_LIMIT = 200

        /** Parses saved JSON leniently: nulls, wrong types and bad entries are dropped. */
        @Throws(JSONException::class)
        fun parse(json: String): BrowserState {
            val root = JSONObject(json)
            val tabs = objects(root, "Tabs").mapNotNull { (it.opt("Url") as? String)?.takeIf(String::isNotBlank) }
            val seen = HashSet<String>()
            val bookmarks = strings(root, "Bookmarks").filter { it.isNotBlank() && seen.add(it) }
            val titles = (root.opt("BookmarkTitles") as? JSONObject)?.let { o ->
                o.keys().asSequence().filter { it in seen }.mapNotNull { k -> (o.opt(k) as? String)?.takeIf(String::isNotEmpty)?.let { k to it } }.toMap()
            } ?: emptyMap()
            val history = objects(root, "History").mapNotNull {
                val url = it.opt("Url") as? String ?: return@mapNotNull null
                if (url.isBlank()) null else HistoryEntry(url, it.opt("VisitedAt") as? String ?: "", it.opt("Title") as? String ?: "")
            }.takeLast(HISTORY_LIMIT)
            val downloads = trimDownloads(objects(root, "Downloads").mapNotNull {
                val path = it.opt("Path") as? String ?: return@mapNotNull null
                if (path.isBlank() || '\u0000' in path) return@mapNotNull null
                val state = DownloadState.entries.firstOrNull { s -> s.name == it.opt("State") } ?: DownloadState.Interrupted
                SavedDownload(path, state, number(it, "Bytes"), number(it, "Total"), number(it, "Id"))
            })
            val settings = (root.opt("Settings") as? JSONObject)?.let { Settings().merge(it) } ?: Settings()
            val index = (root.opt("ActiveIndex") as? Number)?.toInt() ?: 0
            return BrowserState(
                version = maxOf(1, (root.opt("Version") as? Number)?.toInt() ?: 1),
                sidebarOpen = root.opt("SidebarOpen") as? Boolean ?: false,
                tabs = tabs,
                activeIndex = index.coerceIn(0, maxOf(0, tabs.size - 1)),
                bookmarks = bookmarks,
                bookmarkTitles = titles,
                history = history,
                downloads = downloads,
                settings = settings,
            )
        }

        fun pushHistory(history: List<HistoryEntry>, url: String, title: String, now: String = nowIso8601()) =
            (history + HistoryEntry(url, now, title)).takeLast(HISTORY_LIMIT)

        /** Keeps the newest entries; in-progress downloads are never dropped. */
        fun trimDownloads(downloads: List<SavedDownload>): List<SavedDownload> {
            var excess = downloads.size - DOWNLOAD_LIMIT
            if (excess <= 0) return downloads
            return downloads.filter {
                if (excess > 0 && it.state != DownloadState.InProgress) {
                    excess--
                    false
                } else {
                    true
                }
            }
        }

        fun nowIso8601(): String = OffsetDateTime.now(ZoneOffset.UTC).truncatedTo(ChronoUnit.SECONDS)
            .format(DateTimeFormatter.ISO_OFFSET_DATE_TIME).replace("Z", "+00:00")

        private fun objects(root: JSONObject, key: String): List<JSONObject> {
            val array = root.opt(key) as? JSONArray ?: return emptyList()
            return (0 until array.length()).mapNotNull { array.opt(it) as? JSONObject }
        }

        private fun strings(root: JSONObject, key: String): List<String> {
            val array = root.opt(key) as? JSONArray ?: return emptyList()
            return (0 until array.length()).mapNotNull { array.opt(it) as? String }
        }

        private fun number(o: JSONObject, key: String) = (o.opt(key) as? Number)?.toLong() ?: 0L
    }
}
