package app.still.browser

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.time.OffsetDateTime

class SuggestionsTest {
    private fun visit(url: String, title: String = "", daysAgo: Long = 0) =
        HistoryEntry(url, OffsetDateTime.now().minusDays(daysAgo).toString(), title)

    private fun local(input: String, tabs: Tabs = Tabs().apply { add(Address.BLANK) }, bookmarks: List<String> = emptyList(), history: List<HistoryEntry> = emptyList()) =
        Suggestions.local(input, tabs.list, tabs.activeId, bookmarks, history, { url -> history.lastOrNull { it.url == url }?.title ?: "" })

    @Test
    fun addressesThatStartWithTheInputComeFirst() {
        val history = listOf(
            visit("https://news.example.org/github", "About GitHub"),
            visit("https://github.com/", "GitHub"),
        )
        assertEquals(listOf("https://github.com/", "https://news.example.org/github"), local("git", history = history).map { it.url })
    }

    @Test
    fun everyWordMustMatchTheAddressOrTitle() {
        val history = listOf(visit("https://example.org/a", "Weather in Tokyo"), visit("https://example.org/b", "Weather in Osaka"))
        assertEquals(listOf("https://example.org/a"), local("weather tokyo", history = history).map { it.url })
        assertTrue(local("   ", history = history).isEmpty())
    }

    @Test
    fun queryStringsAndFragmentsDoNotMatch() {
        val history = listOf(
            visit("https://example.org/?ref=github"),
            visit("https://example.net/#github"),
        )
        assertTrue(local("git", history = history).isEmpty())
    }

    @Test
    fun asciiWordsOnlyMatchAtWordStarts() {
        val history = listOf(
            visit("https://agit.example/", "Agit"),
            visit("https://example.org/digital", "Digital"),
            visit("https://example.net/1git", "1git"),
            visit("https://example.com/my-git", "My Git"),
            visit("https://example.edu/", "Using:GitHub"),
        )
        assertEquals(setOf("https://example.com/my-git", "https://example.edu/"), local("git", history = history).map { it.url }.toSet())
    }

    @Test
    fun japaneseTitlesAllowSubstringMatches() {
        val history = listOf(visit("https://example.org/", "明日の東京の天気予報"))
        assertEquals(listOf("https://example.org/"), local("天気", history = history).map { it.url })
    }

    @Test
    fun matchTiersBeatUnboundedFrecency() {
        val history = List(100) { visit("https://example.org/git", "Git") } +
            visit("https://gist.github.com/", "GitHub") + visit("https://github.com/", "GitHub")
        assertEquals(listOf("https://github.com/", "https://gist.github.com/", "https://example.org/git"), local("git", history = history).map { it.url })
    }

    @Test
    fun eachPageAppearsOnceAsItsStrongestKind() {
        val tabs = Tabs().apply {
            add("https://example.org/open")
            add(Address.BLANK)
        }
        val history = listOf(visit("https://example.org/open", "Open"), visit("https://example.org/marked", "Marked"))
        val found = local("example.org/", tabs, listOf("https://example.org/marked"), history)
        assertEquals(setOf(Suggestion.Kind.Tab, Suggestion.Kind.Bookmark), found.map { it.kind }.toSet())
        assertEquals(2, found.size)
        assertEquals(tabs.list[0].id, found.first { it.kind == Suggestion.Kind.Tab }.tabId)
    }

    @Test
    fun theActiveTabIsNotSuggested() {
        val tabs = Tabs().apply { add("https://example.org/") }
        assertTrue(local("example", tabs).isEmpty())
    }

    @Test
    fun pagesVisitedMoreOftenRankHigher() {
        val history = listOf(
            visit("https://a.example/", "Example A"),
            visit("https://b.example/", "Example B"),
            visit("https://a.example/", "Example A"),
            visit("https://a.example/", "Example A"),
        )
        assertEquals("https://a.example/", local("example", history = history).first().url)
    }

    @Test
    fun pagesShareTheirHostsFrecency() {
        val history = listOf(visit("https://frequent.example/guide", "Guide")) +
            List(24) { visit("https://frequent.example/other-$it", "Other") } +
            List(3) { visit("https://occasional.example/guide", "Guide") }
        assertEquals("https://frequent.example/guide", local("guide", history = history).first().url)
    }

