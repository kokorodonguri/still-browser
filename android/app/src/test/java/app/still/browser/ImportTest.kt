package app.still.browser

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Mirrors desktop/core/src/import.rs and the session's merge rules. */
class ImportTest {
    private fun read(text: String) = Import.read(text.toByteArray())

    @Test
    fun readsExportedBookmarkHtml() {
        val html = """<!DOCTYPE NETSCAPE-Bookmark-file-1>
<META HTTP-EQUIV="Content-Type" CONTENT="text/html; charset=UTF-8">
<DL><p>
    <DT><H3>Bookmarks bar</H3>
    <DL><p>
        <DT><A HREF="https://example.org/a?x=1&amp;y=2" ADD_DATE="1" ICON="data:image/png;base64,AAA">Example &amp; &#12354;</A>
        <DT><a href='https://example.org/b'>  Two
           lines </a>
        <DT><A HREF="javascript:alert(1)">Bookmarklet</A>
        <DT><A HREF="https://example.org/a?x=1&amp;y=2">Duplicate</A>
        <DT><A ADD_DATE="2" HREF=https://unquoted.test/>Unquoted</A>
    </DL><p>
</DL><p>"""
        assertEquals(
            ImportedFile.Bookmarks(listOf(
                ImportedLink("https://example.org/a?x=1&y=2", "Example & あ"),
                ImportedLink("https://example.org/b", "Two lines"),
                ImportedLink("https://unquoted.test/", "Unquoted"),
            )),
            read("\uFEFF" + html),
        )
    }

    @Test
    fun readsTakeoutHistoryNewestFirst() {
        val json = """{"Browser History":[
            {"title":"Old","url":"https://a.test/","time_usec":1700000000000000},
            {"title":"New","url":"https://a.test/","time_usec":1800000000000000},
            {"title":"B","url":"https://b.test/","time_usec":1750000000000000},
            {"title":"Settings","url":"chrome://settings/","time_usec":1900000000000000}]}"""
        val file = read(json) as ImportedFile.History
        assertEquals(listOf("New" to 1_800_000_000L, "B" to 1_750_000_000L), file.visits.map { it.title to it.visitedAt })
        assertEquals(ImportedFile.Unrecognized, read("""{"other":1}"""))
    }

    @Test
    fun passwordFilesAndOtherFilesAreRecognized() {
        assertEquals(ImportedFile.Passwords, read("name,url,username,password,note\nx,https://a.test/,me,pw,\n"))
        assertEquals(ImportedFile.Passwords, read("\"url\",\"username\",\"password\",\"httpRealm\"\n"))
        assertEquals(ImportedFile.Unrecognized, read("just,some,columns\n1,2,3"))
        assertEquals(ImportedFile.Unrecognized, read("<html><body>no links</body></html>"))
    }

    @Test
    fun bookmarksMergeAfterTheExistingOnesWithTheirTitles() {
        val (bookmarks, titles) = Import.mergeBookmarks(
            listOf("https://a.test/"),
            emptyMap(),
            listOf(ImportedLink("https://a.test/", "A named"), ImportedLink("https://b.test/", "B"), ImportedLink("https://b.test/", "B again")),
        )
        assertEquals(listOf("https://a.test/", "https://b.test/"), bookmarks)
        assertEquals(mapOf("https://a.test/" to "A named", "https://b.test/" to "B"), titles)
    }

    @Test
    fun historyMergesByTimeAndSkipsNewerVisits() {
        val history = listOf(
            HistoryEntry("https://old.test/", "2026-01-01T09:00:00+09:00", "Old"),
            HistoryEntry("https://a.test/", "2026-03-01T00:00:00+00:00", ""),
        )
        val jan = Import.parseTime("2026-01-15T00:00:00+00:00")!!
        val (merged, count) = Import.mergeHistory(history, listOf(
            ImportedVisit("https://between.test/", "Between", jan),
            ImportedVisit("https://a.test/", "A", jan),
            ImportedVisit("https://new.test/", "New", jan + 90 * 86_400),
        ))
        assertEquals(2, count)
        assertEquals(listOf("https://old.test/", "https://between.test/", "https://a.test/", "https://new.test/"), merged.map { it.url })
        assertEquals("2026-01-15T00:00:00+00:00", merged[1].visitedAt)
        assertEquals(0, Import.mergeHistory(merged, listOf(ImportedVisit("https://new.test/", "New", jan))).second)
    }

    @Test
    fun importedHistoryKeepsTheNewestEntries() {
        val visits = (0 until 600).map { ImportedVisit("https://e.test/$it", "", 1_700_000_000L + it) }
        val (merged, count) = Import.mergeHistory(emptyList(), visits)
        assertEquals(BrowserState.HISTORY_LIMIT, count)
        assertEquals("https://e.test/100", merged.first().url)
        assertEquals("https://e.test/599", merged.last().url)
    }

    @Test
    fun bookmarkTitlesRoundTripOnlyForSavedBookmarks() {
        val state = BrowserState.parse("""{"Bookmarks":["https://a.test/"],"BookmarkTitles":{"https://a.test/":"A","https://gone.test/":"G","https://a.test/x":5}}""")
        assertEquals(mapOf("https://a.test/" to "A"), state.bookmarkTitles)
        assertTrue(state.toJson().contains("\"BookmarkTitles\""))
        assertEquals(state.copy(version = BrowserState.SCHEMA_VERSION), BrowserState.parse(state.toJson()))
        assertFalse(BrowserState().toJson().contains("BookmarkTitles"))
        assertTrue(BrowserState.parse("""{"BookmarkTitles":"x"}""").bookmarkTitles.isEmpty())
    }
}
