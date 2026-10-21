package app.still.browser

/** A destination on the new tab page; [host] also identifies its site. */
data class Shortcut(val url: String, val title: String, val host: String)

/** Derives shortcuts from existing state without adding another saved list. */
object Shortcuts {
    const val LIMIT = 8

    /**
     * Newest bookmarks first, followed by the most visited sites. A site appears
     * once, ignoring scheme, port, `www.` and a trailing DNS dot. Equal visit
     * counts favor the most recent visit, whose page becomes the destination.
     */
    fun select(
        bookmarks: List<String>,
        history: List<HistoryEntry>,
        keepHistory: Boolean = true,
        titleFor: (String) -> String = { "" },
    ): List<Shortcut> {
        val selected = ArrayList<Shortcut>(LIMIT)
        val seen = HashSet<String>()
        for (rawUrl in bookmarks.asReversed()) {
            val page = page(rawUrl) ?: continue
            if (!seen.add(page.host)) continue
            selected += page.copy(title = titleFor(rawUrl).ifBlank { page.host })
            if (selected.size == LIMIT) return selected
        }
        if (!keepHistory) return selected

        val visits = LinkedHashMap<String, Visits>()
        history.forEachIndexed { index, entry ->
            val page = page(entry.url) ?: return@forEachIndexed
            if (page.host in seen) return@forEachIndexed
            val previous = visits[page.host]
            visits[page.host] = Visits((previous?.count ?: 0) + 1, index, page, entry)
        }
        for (site in visits.values.sortedWith(compareByDescending<Visits> { it.count }.thenByDescending { it.lastIndex })) {
            val title = titleFor(site.entry.url).ifBlank { site.entry.title }.ifBlank { site.page.host }
            selected += site.page.copy(title = title)
            if (selected.size == LIMIT) break
        }
        return selected
    }

    private data class Visits(val count: Int, val lastIndex: Int, val page: Shortcut, val entry: HistoryEntry)

    private fun page(rawUrl: String): Shortcut? {
        // Saved state can contain old or imported values, including about:blank.
        val text = rawUrl.trim()
        if (!Address.isWeb(text)) return null
        val url = Address.webUrl(text) ?: return null
        val host = Address.hostLabel(url).removeSuffix(".").removePrefix("www.")
        if (host.isBlank()) return null
        return Shortcut(url, host, host)
    }
}
