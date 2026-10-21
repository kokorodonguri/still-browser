package app.still.browser

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ShortcutsTest {
    private fun visit(url: String, title: String = "") = HistoryEntry(url, "2026-10-15T00:00:00+00:00", title)

    @Test
    fun bookmarksAreNewestFirstAndTakePriorityOverFrequentHistory() {
        val old = "https://old.example/saved"
        val newest = "https://new.example/bookmark"
        val history = List(10) { visit("https://popular.example/") } + visit("https://new.example/visited")
        val found = Shortcuts.select(listOf(old, newest), history) { if (it == newest) "Saved title" else "" }

        assertEquals(listOf(newest, old, "https://popular.example/"), found.map { it.url })
        assertEquals("Saved title", found.first().title)
        assertEquals("old.example", found[1].title)
    }

    @Test
    fun visitsAreCountedAcrossPagesAndMostRecentPageRepresentsEachHost() {
        val history = listOf(
            visit("https://frequent.example/first"),
            visit("https://other.example/first"),
            visit("https://frequent.example/second"),
            visit("https://frequent.example/latest", "Frequent site"),
            visit("https://other.example/latest", "Other site"),
            visit("https://recent.example/latest", "Most recent"),
        )
        val found = Shortcuts.select(emptyList(), history)

        assertEquals(listOf("frequent.example", "other.example", "recent.example"), found.map { it.host })
        assertEquals("https://frequent.example/latest", found.first().url)
        assertEquals("Frequent site", found.first().title)
    }

    @Test
    fun equalVisitCountsFavorTheMostRecentHost() {
        val found = Shortcuts.select(emptyList(), listOf(visit("https://older.example/"), visit("https://newer.example/")))
        assertEquals(listOf("newer.example", "older.example"), found.map { it.host })
    }

    @Test
    fun hostNormalizationDeduplicatesBookmarksAndHistory() {
        val found = Shortcuts.select(
            listOf("http://WWW.Example.COM:80/old", "https://example.com.:8443/new"),
            listOf(visit("https://www.example.com/history"), visit("https://other.example/")),
        )

        assertEquals(listOf("example.com", "other.example"), found.map { it.host })
        assertEquals("https://example.com.:8443/new", found.first().url)
    }

    @Test
    fun internationalHostsUseTheSameNormalizedIdentity() {
        val found = Shortcuts.select(listOf("https://例え.jp/old", "https://xn--r8jz45g.jp/new"), emptyList())
        assertEquals(1, found.size)
        assertEquals("xn--r8jz45g.jp", found.single().host)
    }

    @Test
    fun blankAndInvalidAddressesDoNotTakeShortcutSlots() {
        val bad = listOf("", "  ", Address.BLANK, "file:///private", "javascript:alert(1)", "example.org", "https://", "https://bad host/", "http://example.org:99999/")
        assertTrue(Shortcuts.select(bad, bad.map(::visit)).isEmpty())
        assertEquals("valid.example", Shortcuts.select(bad + "https://valid.example/", emptyList()).single().host)
    }

    @Test
    fun historyDisabledShowsOnlyBookmarksEvenIfOldHistoryRemains() {
        val history = List(12) { visit("https://history$it.example/", "Private title") }
        val found = Shortcuts.select(listOf("https://saved.example/"), history, keepHistory = false)
        assertEquals(listOf("saved.example"), found.map { it.host })
        assertTrue(Shortcuts.select(emptyList(), history, keepHistory = false).isEmpty())
    }

    @Test
    fun eightBookmarksFillAllSlotsAndHistoryFillsOnlyTheRemainingSlots() {
        val bookmarks = (0..9).map { "https://bookmark$it.example/" }
        val history = (0..9).map { visit("https://history$it.example/") }
        assertEquals(bookmarks.asReversed().take(8), Shortcuts.select(bookmarks, history).map { it.url })

        val mixed = Shortcuts.select(bookmarks.take(3), history)
        assertEquals(Shortcuts.LIMIT, mixed.size)
        assertEquals(bookmarks.take(3).asReversed(), mixed.take(3).map { it.url })
        assertEquals(history.asReversed().take(5).map { it.url }, mixed.drop(3).map { it.url })
    }
}
