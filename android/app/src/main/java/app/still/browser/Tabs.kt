package app.still.browser

import org.mozilla.geckoview.GeckoSession

class Tab(val id: Long, var url: String) {
    var title = ""
    var loading = false
    var progress = 0
    var crashed = false
    var canGoBack = false
    var canGoForward = false
    /** The tab whose page opened this one (window.open); Back returns there. */
    var opener: Long? = null
    /** The page, open in the engine. Null until the tab is first shown, and again after it is evicted or crashes. */
    var session: GeckoSession? = null
    /** Back/forward list, kept up to date while the page is open and used to restore it after eviction. */
    var saved: GeckoSession.SessionState? = null
    /** Order of the last time this tab was shown; larger is more recent. */
    var shownAt = 0L

    fun label(): String = title.ifBlank { Address.hostLabel(url) }
}

/**
 * Tab order, selection and which views to keep alive. Holds no Android state of
 * its own, so the rules are unit-tested on the JVM.
 */
class Tabs {
    val list = ArrayList<Tab>()
    private var nextId = 1L
    private var clock = 0L
    var activeId = 0L
        private set

    val active: Tab get() = list.first { it.id == activeId }

    fun find(id: Long): Tab? = list.firstOrNull { it.id == id }

    fun indexOf(id: Long) = list.indexOfFirst { it.id == id }

    /** Adds a tab after [after] (or at the end) and optionally selects it. */
    fun add(url: String, after: Long? = null, select: Boolean = true): Tab {
        val tab = Tab(nextId++, url)
        val at = after?.let(::indexOf)?.takeIf { it >= 0 }?.plus(1) ?: list.size
        list.add(at, tab)
        if (select || list.size == 1) select(tab.id)
        return tab
    }

    fun select(id: Long) {
        val tab = find(id) ?: return
        activeId = id
        tab.shownAt = ++clock
    }

    /**
     * Removes the tab and returns it. When it was active, its opener, else the
     * neighbour that took its place, becomes active. Closing the last tab leaves
     * one blank tab, as on the desktop.
     */
    fun close(id: Long): Tab? {
        val index = indexOf(id)
        if (index < 0) return null
        val tab = list.removeAt(index)
        if (list.isEmpty()) {
            add(Address.BLANK)
        } else if (id == activeId) {
            val next = tab.opener?.let(::find) ?: list[minOf(index, list.size - 1)]
            select(next.id)
        }
        list.forEach { if (it.opener == id) it.opener = null }
        return tab
    }

    /** Background tabs whose views should be released to stay within [maxLive]. */
    fun toEvict(maxLive: Int, hasView: (Tab) -> Boolean = { it.session != null }): List<Tab> {
        val live = list.filter { hasView(it) && it.id != activeId }.sortedByDescending { it.shownAt }
        return live.drop(maxOf(0, maxLive - 1))
    }
}
