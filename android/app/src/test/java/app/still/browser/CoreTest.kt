package app.still.browser

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File

/** Mirrors desktop/core's tests so both builds treat addresses and saved data alike. */
class AddressTest {
    @Test
    fun normalizesAddressesLikeTheDesktopBuild() {
        assertEquals(Address.BLANK, Address.normalize("  "))
        assertEquals("https://example.org/", Address.normalize("https://example.org"))
        assertEquals("https://example.org/path", Address.normalize("example.org/path"))
        assertEquals("https://www.google.com/search?q=still%20browser", Address.normalize("still browser"))
        assertEquals("https://www.google.com/search?q=%E6%97%A5%E6%9C%AC%E8%AA%9E", Address.normalize("日本語"))
        assertEquals("https://www.google.com/search?q=javascript%3Aalert%281%29", Address.normalize("javascript:alert(1)"))
        assertTrue(Address.normalize("file:///etc/passwd").startsWith("https://www.google.com/"))
        assertEquals("https://duckduckgo.com/?q=still%20browser", Address.normalize("still browser", SearchEngine.DuckDuckGo))
        assertEquals("https://example.org/", Address.normalize("example.org", SearchEngine.Bing))
    }

    @Test
    fun canonicalizesHostsPortsAndPaths() {
        assertEquals("https://example.org/a%20b?q=1#x", Address.normalize("HTTPS://Example.ORG:443/a b?q=1#x"))
        assertEquals("http://example.org:8080/", Address.normalize("http://example.org:8080"))
        assertEquals("https://xn--r8jz45g.jp/", Address.normalize("例え.jp"))
        assertEquals("https://example.org/?q", Address.normalize("example.org?q"))
        assertEquals("http://[::1]:3000/", Address.normalize("http://[::1]:3000"))
        assertEquals("https://example.org/%E3%81%82", Address.normalize("https://example.org/あ"))
        assertEquals("https://example.org/%F0%9F%98%80", Address.normalize("https://example.org/😀"))
    }

    @Test
    fun rejectsThingsThatAreNotWebAddresses() {
        assertNull(Address.webUrl("just words"))
        assertNull(Address.webUrl("localhost"))
        assertNull(Address.webUrl("a..b"))
        assertNull(Address.webUrl("https://exa mple.org"))
        assertNull(Address.webUrl("https://example.org:99999"))
        assertNull(Address.webUrl("ftp://example.org"))
        assertNull(Address.webUrl("https://"))
        assertNull(Address.webUrl("x".repeat(Address.URL_LIMIT) + ".org"))
    }

    @Test
    fun labelsTabsByHost() {
        assertEquals("example.org", Address.hostLabel("https://example.org/a"))
        assertEquals("", Address.hostLabel(Address.BLANK))
    }

    @Test
    fun searchEnginesBuildResultUrls() {
        assertEquals("https://www.bing.com/search?q=a%20b", SearchEngine.Bing.searchUrl("a%20b"))
        assertEquals("https://duckduckgo.com/?q=a", SearchEngine.DuckDuckGo.searchUrl("a"))
        assertEquals("https://search.yahoo.co.jp/search?p=a", SearchEngine.YahooJapan.searchUrl("a"))
    }
}

class StateTest {
    @Test
    fun readsStateWrittenByTheDesktopAndCSharpBuilds() {
        val json = """{
          "SidebarOpen": true,
          "Tabs": [{"Url":"https://example.org/"}, null, {"Url":"https://a.test/"}],
          "ActiveIndex": 7,
          "Bookmarks": ["https://example.org/", "", "https://example.org/"],
          "History": [{"Url":"https://example.org/","VisitedAt":"2026-10-09T05:00:00+09:00","Title":"Example"}],
          "Downloads": [{"Path":"C:\\Users\\a\\Downloads\\x.zip","State":"InProgress","Bytes":10,"Total":20}],
          "Window": {"X": 1, "Y": 2, "Width": 800, "Height": 600}
        }"""
        val state = BrowserState.parse(json)
        assertEquals("files without a version come from the C# build", 1, state.version)
        assertTrue(state.sidebarOpen)
        assertEquals(listOf("https://example.org/", "https://a.test/"), state.tabs)
        assertEquals(1, state.activeIndex)
        assertEquals(listOf("https://example.org/"), state.bookmarks)
        assertEquals("Example", state.history[0].title)
        assertEquals(DownloadState.InProgress, state.downloads[0].state)
        val round = BrowserState.parse(state.toJson())
        assertEquals(state.copy(version = BrowserState.SCHEMA_VERSION), round)
        assertTrue(state.toJson().contains("\"SidebarOpen\": true"))
    }

