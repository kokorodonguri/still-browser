package app.still.browser

import android.app.Activity
import android.app.AlertDialog
import android.app.SearchManager
import android.app.UiModeManager
import android.content.ActivityNotFoundException
import android.content.ComponentCallbacks2
import android.content.Intent
import android.content.res.ColorStateList
import android.content.res.Configuration
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.ImageDecoder
import android.graphics.Paint
import android.graphics.drawable.BitmapDrawable
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.MediaStore
import android.text.TextUtils
import android.util.Patterns
import android.view.Gravity
import android.view.KeyEvent
import android.view.View
import android.view.ViewGroup.LayoutParams.MATCH_PARENT
import android.view.ViewGroup.LayoutParams.WRAP_CONTENT
import android.view.WindowInsets
import android.view.WindowInsetsController
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputMethodManager
import android.text.Editable
import android.text.TextWatcher
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.ImageButton
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import org.mozilla.geckoview.AllowOrDeny
import org.mozilla.geckoview.BasicSelectionActionDelegate
import org.mozilla.geckoview.GeckoResult
import org.mozilla.geckoview.GeckoSession
import org.mozilla.geckoview.GeckoSession.ContentDelegate
import org.mozilla.geckoview.GeckoSession.NavigationDelegate
import org.mozilla.geckoview.GeckoSession.PermissionDelegate
import org.mozilla.geckoview.GeckoSession.ProgressDelegate
import org.mozilla.geckoview.GeckoSession.PromptDelegate.FilePrompt
import org.mozilla.geckoview.GeckoSession.PromptDelegate.PromptResponse
import org.mozilla.geckoview.GeckoView
import org.mozilla.geckoview.SlowScriptResponse
import org.mozilla.geckoview.StorageController
import org.mozilla.geckoview.WebResponse
import java.io.File
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL
import java.nio.ByteBuffer
import java.util.Locale
import java.util.concurrent.Executors
import kotlin.math.roundToInt

class MainActivity : Activity() {
    val tabs = Tabs()
    var bookmarks: List<String> = emptyList(); private set
    /** Titles of imported bookmarks ([BrowserState.bookmarkTitles]). */
    private var bookmarkTitles: Map<String, String> = emptyMap()
    var history: List<HistoryEntry> = emptyList(); private set
    var downloads: List<SavedDownload> = emptyList(); private set
    var settings = Settings(); private set
    /** Desktop-only field, kept so the saved file round-trips. */
    private var sidebarOpen = false

    private lateinit var store: StateStore
    private lateinit var downloader: Downloads
    private lateinit var updater: Updater
    /** True from an update check or download until its result arrives. */
    private var updating = false
    private val updateFailed = { if (!isDestroyed) showNotice(getString(R.string.update_failed)) }
    /** An update waiting for the user to allow Still to install apps. */
    private var awaitingInstallPermission: Updater.Release? = null
    private val writer = Executors.newSingleThreadExecutor()
    /** Reads, decodes and deletes new tab pictures off the main thread. */
    private val pictures = Executors.newSingleThreadExecutor()
    private val pictureDir by lazy { File(filesDir, "NewTab") }
    /** Asks the search engine for suggestions, one request at a time. */
    private val suggestionFetcher = Executors.newSingleThreadExecutor()
    /** The decoded [Settings.newTabImage]; null when none is chosen or it is still loading. */
    var newTabPicture: Bitmap? = null; private set
    private val main = Handler(Looper.getMainLooper())
    private var saveFailedShown = false
    private var resumed = false

    lateinit var palette: Palette; private set
    private lateinit var root: FrameLayout
    private lateinit var frame: FrameLayout
    private lateinit var content: LinearLayout
    private lateinit var pageHost: FrameLayout
    /** Shows the active tab's page; kept across interface rebuilds. */
    private lateinit var geckoView: GeckoView
    private lateinit var prompts: Prompts
    private lateinit var blank: ImageView
    private lateinit var home: HomeView
    private val thumbnails = HashMap<Long, Bitmap>()
    private val captureRequests = HashMap<Long, Long>()
    private var captureSerial = 0L
    private var menu: MenuSheet? = null
    private lateinit var chrome: LinearLayout
    private lateinit var chromeLine: View
    private lateinit var suggestionBox: LinearLayout
    /** The engine's answer for the input it was asked about. */
    private var engineSuggestions: Pair<String, List<String>> = "" to emptyList()
    private var suggestionRequest = 0
    /** True while the app sets the address text itself, which is not typing. */
    private var settingAddress = false
    private lateinit var progress: ProgressLine
    private lateinit var address: EditText
    private lateinit var siteStatus: ImageView
    private lateinit var reload: ImageButton
    private lateinit var back: ImageButton
    private lateinit var forward: ImageButton
    private lateinit var tabCount: TextView
    private var notice: View? = null
    private var bottomInset = 0
    private var wideLayout = false
    private var panel: Panel? = null

    /** The page shown fullscreen (a video, a game), if any. */
    private var fullscreen: GeckoSession? = null
    private var insets = listOf(0, 0, 0, 0)
    /** A page's file upload waiting for the system file picker. */
    private var filePrompt: Pair<FilePrompt, GeckoResult<PromptResponse>>? = null
    /** A page script that runs too long, waiting for the user to stop it. */
    private var slowScript: GeckoResult<SlowScriptResponse>? = null
    /** An OnBackInvokedCallback on Android 13+; typed loosely so older versions never load the class. */
    private var backCallback: Any? = null
    private var backRegistered = false

    // ---------- Lifecycle ----------

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        store = StateStore(File(filesDir, "state.json"))
        downloader = Downloads(this)
        updater = Updater(this)
        Updater.onFailure = updateFailed
        val (state, problem) = store.load()
        bookmarks = state.bookmarks
        bookmarkTitles = state.bookmarkTitles
        history = state.history
        downloads = state.downloads
        settings = if (!File(filesDir, "state.json").exists() && problem == null) state.settings.copy(accent = Accent.Purple) else state.settings
        sidebarOpen = state.sidebarOpen
        when (settings.startup) {
            Startup.Restore -> state.tabs.forEach { tabs.add(it, select = false) }
            Startup.Home -> tabs.add(settings.homePage.ifEmpty { Address.BLANK })
            Startup.NewTab -> Unit
        }
        if (tabs.list.isEmpty()) tabs.add(Address.BLANK)
        if (settings.startup == Startup.Restore) tabs.list.getOrNull(state.activeIndex)?.let { tabs.select(it.id) }