    @Test
    fun manyVisitsLastMonthOutweighAFewToday() {
        val history = List(20) { visit("https://regular.example/", "Guide", daysAgo = 30) } +
            List(3) { visit("https://recent.example/", "Guide") }
        assertEquals("https://regular.example/", local("guide", history = history).first().url)
        assertEquals("https://regular.example/", local("guide", history = history.reversed()).first().url)
    }

    @Test
    fun visitsUseEachAgeWeight() {
        // Each older group narrowly wins at the intended weights, but loses if over-decayed.
        for ((newerDays, olderDays, counts) in listOf(
            Triple(3L, 10L, 2 to 3),
            Triple(10L, 25L, 2 to 3),
            Triple(25L, 60L, 1 to 2),
            Triple(60L, 120L, 1 to 4),
        )) {
            val history = List(counts.second) { visit("https://older.example/", "Guide", daysAgo = olderDays) } +
                List(counts.first) { visit("https://newer.example/", "Guide", daysAgo = newerDays) }
            assertEquals("ages $newerDays and $olderDays", "https://older.example/", local("guide", history = history).first().url)
        }
    }

    @Test
    fun unreadableDatesUseTheMinimumWeight() {
        val history = List(9) { HistoryEntry("https://unknown.example/", "invalid", "Guide") } +
            visit("https://recent.example/", "Guide")
        assertEquals("https://recent.example/", local("guide", history = history).first().url)
    }

    @Test
    fun frequentlyVisitedHostsSuggestTheirRootFirst() {
        val history = List(30) { visit("https://youtube.com/watch?v=$it", "Video $it") }
        val found = local("you", history = history)
        assertEquals(Suggestion(Suggestion.Kind.History, "youtube.com", "https://youtube.com/"), found.first())
        assertEquals(Suggestions.LOCAL_LIMIT, found.size)
    }

    @Test
    fun rootsWinTiesWithASingleRepeatedPage() {
        val history = List(30) { visit("https://youtube.com/watch?v=1", "Video") }
        assertEquals("https://youtube.com/", local("you", history = history).first().url)
    }

    @Test
    fun existingRootsKeepTheirTitleAndStrongestKind() {
        val history = listOf(visit("https://youtube.com/", "YouTube"), visit("https://youtube.com/watch?v=1", "Video"))
        val found = local("you", bookmarks = listOf("https://youtube.com/"), history = history)
        assertEquals(Suggestion(Suggestion.Kind.Bookmark, "YouTube", "https://youtube.com/"), found.first())
        assertEquals(1, found.count { it.url == "https://youtube.com/" })
    }

    @Test
    fun onlySearchLikeInputGoesToTheEngine() {
        assertTrue(Suggestions.mayAskEngine("天気"))
        assertTrue(Suggestions.mayAskEngine("example.org"))
        assertFalse(Suggestions.mayAskEngine("https://example.org/private?token=1"))
        assertFalse(Suggestions.mayAskEngine("example.org/account/123"))
        assertFalse(Suggestions.mayAskEngine(""))
        assertFalse(Suggestions.mayAskEngine("a".repeat(101)))
    }

    @Test
    fun engineAddressesEncodeTheInput() {
        assertEquals(
            "https://suggestqueries.google.com/complete/search?client=firefox&ie=utf-8&oe=utf-8&q=%E5%A4%A9%E6%B0%97%20a",
            Suggestions.engineUrl(SearchEngine.Google, " 天気 a "),
        )
        assertEquals("https://api.bing.com/osjson.aspx?query=a%26b", Suggestions.engineUrl(SearchEngine.Bing, "a&b"))
        assertEquals("https://duckduckgo.com/ac/?type=list&q=a", Suggestions.engineUrl(SearchEngine.DuckDuckGo, "a"))
        assertNull(Suggestions.engineUrl(SearchEngine.YahooJapan, "a"))
    }

    @Test
    fun readsOpenSearchAnswers() {
        val body = """["てんき",["天気","天気予報","てんき",42,"天気","   "],[],{}]"""
        assertEquals(listOf("天気", "天気予報"), Suggestions.parseEngine(body, "てんき"))
        assertEquals(listOf("a"), Suggestions.parseEngine("""["x",["a","b"]]""", "x", limit = 1))
        assertTrue(Suggestions.parseEngine("<html>", "x").isEmpty())
        assertTrue(Suggestions.parseEngine("""{"q":1}""", "x").isEmpty())
    }
}