    @Test
    fun toleratesNullsMissingFieldsAndNegativeIndex() {
        val state = BrowserState.parse("""{"Tabs":null,"ActiveIndex":-3,"History":null,"Window":{"Width":10}}""")
        assertEquals(BrowserState(version = 1), state)
    }

    @Test(expected = org.json.JSONException::class)
    fun rejectsJunk() {
        BrowserState.parse("not json")
    }

    @Test
    fun historyKeepsTheLatest500() {
        var history = emptyList<HistoryEntry>()
        for (i in 0 until 520) history = BrowserState.pushHistory(history, "https://e.test/$i", "")
        assertEquals(BrowserState.HISTORY_LIMIT, history.size)
        assertEquals("https://e.test/20", history[0].url)
    }

    @Test
    fun downloadsAreCappedWithoutDroppingActiveOnes() {
        val downloads = (0 until BrowserState.DOWNLOAD_LIMIT + 5).map {
            SavedDownload("f$it", if (it == 0) DownloadState.InProgress else DownloadState.Completed)
        }
        val trimmed = BrowserState.trimDownloads(downloads)
        assertEquals(BrowserState.DOWNLOAD_LIMIT, trimmed.size)
        assertEquals("f0", trimmed[0].path)
        assertEquals("f6", trimmed[1].path)
    }

    @Test
    fun timestampsMatchTheDesktopFormat() {
        assertTrue(Regex("""\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\+00:00""").matches(BrowserState.nowIso8601()))
    }
}

class SettingsTest {
    @Test
    fun unknownOrMistypedValuesFallBackPerField() {
        val s = Settings().merge(JSONObject("""{"Theme":"Neon","Accent":"Pink","Search":3,"KeepHistory":"yes","Startup":"Home","Extra":true}"""))
        assertEquals(Settings(accent = Accent.Pink, startup = Startup.Home), s)
        val state = BrowserState.parse("""{"Settings":"not an object"}""")
        assertEquals(Settings(), state.settings)
    }

    @Test
    fun mergeSanitizesTheHomePageAndFolder() {
        var s = Settings().merge(JSONObject("""{"HomePage":" example.org ","DownloadFolder":"  D:\\Files  "}"""))
        assertEquals("https://example.org/" to "D:\\Files", s.homePage to s.downloadFolder)
        s = s.merge(JSONObject("""{"HomePage":"just words"}"""))
        assertEquals("search text is not a home page; the old one stays", "https://example.org/", s.homePage)
        s = s.merge(JSONObject().put("HomePage", "javascript:alert(1)").put("DownloadFolder", "a\u0000b"))
        assertEquals("https://example.org/" to "D:\\Files", s.homePage to s.downloadFolder)
        s = s.merge(JSONObject("""{"Theme":"Dark"}"""))
        assertEquals("absent fields are untouched", Theme.Dark, s.theme)
        s = s.merge(JSONObject("""{"HomePage":" ","DownloadFolder":""}"""))
        assertEquals("empty clears", "" to "", s.homePage to s.downloadFolder)
        assertEquals("", BrowserState.parse("""{"Settings":{"HomePage":"not a url"}}""").settings.homePage)
    }

    @Test
    fun newTabImageAcceptsOnlyStoredPictureNames() {
        var s = Settings().merge(JSONObject("""{"NewTabImage":"newtab-1760400000000.jpg"}"""))
        assertEquals("newtab-1760400000000.jpg", s.newTabImage)
        for (bad in listOf("../state.json", "/sdcard/x.png", "newtab-.png", "newtab-12.svg", "newtab-12.png/x", "picture.png")) {
            s = s.merge(JSONObject().put("NewTabImage", bad))
            assertEquals("$bad is rejected", "newtab-1760400000000.jpg", s.newTabImage)
        }
        assertEquals("newtab-1760400000000.jpg", BrowserState.parse(BrowserState(settings = s).toJson()).settings.newTabImage)
        assertEquals("empty removes the picture", "", s.merge(JSONObject("""{"NewTabImage":""}""")).newTabImage)
    }
}

class StateStoreTest {
    @get:Rule
    val tmp = TemporaryFolder()

    private fun file() = File(tmp.root, "state.json")