        applyNightMode()
        Engine.setTheme(this, settings.theme)
        prompts = Prompts(this)
        geckoView = GeckoView(this).apply {
            importantForAutofill = View.IMPORTANT_FOR_AUTOFILL_YES
        }
        enterEdgeToEdge()
        buildUi()
        handleIntent(intent)
        show(tabs.active)
        loadNewTabPicture()
        when (problem) {
            StateStore.LoadProblem.Corrupt -> showNotice(getString(R.string.state_corrupt))
            StateStore.LoadProblem.Newer -> showNotice(getString(R.string.state_newer))
            StateStore.LoadProblem.Unreadable -> showNotice(getString(R.string.state_unreadable))
            null -> Unit
        }
        // Once per launch, not on rotation or other recreations.
        if (savedInstanceState == null) checkForUpdates(manual = false)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        if (handleIntent(intent)) {
            closeMenu()
            closePanel()
            show(tabs.active)
        }
    }

    override fun onResume() {
        super.onResume()
        resumed = true
        awaitingInstallPermission?.let { release ->
            awaitingInstallPermission = null
            if (packageManager.canRequestPackageInstalls()) installUpdate(release)
        }
        tabs.active.session?.setActive(true)
        pollDownloads()
    }

    override fun onPause() {
        resumed = false
        tabs.active.session?.setActive(false)
        saveNow()
        super.onPause()
    }

    override fun onDestroy() {
        main.removeCallbacksAndMessages(null)
        geckoView.releaseSession()
        for (tab in tabs.list) tab.session?.close()
        filePrompt?.let { (prompt, result) -> result.complete(prompt.dismiss()) }
        writer.shutdown()
        suggestionFetcher.shutdownNow()
        downloader.shutdown()
        pictures.shutdown()
        updater.shutdown()
        if (Updater.onFailure === updateFailed) Updater.onFailure = null
        super.onDestroy()
    }

    override fun onTrimMemory(level: Int) {
        super.onTrimMemory(level)
        @Suppress("DEPRECATION")
        if (level >= ComponentCallbacks2.TRIM_MEMORY_RUNNING_LOW) {
            evict(1)
            thumbnails.clear()
            captureRequests.clear()
            panel?.onModelChanged()
        }
    }

    override fun onConfigurationChanged(newConfig: Configuration) {
        super.onConfigurationChanged(newConfig)
        if (::palette.isInitialized && (isDark() != palette.dark || isWide() != wideLayout)) rebuildUi()
    }

    /** Android 12 and older: the back key and gesture. Newer versions use [updateBackHandling]. */
    @android.annotation.SuppressLint("GestureBackNavigation")
    @Deprecated("Android 13+ uses the OnBackInvokedCallback registered in updateBackHandling.")
    override fun onBackPressed() {
        @Suppress("DEPRECATION")
        if (!handleBack()) super.onBackPressed()
    }

    /** Leaves fullscreen, overlays, the address bar, then page history, in that order. */
    private fun handleBack(): Boolean {
        val tab = tabs.active
        when {
            fullscreen != null -> exitFullscreen()
            menu != null -> closeMenu()
            panel != null -> closePanel()
            address.hasFocus() -> blurAddress()
            tab.canGoBack && tab.session != null -> tab.session?.goBack()
            tab.opener != null -> closeTab(tab.id)
            else -> return false
        }
        return true
    }

    private fun canHandleBack(): Boolean {
        val tab = tabs.active
        return fullscreen != null || menu != null || panel != null || address.hasFocus() || (tab.canGoBack && tab.session != null) || tab.opener != null
    }

    /**
     * Android 13+ (predictive back): the callback is registered only while the app
     * has something to go back to, so otherwise the system shows its back-to-home
     * animation and leaves the app.
     */
    private fun updateBackHandling() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        val callback = backCallback as? android.window.OnBackInvokedCallback
            ?: android.window.OnBackInvokedCallback { handleBack(); updateBackHandling() }.also { backCallback = it }
        val want = canHandleBack()
        if (want == backRegistered) return
        if (want) {
            onBackInvokedDispatcher.registerOnBackInvokedCallback(android.window.OnBackInvokedDispatcher.PRIORITY_DEFAULT, callback)
        } else {
            onBackInvokedDispatcher.unregisterOnBackInvokedCallback(callback)
        }
        backRegistered = want
    }

    // ---------- Interface ----------

    private fun isDark(): Boolean = when (settings.theme) {
        Theme.Dark -> true
        Theme.Light -> false
        Theme.System -> (resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES
    }

    /**
     * The app's own night mode (Android 12+). Pages get the theme from the
     * engine's settings ([Engine.setTheme]).
     */
    private fun applyNightMode() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S) return
        val mode = when (settings.theme) {
            Theme.System -> UiModeManager.MODE_NIGHT_AUTO
            Theme.Light -> UiModeManager.MODE_NIGHT_NO
            Theme.Dark -> UiModeManager.MODE_NIGHT_YES
        }
        getSystemService(UiModeManager::class.java).setApplicationNightMode(mode)
    }

    @Suppress("DEPRECATION")
    private fun enterEdgeToEdge() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            window.setDecorFitsSystemWindows(false)
        } else {
            window.decorView.systemUiVisibility = View.SYSTEM_UI_FLAG_LAYOUT_STABLE or
                View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN or View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
        }
    }

    @Suppress("DEPRECATION")
    private fun styleSystemBars() {
        window.decorView.setBackgroundColor(palette.canvas)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            val light = WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS or WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS
            window.insetsController?.setSystemBarsAppearance(if (palette.dark) 0 else light, light)
        } else {
            val flags = View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR or View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR
            val v = window.decorView
            v.systemUiVisibility = if (palette.dark) v.systemUiVisibility and flags.inv() else v.systemUiVisibility or flags
        }
    }

    private fun isWide() = resources.configuration.screenWidthDp >= 600 || resources.configuration.orientation == Configuration.ORIENTATION_LANDSCAPE

    private fun rebuildUi() {
        val reopen = panel?.kind
        val reopenMenu = menu != null
        val noticeText = (notice?.tag as? String)
        val typing = if (address.hasFocus()) address.text.toString() else null
        buildUi()
        show(tabs.active)
        reopen?.let(::openPanel)
        if (reopenMenu) openMenu()
        // show() may already have raised the crash notice, with its Reload action.
        if (notice == null) noticeText?.let { showNotice(it) }
        typing?.let {
            address.requestFocus()
            setAddress(it)
            address.setSelection(it.length)
            updateSuggestions()
        }
    }

    /** Builds the browser chrome. The page view survives a rebuild and is re-attached. */
    private fun buildUi() {
        palette = Palette(isDark(), settings.accent)
        val p = palette
        (geckoView.parent as? FrameLayout)?.removeView(geckoView)
        panel = null
        menu = null
        notice = null

        pageHost = FrameLayout(this)
        blank = ImageView(this).apply {
            setBackgroundColor(p.canvas)
            scaleType = ImageView.ScaleType.CENTER_CROP
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }
        pageHost.addView(blank, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT))
        home = HomeView(this)
        pageHost.addView(home, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT))
        pageHost.addView(geckoView, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT))
        // Over the bottom of the page, right above the address bar.
        suggestionBox = column().apply {
            setBackgroundColor(p.surface)
            elevation = dp(4).toFloat()
            visibility = View.GONE
            contentDescription = getString(R.string.suggestions)
        }
        pageHost.addView(suggestionBox, FrameLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT, Gravity.BOTTOM))
        progress = ProgressLine(this, p.accent)

        address = SwipeAddressEditText(this).apply {
            onSwipe = { offset ->
                tabs.list.getOrNull(tabs.indexOf(tabs.activeId) + offset)?.let { selectTab(it.id) }
            }
            onSwipeUp = { openPanel(PanelKind.Tabs) }
            background = null
            hint = getString(R.string.address_hint)
            setHintTextColor(p.secondary)
            setTextColor(p.label)
            textSize = 16f
            isSingleLine = true
            ellipsize = TextUtils.TruncateAt.END
            setSelectAllOnFocus(true)
            imeOptions = EditorInfo.IME_ACTION_GO or EditorInfo.IME_FLAG_NO_EXTRACT_UI
            inputType = android.text.InputType.TYPE_CLASS_TEXT or android.text.InputType.TYPE_TEXT_VARIATION_URI
            setPadding(dp(4), 0, dp(4), 0)
            highlightColor = (p.accent and 0x00FFFFFF) or 0x40000000
            textCursorDrawable = rounded(p.accent, 0).apply { setSize(dp(2), 0) }
            setOnEditorActionListener { _, action, event ->
                val enter = event?.keyCode == KeyEvent.KEYCODE_ENTER && event.action == KeyEvent.ACTION_DOWN
                if (action == EditorInfo.IME_ACTION_GO || enter) {
                    navigate(text.toString())
                    true
                } else {
                    false
                }
            }
            setOnFocusChangeListener { v, focused ->
                if (focused) {
                    val url = tabs.active.url
                    setAddress(if (url == Address.BLANK) "" else url)
                    selectAll()
                    updateBackHandling()
                } else {
                    getSystemService(InputMethodManager::class.java).hideSoftInputFromWindow(v.windowToken, 0)
                    hideSuggestions()
                    render()
                }
            }
            addTextChangedListener(object : TextWatcher {
                override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) = Unit
                override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) = Unit
                override fun afterTextChanged(s: Editable?) {
                    if (!settingAddress) updateSuggestions()
                }
            })
        }
        siteStatus = ImageView(this).apply {
            imageTintList = ColorStateList.valueOf(p.secondary)
            setPadding(dp(12), 0, dp(8), 0)
        }
        reload = iconButton(R.drawable.ic_reload, R.string.reload, p) { reloadOrStop() }
        val pill = row(siteStatus, address, reload).apply {
            background = rounded(p.field, dp(8))
            minimumHeight = dp(44)
        }
        address.layoutParams = weight(1f, dp(48))
        siteStatus.layoutParams = size(dp(44), dp(44))
        reload.layoutParams = size(dp(48), dp(48))

        back = iconButton(R.drawable.ic_back, R.string.back, p) { goBack() }
        forward = iconButton(R.drawable.ic_forward, R.string.forward, p) { tabs.active.session?.goForward() }
        val plus = iconButton(R.drawable.ic_plus, R.string.new_tab, p) { newTab() }
        tabCount = label("1", 13f, p.label, bold = true).apply {
            gravity = Gravity.CENTER
            background = rounded(0, dp(5), p.label, dp(1.6f))
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }
        val tabsButton = FrameLayout(this).apply {
            background = pressable(p, radius = dp(8))
            contentDescription = getString(R.string.tabs)
            isClickable = true
            isFocusable = true
            setOnClickListener { openPanel(PanelKind.Tabs) }
            addView(tabCount, FrameLayout.LayoutParams(dp(22), dp(22), Gravity.CENTER))
        }
        val more = iconButton(R.drawable.ic_menu, R.string.menu, p) { openMenu() }
        val buttons = listOf(back, forward, plus, tabsButton, more)
        wideLayout = isWide()
        chrome = if (wideLayout) {
            // Landscape and tablets: one row, so the page keeps most of the height.
            row(back, forward, pill, plus, tabsButton, more).also {
                for (b in buttons) b.layoutParams = size(dp(48), dp(48))
                pill.layoutParams = weight(1f).apply { setMargins(dp(8), 0, dp(8), 0) }
            }
        } else {
            column(pill, row(*buttons.toTypedArray())).also {
                for (b in buttons) b.layoutParams = weight(1f, dp(48))
                (pill.layoutParams as LinearLayout.LayoutParams).bottomMargin = dp(4)
            }
        }
        chrome.setBackgroundColor(p.surface)
        chrome.setPadding(dp(12), dp(8), dp(12), dp(if (wideLayout) 8 else 2))

        chromeLine = hairline(this, p)
        content = column(pageHost, progress, chromeLine, chrome)
        pageHost.layoutParams = LinearLayout.LayoutParams(MATCH_PARENT, 0, 1f)
        progress.layoutParams = LinearLayout.LayoutParams(MATCH_PARENT, dp(2))

        frame = FrameLayout(this).apply {
            setBackgroundColor(p.canvas)
            // Holds focus when the address bar lets go, so it is not refocused.
            isFocusableInTouchMode = true
            addView(content, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT))
        }
        root = FrameLayout(this).apply { addView(frame, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT)) }
        root.setOnApplyWindowInsetsListener { _, windowInsets ->
            insets = systemInsets(windowInsets)
            applyInsets()
            windowInsets
        }
        setContentView(root)
        styleSystemBars()
        if (fullscreen != null) showChrome(false)
    }

    private fun applyInsets() {
        val (l, t, r, b) = insets
        // A fullscreen page covers the whole screen, under the hidden system bars.
        if (fullscreen != null) {
            frame.setPadding(0, 0, 0, 0)
            return
        }
        // The toolbar's color continues behind the navigation bar and keyboard gap.
        frame.setPadding(l, t, r, 0)
        chrome.setPadding(dp(12), dp(8), dp(12), dp(if (wideLayout) 8 else 2) + b)
        bottomInset = b
        panel?.view?.let { (it.layoutParams as FrameLayout.LayoutParams).bottomMargin = b; it.requestLayout() }
        menu?.view?.let { (it.layoutParams as FrameLayout.LayoutParams).bottomMargin = b; it.requestLayout() }
    }

    private fun showChrome(shown: Boolean) {
        val visibility = if (shown) View.VISIBLE else View.GONE
        chrome.visibility = visibility
        chromeLine.visibility = visibility
        progress.visibility = visibility
        applyInsets()
    }

    private fun setAddress(text: String) {
        settingAddress = true
        address.setText(text)
        settingAddress = false
    }

    /** Status/navigation bars, the display cutout and the keyboard, as l/t/r/b. */
    @Suppress("DEPRECATION")
    private fun systemInsets(insets: WindowInsets): List<Int> {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            val i = insets.getInsets(WindowInsets.Type.systemBars() or WindowInsets.Type.displayCutout() or WindowInsets.Type.ime())
            return listOf(i.left, i.top, i.right, i.bottom)
        }
        return listOf(insets.systemWindowInsetLeft, insets.systemWindowInsetTop, insets.systemWindowInsetRight, insets.systemWindowInsetBottom)
    }

    /** Updates the chrome from the active tab, writing only what changed. */
    private fun render() {
        val tab = tabs.active
        if (!address.hasFocus()) {
            val shown = displayAddress(tab.url)
            if (address.text.toString() != shown) setAddress(shown)
        }
        back.isEnabled = (tab.canGoBack && tab.session != null) || tab.opener != null
        forward.isEnabled = tab.canGoForward && tab.session != null
        back.alpha = if (back.isEnabled) 1f else 0.3f
        forward.alpha = if (forward.isEnabled) 1f else 0.3f
        val web = Address.isWeb(tab.url)
        siteStatus.setImageResource(when {
            tab.url == Address.BLANK || tab.url.isEmpty() -> R.drawable.ic_search
            tab.url.startsWith("https://", ignoreCase = true) -> R.drawable.ic_lock
            else -> R.drawable.ic_info
        })
        siteStatus.contentDescription = getString(when {
            tab.url == Address.BLANK || tab.url.isEmpty() -> R.string.search
            tab.url.startsWith("https://", ignoreCase = true) -> R.string.site_secure
            else -> R.string.site_info
        })
        reload.visibility = if (web || tab.crashed) View.VISIBLE else View.GONE
        reload.setImageResource(if (tab.loading) R.drawable.ic_close else R.drawable.ic_reload)
        reload.contentDescription = getString(if (tab.loading) R.string.stop else R.string.reload)
        val count = tabs.list.size.toString()
        val shownCount = if (tabs.list.size > 99) "99+" else count
        if (tabCount.text.toString() != shownCount) {
            tabCount.text = shownCount
            tabCount.textSize = if (tabs.list.size > 99) 9f else 13f
        }
        (tabCount.parent as View).contentDescription = resources.getQuantityString(R.plurals.tabs_count, tabs.list.size, tabs.list.size)
        progress.set(if (tab.loading) tab.progress else 0)
        panel?.onModelChanged()
        if (tabs.active.url == Address.BLANK) updateHome()
        updateBackHandling()
    }

    /** The host for https pages, the whole address otherwise (so http stands out). */
    private fun displayAddress(url: String): String = when {
        url == Address.BLANK || url.isEmpty() -> ""
        url.startsWith("https://") -> Address.hostLabel(url).removePrefix("www.").ifEmpty { url }
        else -> url
    }

    private fun blurAddress() {
        if (address.hasFocus()) frame.requestFocus()
    }

    fun showNotice(text: String, action: String? = null, onAction: (() -> Unit)? = null) {
        notice?.let { frame.removeView(it) }
        val p = palette
        val message = label(text, 15f, p.label).apply { setLineSpacing(0f, 1.15f) }
        val box = row(message).apply {
            tag = text
            background = rounded(p.surface, dp(12), p.line, dp(1))
            elevation = dp(6).toFloat()
            setPadding(dp(16), dp(12), dp(4), dp(12))
        }
        message.layoutParams = weight()
        if (action != null && onAction != null) {
            box.addView(label(action, 15f, p.accent, bold = true).apply {
                setPadding(dp(12), dp(8), dp(12), dp(8))
                background = pressable(p, radius = dp(8))
                setOnClickListener { dismissNotice(); onAction() }
            })
        }
        box.addView(iconButton(R.drawable.ic_close, R.string.dismiss, p) { dismissNotice() }, size(dp(40), dp(40)))
        frame.addView(box, FrameLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT, Gravity.TOP).apply {
            setMargins(dp(12), dp(8), dp(12), 0)
        })
        notice = box
        updateOverlayAccessibility()
        if (action == null) main.postDelayed({ if (notice === box) dismissNotice() }, 5000)
    }

    private fun dismissNotice() {
        notice?.let { frame.removeView(it) }
        notice = null
    }

    fun openPanel(kind: PanelKind) {
        closeMenu()
        if (kind == PanelKind.Tabs) captureThumbnail()
        blurAddress()
        hideSuggestions()
        exitFullscreen()
        panel?.let { frame.removeView(it.view) }
        val next = when (kind) {
            PanelKind.Settings -> SettingsPanel(this)
            PanelKind.Tabs -> TabsPanel(this)
            else -> LibraryPanel(this, kind)
        }
        panel = next
        frame.addView(next.view, 1, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT).apply { bottomMargin = bottomInset })
        // Screen readers should not reach the page and toolbar behind the sheet.
        updateOverlayAccessibility()
        next.view.requestFocus()
        if (kind == PanelKind.Downloads) pollDownloads()
        updateBackHandling()
    }

    fun closePanel() {
        panel?.let { frame.removeView(it.view) }
        panel = null
        updateOverlayAccessibility()
        updateBackHandling()
    }

    private fun openMenu() {
        blurAddress()
        hideSuggestions()
        exitFullscreen()
        closePanel()
        closeMenu()
        val next = MenuSheet(this, ::toggleBookmark, ::share) {
            val tab = tabs.active
            if (tab.session == null) recover(tab) else tab.session?.reload()
        }
        menu = next
        frame.addView(next.view, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT).apply { bottomMargin = bottomInset })
        updateOverlayAccessibility()
        next.view.requestFocus()
        updateBackHandling()
    }

    fun closeMenu() {
        menu?.let { frame.removeView(it.view) }
        menu = null
        updateOverlayAccessibility()
        updateBackHandling()
    }

    private fun updateOverlayAccessibility() {
        val hidden = panel != null || menu != null
        content.descendantFocusability = if (hidden) android.view.ViewGroup.FOCUS_BLOCK_DESCENDANTS else android.view.ViewGroup.FOCUS_AFTER_DESCENDANTS
        if (hidden) content.findFocus()?.clearFocus()
        content.importantForAccessibility = if (hidden) View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS else View.IMPORTANT_FOR_ACCESSIBILITY_AUTO
        notice?.importantForAccessibility = if (hidden) View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS else View.IMPORTANT_FOR_ACCESSIBILITY_AUTO
    }

    private fun updateHome() {
        home.update(Shortcuts.select(bookmarks, history, settings.keepHistory, ::titleFor), newTabPicture != null)
    }

    fun thumbnailFor(id: Long): Bitmap? = thumbnails[id]

    /** Capture belongs to the displayed session, which may already be losing its selection. */
    private fun captureThumbnail() {
        if (!::geckoView.isInitialized) return
        val session = geckoView.session ?: return
        val tab = tabs.list.firstOrNull { it.session === session } ?: return
        if (tab.url == Address.BLANK || geckoView.width == 0 || geckoView.height == 0) return
        val url = tab.url
        val request = ++captureSerial
        captureRequests[tab.id] = request
        try {
            geckoView.capturePixels().accept({ bitmap ->
                if (bitmap != null) {
                    if (!isDestroyed && tabs.find(tab.id) === tab && tab.session === session && tab.url == url && captureRequests[tab.id] == request) {
                        try {
                            // The top of the page, in the tab card's 4:3 shape, as Firefox shows it.
                            val top = Bitmap.createBitmap(bitmap, 0, 0, bitmap.width, minOf(bitmap.height, bitmap.width * 3 / 4))
                            val scaled = Bitmap.createScaledBitmap(top, maxOf(1, top.width / 3), maxOf(1, top.height / 3), true)
                            thumbnails[tab.id] = scaled
                            if (top !== scaled && top !== bitmap) top.recycle()
                            if (scaled !== bitmap) bitmap.recycle()
                            if (panel?.kind == PanelKind.Tabs) panel?.onModelChanged()
                        } catch (_: OutOfMemoryError) {
                            bitmap.recycle()
                        } catch (_: RuntimeException) {
                            bitmap.recycle()
                        }
                    } else {
                        bitmap.recycle()
                    }
                }
            }, { /* The compositor may be starting, stopping or unavailable. */ })
        } catch (_: OutOfMemoryError) {
            // Keep the placeholder if the compositor cannot allocate a bitmap.
        } catch (_: RuntimeException) {
            // A page may lose its surface before the request reaches Gecko.
        }
    }

    // ---------- Tabs and pages ----------

    /** Makes [tab] the visible one, creating its page if needed. */
    fun show(tab: Tab) {
        exitFullscreen()
        val previous = geckoView.session
        if (previous !== tab.session) captureThumbnail()
        tabs.select(tab.id)
        val session = if (tab.session != null || (tab.url != Address.BLANK && !tab.crashed)) ensureSession(tab) else null
        if (previous !== session) {
            previous?.setActive(false)
            geckoView.releaseSession()
            if (session != null) {
                geckoView.coverUntilFirstPaint(palette.canvas)
                geckoView.setSession(session)
            }
        }
        session?.setActive(resumed)
        showBlank(tab)
        if (tab.crashed) showCrashNotice()
        evict(MAX_LIVE_PAGES)
        render()
        scheduleSave()
    }

    /** A tab without a page shows the canvas; a new tab shows the chosen picture on it. */
    private fun showBlank(tab: Tab) {
        val homeTab = tab.url == Address.BLANK
        val noPage = homeTab || tab.session == null
        geckoView.visibility = if (noPage) View.INVISIBLE else View.VISIBLE
        blank.visibility = if (noPage) View.VISIBLE else View.GONE
        val picture = newTabPicture.takeIf { tab.url == Address.BLANK }
        if ((blank.drawable as? BitmapDrawable)?.bitmap !== picture) blank.setImageBitmap(picture)
        home.visibility = if (homeTab) View.VISIBLE else View.GONE
        if (tab.url == Address.BLANK) updateHome()
    }

    fun selectTab(id: Long) {
        tabs.find(id)?.let(::show)
    }

    fun newTab() {
        show(tabs.add(Address.BLANK))
        closePanel()
        address.requestFocus()
        address.post {
            getSystemService(InputMethodManager::class.java).showSoftInput(address, 0)
        }
    }

    fun closeTab(id: Long) {
        val tab = tabs.close(id) ?: return
        thumbnails.remove(id)
        captureRequests.remove(id)
        tab.session?.let {
            if (geckoView.session === it) geckoView.releaseSession()
            it.close()
        }
        tab.session = null
        show(tabs.active)
    }

    /** Opens [url] in the active tab. */
    fun open(url: String) {
        val tab = tabs.active
        thumbnails.remove(tab.id)
        captureRequests.remove(tab.id)
        tab.url = url
        tab.crashed = false
        val session = tab.session
        if (session == null) show(tab) else session.loadUri(url)
        showBlank(tab)
        render()
    }

    private fun goBack() {
        val tab = tabs.active
        if (tab.canGoBack && tab.session != null) tab.session?.goBack() else tab.opener?.let { closeTab(tab.id) }
    }

    private fun navigate(input: String) {
        val text = input.trim()
        if (text.length > Address.URL_LIMIT) {
            showNotice(getString(R.string.address_too_long))
            return
        }
        val url = Address.normalize(text, settings.search)
        blurAddress()
        if (url != Address.BLANK) open(url)
    }

    private fun showCrashNotice() {
        showNotice(getString(R.string.page_crashed), getString(R.string.reload)) { recover(tabs.active) }
    }

    /** Recreates the page of a crashed tab. */
    private fun recover(tab: Tab) {
        tab.crashed = false
        dismissNotice()
        show(tab)
    }

    private fun reloadOrStop() {
        val tab = tabs.active
        val session = tab.session
        when {
            session == null -> recover(tab)
            tab.loading -> session.stop()
            else -> session.reload()
        }
    }

    private fun share() {
        val url = tabs.active.url
        if (!Address.isWeb(url)) return
        val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, url)
            .putExtra(Intent.EXTRA_TITLE, tabs.active.title)
        startActivity(Intent.createChooser(send, null))
    }

    /** The tab's open page; a released page comes back with its back/forward history. */
    private fun ensureSession(tab: Tab): GeckoSession {
        tab.session?.let { return it }
        val session = createSession(tab)
        tab.session = session
        tab.crashed = false
        session.open(Engine.runtime(this))
        val saved = tab.saved
        if (saved != null && saved.size > 0) session.restoreState(saved) else session.loadUri(tab.url)
        return session
    }

    /** Releases the pages of the least recently shown background tabs. */
    private fun evict(maxLive: Int) {
        for (tab in tabs.toEvict(maxLive)) {
            val session = tab.session ?: continue
            // tab.saved already holds the latest back/forward list (onSessionStateChange).
            tab.session = null
            tab.loading = false
            session.close()
        }
    }

    /** A page for [tab], not yet open: the engine opens popups itself. */
    private fun createSession(tab: Tab): GeckoSession = Engine.newSession().apply {
        val delegate = PageDelegate(tab, this)
        navigationDelegate = delegate
        progressDelegate = delegate
        contentDelegate = delegate
        permissionDelegate = delegate
        promptDelegate = prompts
        selectionActionDelegate = BasicSelectionActionDelegate(this@MainActivity)
    }

    /**
     * Events of one tab's page. Each checks that the page still belongs to the
     * tab: a closed or replaced page can report late.
     */
    private inner class PageDelegate(private val tab: Tab, private val page: GeckoSession) :
        NavigationDelegate, ProgressDelegate, ContentDelegate, PermissionDelegate {

        private val current get() = tab.session === page && tabs.find(tab.id) === tab
        private val active get() = current && tab.id == tabs.activeId

        override fun onLocationChange(session: GeckoSession, url: String?, perms: MutableList<PermissionDelegate.ContentPermission>, hasUserGesture: Boolean) {
            if (!current || url == null) return
            // The first, empty document of a new page is not an address.
            if (url == Address.BLANK && tab.url != Address.BLANK) return
            if (url != tab.url) {
                thumbnails.remove(tab.id)
                captureRequests.remove(tab.id)
                tab.url = url
                scheduleSave()
            }
            if (settings.keepHistory && Address.isWeb(url) && history.lastOrNull()?.url != url) {
                // The title arrives later through onTitleChange.
                history = BrowserState.pushHistory(history, url, "")
                scheduleSave()
            }
            if (active) { showBlank(tab); render() }
        }

        override fun onCanGoBack(session: GeckoSession, canGoBack: Boolean) {
            if (!current) return
            tab.canGoBack = canGoBack
            if (active) render()
        }

        override fun onCanGoForward(session: GeckoSession, canGoForward: Boolean) {
            if (!current) return
            tab.canGoForward = canGoForward
            if (active) render()
        }

        override fun onLoadRequest(session: GeckoSession, request: NavigationDelegate.LoadRequest): GeckoResult<AllowOrDeny> {
            val external = openExternal(Uri.parse(request.uri), request.hasUserGesture || request.isRedirect)
            return GeckoResult.fromValue(if (external) AllowOrDeny.DENY else AllowOrDeny.ALLOW)
        }

        /** `target=_blank` and `window.open` open a tab that keeps `opener`; Gecko blocks popups without a tap. */
        override fun onNewSession(session: GeckoSession, uri: String): GeckoResult<GeckoSession>? {
            if (!current) return null
            val child = tabs.add(Address.BLANK, after = tab.id, select = false)
            child.opener = tab.id
            child.url = uri
            val created = createSession(child)
            child.session = created
            show(child)
            return GeckoResult.fromValue(created)
        }

        override fun onPageStart(session: GeckoSession, url: String) {
            if (!current) return
            captureRequests.remove(tab.id)
            thumbnails.remove(tab.id)
            tab.loading = true
            tab.progress = 10
            if (active) render()
        }

        override fun onPageStop(session: GeckoSession, success: Boolean) {
            if (!current) return
            tab.loading = false
            if (active) {
                render()
                // The compositor paints the finished page a moment after it stops loading.
                main.postDelayed({ if (active && !tab.loading) captureThumbnail() }, 800)
            }
        }

        override fun onProgressChange(session: GeckoSession, value: Int) {
            if (!current) return
            tab.progress = value
            if (active) progress.set(if (tab.loading) value else 0)
        }

        override fun onSessionStateChange(session: GeckoSession, sessionState: GeckoSession.SessionState) {
            if (current) tab.saved = sessionState
        }

        override fun onTitleChange(session: GeckoSession, title: String?) {
            if (!current) return
            tab.title = title.orEmpty().take(TITLE_LIMIT)
            val last = history.lastOrNull()
            if (last != null && last.url == tab.url && last.title != tab.title && settings.keepHistory) {
                history = history.dropLast(1) + last.copy(title = tab.title)
                scheduleSave()
            }
            if (active) render() else panel?.onModelChanged()
        }

        override fun onCloseRequest(session: GeckoSession) {
            if (current) closeTab(tab.id)
        }

        override fun onFullScreen(session: GeckoSession, fullScreen: Boolean) {
            if (!active) return
            if (fullScreen) enterFullscreen(session) else leaveFullscreen()
        }

        override fun onExternalResponse(session: GeckoSession, response: WebResponse) {
            startDownload(response)
            // A tab opened only to download (target=_blank) has nothing to show.
            if (current && tab.opener != null && !tab.canGoBack && !Address.isWeb(tab.url)) closeTab(tab.id)
        }

        override fun onContextMenu(session: GeckoSession, screenX: Int, screenY: Int, element: ContentDelegate.ContextElement) {
            val link = element.linkUri?.takeIf(Address::isWeb) ?: return
            if (active) showLinkMenu(tab, link)
        }

        /** The page's process crashed; the page is gone and its history with it. */
        override fun onCrash(session: GeckoSession) {
            if (!current) return
            lose(session, keepHistory = false)
        }

        /** Android ended the page's process for memory; it comes back with its history. */
        override fun onKill(session: GeckoSession) {
            if (!current) return
            lose(session, keepHistory = true)
        }

        private fun lose(session: GeckoSession, keepHistory: Boolean) {
            if (geckoView.session === session) geckoView.releaseSession()
            session.close()
            tab.session = null
            if (!keepHistory) tab.saved = null
            tab.loading = false
            tab.crashed = !keepHistory
            if (tab.id == tabs.activeId) {
                if (keepHistory) show(tab) else {
                    showBlank(tab)
                    render()
                    showCrashNotice()
                }
            }
        }

        override fun onSlowScript(session: GeckoSession, scriptFileName: String): GeckoResult<SlowScriptResponse> {
            if (!active) return GeckoResult.fromValue(SlowScriptResponse.CONTINUE)
            slowScript?.complete(SlowScriptResponse.CONTINUE)
            val result = GeckoResult<SlowScriptResponse>()
            slowScript = result
            showNotice(getString(R.string.page_unresponsive), getString(R.string.page_stop)) {
                if (slowScript === result) slowScript = null
                result.complete(SlowScriptResponse.STOP)
            }
            // Unanswered, the page keeps running; the engine asks again if it stays stuck.
            main.postDelayed({
                if (slowScript === result) {
                    slowScript = null
                    result.complete(SlowScriptResponse.CONTINUE)
                    if (notice?.tag == getString(R.string.page_unresponsive)) dismissNotice()
                }
            }, 10_000)
            return result
        }

        /** Sites get no location, notifications, camera or microphone (the app has none of those permissions). */
        override fun onContentPermissionRequest(session: GeckoSession, perm: PermissionDelegate.ContentPermission): GeckoResult<Int> =
            GeckoResult.fromValue(
                if (perm.permission == PermissionDelegate.PERMISSION_AUTOPLAY_INAUDIBLE) PermissionDelegate.ContentPermission.VALUE_ALLOW
                else PermissionDelegate.ContentPermission.VALUE_DENY,
            )

        override fun onMediaPermissionRequest(
            session: GeckoSession,
            uri: String,
            video: Array<out PermissionDelegate.MediaSource>?,
            audio: Array<out PermissionDelegate.MediaSource>?,
            callback: PermissionDelegate.MediaCallback,
        ) = callback.reject()

        override fun onAndroidPermissionsRequest(session: GeckoSession, permissions: Array<out String>?, callback: PermissionDelegate.Callback) =
            callback.reject()
    }

    /** Long-pressing a link: open it in a new tab, copy it or share it. */
    private fun showLinkMenu(tab: Tab, link: String) {
        val actions = arrayOf(getString(R.string.link_open_new_tab), getString(R.string.link_copy), getString(R.string.link_share))
        AlertDialog.Builder(this)
            .setTitle(link)
            .setItems(actions) { _, which ->
                when (which) {
                    0 -> tabs.add(link, after = tab.id, select = false).also { it.opener = null; render() }
                    1 -> {
                        getSystemService(android.content.ClipboardManager::class.java)
                            .setPrimaryClip(android.content.ClipData.newPlainText(link, link))
                        showNotice(getString(R.string.link_copied))
                    }
                    else -> startActivity(Intent.createChooser(Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, link), null))
                }
            }
            .show()
    }

    /** Called by [Prompts] for `<input type=file>`; answered in [onActivityResult]. */
    fun chooseFiles(prompt: FilePrompt): GeckoResult<PromptResponse> {
        filePrompt?.let { (old, result) -> result.complete(old.dismiss()) }
        val types = prompt.mimeTypes?.filter { it.contains('/') }.orEmpty()
        val intent = Intent(Intent.ACTION_GET_CONTENT).addCategory(Intent.CATEGORY_OPENABLE)
            .setType(types.singleOrNull() ?: "*/*")
            .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, prompt.type == FilePrompt.Type.MULTIPLE)
        if (types.size > 1) intent.putExtra(Intent.EXTRA_MIME_TYPES, types.toTypedArray())
        val result = GeckoResult<PromptResponse>()
        return try {
            @Suppress("DEPRECATION")
            startActivityForResult(intent, FILE_REQUEST)
            filePrompt = prompt to result
            result
        } catch (e: ActivityNotFoundException) {
            GeckoResult.fromValue(prompt.dismiss())
        }
    }

    @Deprecated("Uses the platform result API to avoid an AndroidX dependency.")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        @Suppress("DEPRECATION")
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == PICTURE_REQUEST) {
            data?.data?.takeIf { resultCode == RESULT_OK }?.let(::setNewTabPicture)
            return
        }
        if (requestCode == IMPORT_REQUEST) {
            data?.data?.takeIf { resultCode == RESULT_OK }?.let(::importFile)
            return
        }
        if (requestCode != FILE_REQUEST) return
        val (prompt, result) = filePrompt ?: return
        filePrompt = null
        val clip = data?.clipData
        val uris = when {
            resultCode != RESULT_OK -> emptyArray()
            clip != null -> Array(clip.itemCount) { clip.getItemAt(it).uri }
            else -> listOfNotNull(data?.data).toTypedArray()
        }
        result.complete(
            when {
                uris.isEmpty() -> prompt.dismiss()
                prompt.type == FilePrompt.Type.MULTIPLE -> prompt.confirm(this, uris)
                else -> prompt.confirm(this, uris[0])
            },
        )
    }

    private fun enterFullscreen(session: GeckoSession) {
        if (fullscreen === session) return
        blurAddress()
        closeMenu()
        closePanel()
        fullscreen = session
        showChrome(false)
        setSystemBarsHidden(true)
        updateBackHandling()
    }

    /** The page left fullscreen itself. */
    private fun leaveFullscreen() {
        if (fullscreen == null) return
        fullscreen = null
        showChrome(true)
        setSystemBarsHidden(false)
        updateBackHandling()
    }

    /** Asks the page to leave fullscreen (Back, switching tabs, opening a panel). */
    private fun exitFullscreen() {
        val session = fullscreen ?: return
        session.exitFullScreen()
        leaveFullscreen()
    }

    @Suppress("DEPRECATION")
    private fun setSystemBarsHidden(hidden: Boolean) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            val controller = window.insetsController ?: return
            if (hidden) {
                controller.systemBarsBehavior = WindowInsetsController.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
                controller.hide(WindowInsets.Type.systemBars())
            } else {
                controller.show(WindowInsets.Type.systemBars())
            }
        } else {
            val base = View.SYSTEM_UI_FLAG_LAYOUT_STABLE or View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN or View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
            window.decorView.systemUiVisibility = if (hidden) {
                base or View.SYSTEM_UI_FLAG_FULLSCREEN or View.SYSTEM_UI_FLAG_HIDE_NAVIGATION or View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
            } else {
                base
            }
            styleSystemBars()
        }
    }

    /**
     * Links to other apps (mailto:, tel:, intent:, market:…) open only after a user
     * gesture, and never address a specific component. Local files are refused.
     */
    private fun openExternal(uri: Uri, gesture: Boolean): Boolean {
        when (uri.scheme?.lowercase()) {
            "http", "https", "about", "data", "blob", "javascript" -> return false
            "file", "content", null -> return true
        }
        if (!gesture) return true
        val intent = try {
            if (uri.scheme.equals("intent", ignoreCase = true)) {
                Intent.parseUri(uri.toString(), Intent.URI_INTENT_SCHEME)
            } else {
                Intent(Intent.ACTION_VIEW, uri)
            }
        } catch (e: java.net.URISyntaxException) {
            return true
        }
        if (intent.`package` == packageName) return true
        intent.addCategory(Intent.CATEGORY_BROWSABLE)
        intent.component = null
        intent.selector = null
        intent.flags = Intent.FLAG_ACTIVITY_NEW_TASK
        try {
            startActivity(intent)
        } catch (e: ActivityNotFoundException) {
            val fallback = intent.getStringExtra("browser_fallback_url")?.takeIf(Address::isWeb)
            if (fallback != null) open(fallback) else showNotice(getString(R.string.no_app))
        } catch (e: SecurityException) {
            showNotice(getString(R.string.no_app))
        }
        return true
    }

    /** VIEW, WEB_SEARCH and SEND intents open a tab; returns true when one did. */
    private fun handleIntent(received: Intent?): Boolean {
        val intent = received ?: return false
        val url = when (intent.action) {
            Intent.ACTION_VIEW -> intent.dataString?.takeIf(Address::isWeb)
            Intent.ACTION_WEB_SEARCH -> intent.getStringExtra(SearchManager.QUERY)?.let { Address.normalize(it, settings.search) }
            Intent.ACTION_SEND -> intent.getStringExtra(Intent.EXTRA_TEXT)?.let { text ->
                val link = Patterns.WEB_URL.matcher(text).let { m -> if (m.find()) m.group() else null }
                Address.webUrl(link ?: text) ?: Address.normalize(text, settings.search)
            }
            else -> null
        } ?: return false
        if (url == Address.BLANK || url.length > Address.URL_LIMIT) return false
        // Consume the intent so a later recreation does not open it again.
        intent.action = null
        captureThumbnail()
        val active = tabs.active
        if (active.url == Address.BLANK && active.session == null) {
            active.url = url
        } else {
            tabs.add(url)
        }
        return true
    }

    // ---------- Library ----------

    private fun toggleBookmark() {
        val url = tabs.active.url
        if (!Address.isWeb(url)) return
        bookmarks = if (url in bookmarks) bookmarks - url else bookmarks + url
        bookmarkTitles = bookmarkTitles - url
        render()
        scheduleSave()
    }

    fun removeBookmark(url: String) {
        bookmarks = bookmarks - url
        bookmarkTitles = bookmarkTitles - url
        render()
        scheduleSave()
    }

    /** Titles for bookmarked pages, from history (the newest visit wins). */
    fun titleFor(url: String): String = history.lastOrNull { it.url == url && it.title.isNotBlank() }?.title
        ?: tabs.list.firstOrNull { it.url == url && it.title.isNotBlank() }?.title ?: bookmarkTitles[url] ?: Address.hostLabel(url)

    private fun startDownload(response: WebResponse) {
        val download = downloader.start(response)
        if (download == null) {
            showNotice(getString(R.string.download_unsupported))
            return
        }
        downloads = BrowserState.trimDownloads(downloads + download)
        showNotice(getString(R.string.download_started, download.path), getString(R.string.downloads)) {
            openPanel(PanelKind.Downloads)
        }
        scheduleSave()
        pollDownloads()
    }

    fun openDownload(download: SavedDownload) {
        if (!downloader.open(download)) showNotice(getString(R.string.download_open_failed))
    }

    private val downloadTick = Runnable { pollDownloads() }

    /** Refreshes in-progress downloads once a second while the app is visible. */
    private fun pollDownloads() {
        main.removeCallbacks(downloadTick)
        if (downloads.none { it.state == DownloadState.InProgress }) return
        val next = BrowserState.trimDownloads(downloader.refresh(downloads))
        if (next != downloads) {
            downloads = next
            panel?.onModelChanged()
            scheduleSave()
        }
        if (resumed && downloads.any { it.state == DownloadState.InProgress }) main.postDelayed(downloadTick, 1000)
    }

    fun updateSettings(next: Settings) {
        val old = settings
        if (next == old) return
        settings = next
        scheduleSave()
        if (next.theme != old.theme) {
            applyNightMode()
            Engine.setTheme(this, next.theme)
        }
        if (next.theme != old.theme || next.accent != old.accent) rebuildUi() else {
            panel?.onModelChanged()
            if (tabs.active.url == Address.BLANK) updateHome()
        }
    }

    // ---------- Moving from another browser ----------

    /** Opens the document picker for an exported bookmarks file or Takeout history. */
    fun chooseImportFile() {
        val intent = Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("*/*")
            .putExtra(Intent.EXTRA_MIME_TYPES, arrayOf("text/html", "application/json", "text/csv", "text/comma-separated-values", "text/plain", "application/octet-stream"))
        try {
            @Suppress("DEPRECATION")
            startActivityForResult(intent, IMPORT_REQUEST)
        } catch (e: ActivityNotFoundException) {
            showNotice(getString(R.string.import_no_app))
        }
    }

    private fun importFile(uri: Uri) {
        pictures.execute {
            val file: Any = try {
                contentResolver.openInputStream(uri)?.use(Import::readLimited)?.let(Import::read) ?: R.string.import_too_large
            } catch (e: IOException) {
                R.string.import_unreadable
            } catch (e: SecurityException) {
                R.string.import_unreadable
            }
            main.post { if (!isDestroyed) applyImport(file) }
        }
    }

    private fun applyImport(file: Any) {
        val notice = when (file) {
            is ImportedFile.Bookmarks -> {
                val before = bookmarks.size
                val (merged, titles) = Import.mergeBookmarks(bookmarks, bookmarkTitles, file.links)
                bookmarks = merged
                bookmarkTitles = titles
                val count = merged.size - before
                if (count == 0) getString(R.string.import_nothing) else resources.getQuantityString(R.plurals.imported_bookmarks, count, count)
            }
            is ImportedFile.History -> if (!settings.keepHistory) {
                getString(R.string.import_history_off)
            } else {
                val (merged, count) = Import.mergeHistory(history, file.visits)
                history = merged
                if (count == 0) getString(R.string.import_nothing) else resources.getQuantityString(R.plurals.imported_history, count, count)
            }
            ImportedFile.Passwords -> getString(R.string.import_passwords_autofill)
            ImportedFile.Unrecognized -> getString(R.string.import_unrecognized)
            else -> getString(file as Int)
        }
        render()
        panel?.onModelChanged()
        scheduleSave()
        showNotice(notice)
    }

    /** The name of the autofill service that fills passwords into pages; null when autofill is off. */
    fun autofillService(): String? {
        val manager = getSystemService(android.view.autofill.AutofillManager::class.java) ?: return null
        if (!manager.isEnabled) return null
        val component = manager.autofillServiceComponentName ?: return null
        // Other apps are hidden from Still (package visibility), so name the usual one directly.
        if (component.packageName == "com.google.android.gms") return "Google"
        return try {
            packageManager.getApplicationLabel(packageManager.getApplicationInfo(component.packageName, 0)).toString()
        } catch (e: android.content.pm.PackageManager.NameNotFoundException) {
            component.packageName
        }
    }

    /** Android's autofill service (Google Password Manager by default) fills passwords in pages. */
    fun openAutofillSettings() {
        val intents = listOfNotNull(
            if (Build.VERSION.SDK_INT >= 34) Intent("android.settings.CREDENTIAL_PROVIDER") else null,
            Intent(android.provider.Settings.ACTION_REQUEST_SET_AUTOFILL_SERVICE).setData(Uri.parse("package:com.google.android.gms")),
            Intent(android.provider.Settings.ACTION_SETTINGS),
        )
        for (intent in intents) {
            try {
                startActivity(intent)
                return
            } catch (e: ActivityNotFoundException) {
                // Try the next screen.
            }
        }
    }

    // ---------- New tab picture ----------

    /** Opens the system photo picker (Android 13+) or the document picker. */
    fun chooseNewTabPicture() {
        val pickers = listOfNotNull(
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) Intent(MediaStore.ACTION_PICK_IMAGES).setType("image/*") else null,
            Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("image/*"),
        )
        for (intent in pickers) {
            try {
                @Suppress("DEPRECATION")
                startActivityForResult(intent, PICTURE_REQUEST)
                return
            } catch (e: ActivityNotFoundException) {
                // Try the next picker.
            }
        }
        showNotice(getString(R.string.picture_no_app))
    }

    fun removeNewTabPicture() {
        newTabPicture = null
        updateSettings(settings.copy(newTabImage = ""))
        showBlank(tabs.active)
        pictures.execute { NewTabImage.removeOthers(pictureDir, "") }
    }

    private class Picture(val name: String, val bitmap: Bitmap)

    /** Copies the chosen picture into the app's files once it is known to decode, then shows it. */
    private fun setNewTabPicture(uri: Uri) {
        val longest = pictureSide()
        pictures.execute {
            val result: Any = try {
                val bytes = contentResolver.openInputStream(uri)?.use(NewTabImage::readLimited)
                val bitmap = bytes?.takeIf { NewTabImage.sniff(it) != null }
                    ?.let { decodePicture(ImageDecoder.createSource(ByteBuffer.wrap(it)), longest) }
                if (bytes == null || bitmap == null) {
                    R.string.picture_unsupported
                } else {
                    try {
                        NewTabImage.store(pictureDir, bytes)?.let { Picture(it, bitmap) } ?: R.string.picture_unsupported
                    } catch (e: IOException) {
                        R.string.picture_failed
                    }
                }
            } catch (e: IOException) {
                R.string.picture_unreadable
            } catch (e: SecurityException) {
                R.string.picture_unreadable
            }
            main.post {
                if (isDestroyed) return@post
                if (result is Picture) {
                    newTabPicture = result.bitmap
                    updateSettings(settings.copy(newTabImage = result.name))
                    showBlank(tabs.active)
                    pictures.execute { NewTabImage.removeOthers(pictureDir, result.name) }
                } else {
                    showNotice(getString(result as Int))
                }
            }
        }
    }

    /** Decodes the saved picture at launch, and deletes any left over from an interrupted change. */
    private fun loadNewTabPicture() {
        val name = settings.newTabImage
        val longest = pictureSide()
        pictures.execute {
            NewTabImage.removeOthers(pictureDir, name)
            val bitmap = if (name.isEmpty()) null else decodePicture(ImageDecoder.createSource(File(pictureDir, name)), longest)
            main.post {
                if (isDestroyed || settings.newTabImage != name) return@post
                // A missing or unreadable file cannot come back; forget it rather than show an empty choice.
                if (bitmap == null && name.isNotEmpty()) updateSettings(settings.copy(newTabImage = ""))
                newTabPicture = bitmap
                showBlank(tabs.active)
                panel?.onModelChanged()
            }
        }
    }

    /** The screen's longer side: the picture must cover it in either orientation. */
    private fun pictureSide() = resources.displayMetrics.let { maxOf(it.widthPixels, it.heightPixels) }

    /** Decodes no larger than needed to cover [longest] pixels in both directions; null if it cannot. */
    private fun decodePicture(source: ImageDecoder.Source, longest: Int): Bitmap? = try {
        ImageDecoder.decodeBitmap(source) { decoder, info, _ ->
            val (width, height) = info.size.width to info.size.height
            val scale = longest.toFloat() / minOf(width, height)
            if (scale < 1f) decoder.setTargetSize(maxOf(1, (width * scale).roundToInt()), maxOf(1, (height * scale).roundToInt()))
        }
    } catch (e: IOException) {
        null
    } catch (e: IllegalArgumentException) {
        null
    }

    fun clearBrowsingData(clearHistory: Boolean, siteData: Boolean, cache: Boolean) {
        thumbnails.clear()
        captureRequests.clear()
        if (clearHistory) {
            history = emptyList()
            for (tab in tabs.list) {
                tab.session?.purgeHistory()
                tab.saved = null
            }
        }
        var flags = 0L
        if (siteData) {
            flags = flags or StorageController.ClearFlags.COOKIES or StorageController.ClearFlags.DOM_STORAGES or
                StorageController.ClearFlags.SITE_DATA or StorageController.ClearFlags.AUTH_SESSIONS
        }
        if (cache) flags = flags or StorageController.ClearFlags.ALL_CACHES
        scheduleSave()
        render()
        if (flags == 0L) {
            showNotice(getString(R.string.cleared))
            return
        }
        Engine.runtime(this).storageController.clearData(flags).accept(
            { if (!isDestroyed) showNotice(getString(R.string.cleared)) },
            { if (!isDestroyed) showNotice(getString(R.string.cleared)) },
        )
    }

    // ---------- Address bar suggestions ----------

    private val engineTask = Runnable { askEngine() }

    /** Rebuilds the list for what is typed; the engine's answer follows when it arrives. */
    private fun updateSuggestions() {
        val typed = if (address.hasFocus()) address.text.toString().trim() else ""
        if (typed.isEmpty() || panel != null || menu != null || fullscreen != null) {
            hideSuggestions()
            return
        }
        val pages = Suggestions.local(typed, tabs.list, tabs.activeId, bookmarks, if (settings.keepHistory) history else emptyList(), ::titleFor)
        val phrases = engineSuggestions.takeIf { it.first == typed }?.second.orEmpty()
        val searches = phrases.map { Suggestion(Suggestion.Kind.Search, it, "") }
        showSuggestions(pages + searches)
        main.removeCallbacks(engineTask)
        if (settings.searchSuggestions && engineSuggestions.first != typed && Suggestions.mayAskEngine(typed)) {
            main.postDelayed(engineTask, 150)
        }
    }

    private fun askEngine() {
        val typed = address.text.toString().trim()
        val url = Suggestions.engineUrl(settings.search, typed) ?: return
        val request = ++suggestionRequest
        val language = Locale.getDefault().toLanguageTag()
        suggestionFetcher.execute {
            val phrases = try {
                val connection = URL(url).openConnection() as HttpURLConnection
                try {
                    connection.connectTimeout = 3000
                    connection.readTimeout = 3000
                    connection.instanceFollowRedirects = false
                    connection.setRequestProperty("Accept-Language", language)
                    if (connection.responseCode != 200) emptyList() else {
                        val body = connection.inputStream.use { it.readLimited(64 * 1024) }.toString(Charsets.UTF_8)
                        Suggestions.parseEngine(body, typed)
                    }
                } finally {
                    connection.disconnect()
                }
            } catch (e: IOException) {
                emptyList()
            } catch (e: RuntimeException) {
                emptyList()
            }
            main.post {
                if (isDestroyed || request != suggestionRequest) return@post
                engineSuggestions = typed to phrases
                updateSuggestions()
            }
        }
    }

    /** The best suggestion sits right above the address bar. */
    private fun showSuggestions(items: List<Suggestion>) {
        if (items.isEmpty()) {
            hideSuggestions()
            return
        }
        val p = palette
        suggestionBox.removeAllViews()
        suggestionBox.addView(hairline(this, p))
        for (item in items.asReversed()) suggestionBox.addView(suggestionRow(item, p))
        suggestionBox.visibility = View.VISIBLE
    }

    private fun hideSuggestions() {
        main.removeCallbacks(engineTask)
        if (!::suggestionBox.isInitialized || suggestionBox.visibility == View.GONE) return
        suggestionBox.visibility = View.GONE
        suggestionBox.removeAllViews()
    }

    private fun suggestionRow(item: Suggestion, p: Palette): View {
        val icon = ImageView(this).apply {
            setImageResource(when (item.kind) {
                Suggestion.Kind.Search -> R.drawable.ic_search
                Suggestion.Kind.Tab -> R.drawable.ic_tab
                Suggestion.Kind.Bookmark -> R.drawable.ic_star
                Suggestion.Kind.History -> R.drawable.ic_history
            })
            imageTintList = ColorStateList.valueOf(p.secondary)
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }
        val title = label(item.title.ifBlank { Suggestions.bare(item.url) }, 16f, p.label).apply {
            isSingleLine = true
            ellipsize = TextUtils.TruncateAt.END
        }
        val text = column(title)
        if (item.kind != Suggestion.Kind.Search) {
            val where = Suggestions.bare(item.url)
            text.addView(label(if (item.kind == Suggestion.Kind.Tab) getString(R.string.suggestion_switch_tab, where) else where, 13f, p.secondary).apply {
                isSingleLine = true
                ellipsize = TextUtils.TruncateAt.MIDDLE
                setPadding(0, dp(3), 0, 0)
            })
        }
        return row(icon, text).apply {
            minimumHeight = dp(52)
            setPadding(dp(16), dp(6), dp(16), dp(6))
            background = pressable(p)
            isClickable = true
            contentDescription = if (item.kind == Suggestion.Kind.Search) item.title else "${title.text}, ${Suggestions.bare(item.url)}"
            icon.layoutParams = size(dp(20), dp(20)).apply { marginEnd = dp(14) }
            text.layoutParams = weight()
            setOnClickListener { choose(item) }
        }
    }

    private fun choose(item: Suggestion) {
        blurAddress()
        when (item.kind) {
            Suggestion.Kind.Search -> open(settings.search.searchUrl(Address.encodeComponent(item.title)))
            Suggestion.Kind.Tab -> selectTab(item.tabId)
            else -> open(item.url)
        }
    }

    // ---------- Updates ----------

    /** A launch check speaks only when there is a new version; a manual one always answers. */
    fun checkForUpdates(manual: Boolean) {
        if (updating) return
        updating = true
        if (manual) showNotice(getString(R.string.update_checking))
        updater.check { result ->
            updating = false
            if (isDestroyed) return@check
            when (result) {
                is Updater.Check.Available -> showNotice(getString(R.string.update_available, result.release.version), getString(R.string.update_action)) {
                    installUpdate(result.release)
                }
                Updater.Check.UpToDate -> if (manual) showNotice(getString(R.string.update_latest))
                Updater.Check.Failed -> if (manual) showNotice(getString(R.string.update_check_failed))
            }
        }
    }

    private fun installUpdate(release: Updater.Release) {
        if (updating) return
        if (!packageManager.canRequestPackageInstalls()) {
            // Ask first; the update continues in onResume once allowed. Without this,
            // the installer's own prompt drops its confirmation on the way back.
            val request = Intent(android.provider.Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:$packageName"))
            try {
                startActivity(request)
                awaitingInstallPermission = release
                return
            } catch (e: ActivityNotFoundException) {
                // The installer asks instead.
            }
        }
        updating = true
        showNotice(getString(R.string.update_downloading))
        // The installer replaces the process; keep everything up to now.
        saveNow()
        updater.install(release) { ok ->
            updating = false
            if (!ok && !isDestroyed) showNotice(getString(R.string.update_failed))
        }
    }

    fun confirm(message: String, action: String, onConfirm: () -> Unit) {
        AlertDialog.Builder(this)
            .setMessage(message)
            .setPositiveButton(action) { _, _ -> onConfirm() }
            .setNegativeButton(android.R.string.cancel, null)
            .show()
    }

    // ---------- Saving ----------

    private val saveTask = Runnable { saveNow() }

    /** Changes are coalesced into one write at most every [SAVE_DELAY_MS]. */
    fun scheduleSave() {
        main.removeCallbacks(saveTask)
        main.postDelayed(saveTask, SAVE_DELAY_MS)
    }

    private fun saveNow() {
        main.removeCallbacks(saveTask)
        if (writer.isShutdown) return
        val state = BrowserState(
            sidebarOpen = sidebarOpen,
            tabs = tabs.list.map { it.url.ifEmpty { Address.BLANK } },
            activeIndex = maxOf(0, tabs.indexOf(tabs.activeId)),
            bookmarks = bookmarks,
            bookmarkTitles = bookmarkTitles,
            history = history,
            downloads = downloads,
            settings = settings,
        )
        writer.execute {
            val ok = store.save(state.toJson())
            main.post {
                if (ok) {
                    saveFailedShown = false
                } else if (!saveFailedShown && !isDestroyed) {
                    saveFailedShown = true
                    showNotice(getString(R.string.save_failed))
                }
            }
        }
    }

    /** A thin accent line showing page load progress. */
    @android.annotation.SuppressLint("ViewConstructor")
    class ProgressLine(context: android.content.Context, color: Int) : View(context) {
        private val paint = Paint().apply { this.color = color }
        private var value = 0

        fun set(progress: Int) {
            val next = progress.coerceIn(0, 100)
            if (next == value) return
            value = next
            invalidate()
        }

        override fun onDraw(canvas: Canvas) {
            if (value in 1..99) canvas.drawRect(0f, 0f, width * value / 100f, height.toFloat(), paint)
        }
    }

    companion object {
        const val MAX_LIVE_PAGES = 4
        const val TITLE_LIMIT = 300
        const val SAVE_DELAY_MS = 800L
        private const val FILE_REQUEST = 1
        private const val PICTURE_REQUEST = 2
        private const val IMPORT_REQUEST = 3
    }
}