    @Test
    fun missingFileStartsFresh() {
        val (state, problem) = StateStore(file()).load()
        assertEquals(BrowserState(), state)
        assertNull(problem)
    }

    @Test
    fun savesAtomicallyAndReadsBack() {
        val store = StateStore(file())
        val state = BrowserState(tabs = listOf("https://example.org/"), bookmarks = listOf("https://a.test/"))
        assertTrue(store.save(state.toJson()))
        assertEquals(state, StateStore(file()).load().first)
        assertFalse(File(tmp.root, "state.json.tmp").exists())
    }

    @Test
    fun corruptFileIsBackedUpBeforeTheFirstOverwrite() {
        file().writeText("{ broken")
        val store = StateStore(file())
        assertEquals(StateStore.LoadProblem.Corrupt, store.load().second)
        assertTrue(store.save(BrowserState().toJson()))
        val backups = tmp.root.listFiles()!!.filter { it.name.startsWith("state.json.invalid-") }
        assertEquals(1, backups.size)
        assertEquals("{ broken", backups[0].readText())
    }

    @Test
    fun newerFileIsBackedUpUnderAUniqueName() {
        file().writeText("""{"Version": 9, "Tabs":[{"Url":"https://new.test/"}]}""")
        File(tmp.root, "state.json.v9-backup").writeText("older backup")
        val store = StateStore(file())
        val (state, problem) = store.load()
        assertEquals(StateStore.LoadProblem.Newer, problem)
        assertEquals(listOf("https://new.test/"), state.tabs)
        assertTrue(store.save(state.toJson()))
        assertEquals("older backup", File(tmp.root, "state.json.v9-backup").readText())
        assertTrue(File(tmp.root, "state.json.v9-backup-1").readText().contains("\"Version\": 9"))
    }

    @Test
    fun failedBackupBlocksTheOverwrite() {
        file().writeText("{ broken")
        val store = StateStore(file())
        store.load()
        tmp.root.setWritable(false)
        try {
            assertFalse(store.save(BrowserState().toJson()))
        } finally {
            tmp.root.setWritable(true)
        }
        assertEquals("the original is untouched", "{ broken", file().readText())
        assertTrue("retried once the folder is writable", store.save(BrowserState().toJson()))
        assertEquals(1, tmp.root.listFiles()!!.count { it.name.startsWith("state.json.invalid-") })
    }
}

class TabsTest {
    @Test
    fun closingTheLastTabLeavesABlankOne() {
        val tabs = Tabs()
        val only = tabs.add("https://example.org/")
        tabs.close(only.id)
        assertEquals(1, tabs.list.size)
        assertEquals(Address.BLANK, tabs.active.url)
    }

    @Test
    fun closingTheActiveTabSelectsItsOpenerElseTheNeighbour() {
        val tabs = Tabs()
        val a = tabs.add("https://a.test/")
        val b = tabs.add("https://b.test/")
        val c = tabs.add("https://c.test/", after = a.id)
        c.opener = b.id
        assertEquals(listOf(a.id, c.id, b.id), tabs.list.map { it.id })
        tabs.close(c.id)
        assertEquals(b.id, tabs.activeId)
        tabs.select(a.id)
        tabs.close(a.id)
        assertEquals(b.id, tabs.activeId)
    }

    @Test
    fun backgroundTabsAreAddedWithoutSelection() {
        val tabs = Tabs()
        val a = tabs.add("https://a.test/")
        tabs.add("https://b.test/", select = false)
        assertEquals(a.id, tabs.activeId)
    }

    @Test
    fun evictsTheLeastRecentlyShownViewsFirst() {
        val tabs = Tabs()
        val t = (0 until 6).map { tabs.add("https://$it.test/") }
        val live = t.map { it.id }.toMutableSet()
        tabs.select(t[1].id)
        tabs.select(t[4].id)
        tabs.select(t[2].id)
        // Recency: 2 (active), 4, 1, 5, 3, 0. Keeping three live pages keeps 2, 4 and 1.
        val evicted = tabs.toEvict(3) { it.id in live }.map { it.id }
        assertEquals(listOf(t[5].id, t[3].id, t[0].id), evicted)
        live.removeAll(evicted.toSet())
        assertEquals(emptyList<Tab>(), tabs.toEvict(3) { it.id in live })
        assertEquals("the active page is never evicted", listOf(t[4].id, t[1].id), tabs.toEvict(1) { it.id in live }.map { it.id })
    }
}
