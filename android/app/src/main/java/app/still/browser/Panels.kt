package app.still.browser

import android.app.AlertDialog
import android.content.res.ColorStateList
import android.graphics.Color
import android.text.TextUtils
import android.text.format.DateFormat
import android.text.format.DateUtils
import android.text.format.Formatter
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.ViewGroup.LayoutParams.MATCH_PARENT
import android.view.ViewGroup.LayoutParams.WRAP_CONTENT
import android.view.inputmethod.EditorInfo
import android.widget.BaseAdapter
import android.widget.CheckBox
import android.widget.EditText
import android.widget.FrameLayout
import android.widget.ImageButton
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ListView
import android.widget.ScrollView
import android.widget.Switch
import android.widget.TextView
import org.json.JSONObject
import java.time.OffsetDateTime
import java.time.format.DateTimeParseException

enum class PanelKind { Tabs, Bookmarks, History, Downloads, Settings }

/** A library destination, settings page or tab sheet over the browser. */
abstract class Panel(var kind: PanelKind) {
    abstract val view: View
    /** Called after tabs, the library or settings change. */
    open fun onModelChanged() {}
}

/** Shared top bar for full-screen library and settings destinations. */
private fun MainActivity.panelBar(title: Int): View {
    val back = iconButton(R.drawable.ic_back, R.string.back, palette) { closePanel() }
    val text = label(getString(title), 20f, palette.label).apply {
        isAccessibilityHeading = true
    }
    return row(back, text).apply {
        setBackgroundColor(palette.surface)
        setPadding(dp(4), 0, dp(16), 0)
        minimumHeight = dp(56)
        back.layoutParams = size(dp(48), dp(48))
        text.layoutParams = weight().apply { marginStart = dp(4) }
    }
}

class LibraryPanel(private val a: MainActivity, kind: PanelKind) : Panel(kind) {
    private val p = a.palette
    /** Declared before the list, whose adapter reads it during construction. */
    private var items: List<Any> = emptyList()
    private val adapter = Rows()
    private val list = ListView(a).apply {
        this.adapter = this@LibraryPanel.adapter
        divider = null
        selector = android.graphics.drawable.ColorDrawable(Color.TRANSPARENT)
        clipToPadding = false
        setPadding(0, a.dp(4), 0, a.dp(8))
        setOnItemClickListener { _, _, position, _ -> pick(adapter.getItem(position)) }
    }
    private val empty = a.label("", 15f, p.secondary).apply {
        gravity = Gravity.CENTER
        setPadding(a.dp(32), 0, a.dp(32), 0)
    }
    override val view: View = a.column().apply {
        setBackgroundColor(p.surface)
        isClickable = true
        addView(a.panelBar(when (kind) {
            PanelKind.Bookmarks -> R.string.bookmarks
            PanelKind.History -> R.string.history
            PanelKind.Downloads -> R.string.downloads
            else -> error("LibraryPanel requires a library destination")
        }), LinearLayout.LayoutParams(MATCH_PARENT, a.dp(56)))
        addView(hairline(a, p))
        addView(FrameLayout(a).apply {
            addView(list, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT))
            addView(empty, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT))
        }, LinearLayout.LayoutParams(MATCH_PARENT, 0, 1f))
    }

    init {
        onModelChanged()
    }

    override fun onModelChanged() {
        items = when (kind) {
            PanelKind.Bookmarks -> a.bookmarks.asReversed()
            PanelKind.History -> a.history.asReversed()
            PanelKind.Downloads -> a.downloads.asReversed()
            else -> emptyList()
        }
        empty.text = when (kind) {
            PanelKind.Bookmarks -> a.getString(R.string.empty_bookmarks)
            PanelKind.History -> a.getString(R.string.empty_history)
            PanelKind.Downloads -> a.getString(R.string.empty_downloads)
            else -> ""
        }
        empty.visibility = if (items.isEmpty()) View.VISIBLE else View.GONE
        adapter.notifyDataSetChanged()
    }

    private fun pick(item: Any) {
        when (item) {
            is String -> {
                a.open(item)
                a.closePanel()
            }
            is HistoryEntry -> {
                a.open(item.url)
                a.closePanel()
            }
            is SavedDownload -> if (item.state == DownloadState.Completed) a.openDownload(item)
        }
    }

    private class Holder(val row: LinearLayout, val icon: ImageView, val title: TextView, val detail: TextView, val action: ImageButton)

    private inner class Rows : BaseAdapter() {
        override fun getCount() = items.size
        override fun getItem(position: Int) = items[position]
        override fun getItemId(position: Int) = position.toLong()

        override fun getView(position: Int, convertView: View?, parent: ViewGroup): View {
            val holder = (convertView?.tag as? Holder) ?: create()
            val item = items[position]
            var actionVisible = false
            when (item) {
                is String -> {
                    holder.icon.setImageResource(R.drawable.ic_star)
                    holder.title.text = a.titleFor(item)
                    holder.detail.text = item
                    actionVisible = true
                    holder.action.contentDescription = a.getString(R.string.remove_named, holder.title.text)
                    holder.action.setOnClickListener { a.removeBookmark(item) }
                }
                is HistoryEntry -> {
                    holder.icon.setImageResource(R.drawable.ic_history)
                    holder.title.text = item.title.ifBlank { Address.hostLabel(item.url).ifBlank { item.url } }
                    holder.detail.text = listOf(item.url, time(item.visitedAt)).filter { it.isNotEmpty() }.joinToString(" · ")
                }
                is SavedDownload -> {
                    holder.icon.setImageResource(R.drawable.ic_file)
                    holder.title.text = item.path
                    holder.detail.text = status(item)
                }
            }
            holder.detail.visibility = if (holder.detail.text.isEmpty()) View.GONE else View.VISIBLE
            holder.action.visibility = if (actionVisible) View.VISIBLE else View.GONE
            holder.row.background = pressable(p)
            holder.row.isFocusable = true
            holder.row.setOnClickListener { pick(item) }
            return holder.row
        }

        private fun create(): Holder {
            val icon = ImageView(a).apply {
                imageTintList = ColorStateList.valueOf(p.secondary)
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
            }
            val title = a.label("", 16f, p.label).apply {
                maxLines = 1
                ellipsize = TextUtils.TruncateAt.END
            }
            val detail = a.label("", 13f, p.secondary).apply {
                maxLines = 1
                ellipsize = TextUtils.TruncateAt.END
                setPadding(0, a.dp(3), 0, 0)
            }
            val texts = a.column(title, detail)
            val action = a.iconButton(R.drawable.ic_close, R.string.remove, p) {}.apply {
                imageTintList = ColorStateList.valueOf(p.secondary)
            }
            val row = a.row(icon, texts, action).apply {
                minimumHeight = a.dp(72)
                setPadding(a.dp(20), a.dp(12), a.dp(8), a.dp(12))
                layoutParams = android.widget.AbsListView.LayoutParams(MATCH_PARENT, WRAP_CONTENT)
                isScreenReaderFocusable = true
            }
            icon.layoutParams = size(a.dp(24), a.dp(24)).apply { marginEnd = a.dp(20) }
            texts.layoutParams = weight()
            action.layoutParams = size(a.dp(48), a.dp(48))
            return Holder(row, icon, title, detail, action).also { row.tag = it }
        }
    }

    private fun time(iso: String): String = try {
        val millis = OffsetDateTime.parse(iso).toInstant().toEpochMilli()
        if (DateUtils.isToday(millis)) DateFormat.getTimeFormat(a).format(millis)
        else DateUtils.formatDateTime(a, millis, DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_ABBREV_MONTH)
    } catch (e: DateTimeParseException) {
        ""
    }

    private fun status(d: SavedDownload): String = when (d.state) {
        DownloadState.Completed -> listOf(a.getString(R.string.download_completed), if (d.total > 0) Formatter.formatShortFileSize(a, d.total) else "")
            .filter { it.isNotEmpty() }.joinToString(" · ")
        DownloadState.Failed -> a.getString(R.string.download_failed)
        DownloadState.Interrupted -> a.getString(R.string.download_interrupted)
        DownloadState.InProgress -> if (d.total > 0) {
            a.getString(R.string.download_progress, Formatter.formatShortFileSize(a, d.bytes), Formatter.formatShortFileSize(a, d.total))
        } else if (d.bytes > 0) {
            Formatter.formatShortFileSize(a, d.bytes)
        } else {
            a.getString(R.string.download_waiting)
        }
    }
}

class SettingsPanel(private val a: MainActivity) : Panel(PanelKind.Settings) {
    private val p = a.palette
    private val content = a.column().apply { setPadding(0, 0, 0, a.dp(24)) }
    /** What the panel was built from; the picture arrives after the setting at launch. */
    private var shown: Pair<Settings, Any?>? = null
    private var clearHistory = true
    private var clearSiteData = true
    private var clearCache = true

    override val view: View = a.column().apply {
        setBackgroundColor(p.surface)
        isClickable = true
        addView(a.panelBar(R.string.settings), LinearLayout.LayoutParams(MATCH_PARENT, a.dp(56)))
        addView(hairline(a, p))
        addView(ScrollView(a).apply {
            isFillViewport = true
            addView(content)
        }, LinearLayout.LayoutParams(MATCH_PARENT, 0, 1f))
    }

    init {
        onModelChanged()
    }

    override fun onModelChanged() {
        val model = a.settings to a.newTabPicture
        if (shown == model) return
        shown = model
        build(a.settings)
    }

    private fun update(change: JSONObject) = a.updateSettings(a.settings.merge(change))

    private fun build(s: Settings) {
        content.removeAllViews()

        section(R.string.general)
        group(
            preference(R.string.startup, listOf(
                a.getString(R.string.startup_restore) to Startup.Restore,
                a.getString(R.string.startup_new_tab) to Startup.NewTab,
                a.getString(R.string.startup_home) to Startup.Home,
            ), s.startup) { update(JSONObject().put("Startup", it.name)) },
            caption(R.string.home_page),
            homePageField(s),
        )

        section(R.string.appearance)
        group(
            preference(R.string.theme, listOf(
                a.getString(R.string.theme_system) to Theme.System,
                a.getString(R.string.theme_light) to Theme.Light,
                a.getString(R.string.theme_dark) to Theme.Dark,
            ), s.theme) { update(JSONObject().put("Theme", it.name)) },
            note(a.getString(R.string.theme_note)),
            caption(R.string.accent),
            swatches(s.accent),
            caption(R.string.new_tab_picture),
            newTabPicture(s),
        )

        section(R.string.search)
        group(
            preference(R.string.search, listOf(
                a.getString(R.string.search_google) to SearchEngine.Google,
                a.getString(R.string.search_bing) to SearchEngine.Bing,
                a.getString(R.string.search_duckduckgo) to SearchEngine.DuckDuckGo,
                a.getString(R.string.search_yahoo) to SearchEngine.YahooJapan,
            ), s.search) { update(JSONObject().put("Search", it.name)) },
            switchRow(a.getString(R.string.search_suggestions), s.searchSuggestions) { update(JSONObject().put("SearchSuggestions", it)) },
            note(a.getString(R.string.search_suggestions_note)),
        )

        section(R.string.privacy)
        group(switchRow(a.getString(R.string.keep_history), s.keepHistory) { update(JSONObject().put("KeepHistory", it)) })
        val clearButton = a.label(a.getString(R.string.clear_button), 17f, p.danger, bold = true).apply {
            gravity = Gravity.CENTER_VERTICAL
            background = pressable(p)
            isFocusable = true
            setOnClickListener {
                if (!clearHistory && !clearSiteData && !clearCache) return@setOnClickListener
                a.confirm(a.getString(R.string.clear_data) + "?", a.getString(R.string.clear_button)) {
                    a.clearBrowsingData(clearHistory, clearSiteData, clearCache)
                }
            }
        }
        group(
            caption(R.string.clear_data),
            checkRow(R.string.clear_history, clearHistory) { clearHistory = it },
            checkRow(R.string.clear_site_data, clearSiteData) { clearSiteData = it },
            checkRow(R.string.clear_cache, clearCache) { clearCache = it },
            clearButton,
        )
        (clearButton.layoutParams as LinearLayout.LayoutParams).apply {
            height = a.dp(48)
            topMargin = a.dp(8)
        }

        section(R.string.import_section)
        val service = a.autofillService()
        group(
            caption(R.string.import_file),
            note(a.getString(R.string.import_file_note)),
            textButton(a.getString(R.string.import_choose), p.accent) { a.chooseImportFile() }.apply { gravity = Gravity.CENTER_VERTICAL; setPadding(0, 0, 0, 0) },
            caption(R.string.passwords),
            note(if (service != null) a.getString(R.string.autofill_on, service) else a.getString(R.string.autofill_off)),
            textButton(a.getString(R.string.autofill_open), p.accent) { a.openAutofillSettings() }.apply { gravity = Gravity.CENTER_VERTICAL; setPadding(0, 0, 0, 0) },
            caption(R.string.signing_in),
            note(a.getString(R.string.import_signin_note)),
        )

        section(R.string.about)
        val engine = Engine.version
        val checkUpdates = a.label(a.getString(R.string.update_check), 16f, p.accent, bold = true).apply {
            gravity = Gravity.CENTER_VERTICAL
            minimumHeight = a.dp(48)
            background = pressable(p)
            isFocusable = true
            setOnClickListener { a.checkForUpdates(manual = true) }
        }
        group(
            a.label(a.getString(R.string.version, BuildInfo.versionName(a)), 15f, p.label),
            note(a.getString(R.string.engine, engine)),
            checkUpdates,
        )
    }

    private fun section(title: Int) {
        if (content.childCount > 0) content.addView(hairline(a, p).apply {
            (layoutParams as LinearLayout.LayoutParams).topMargin = a.dp(16)
        })
        content.addView(a.label(a.getString(title), 14f, p.accent, bold = true).apply {
            isAccessibilityHeading = true
            setPadding(a.dp(20), a.dp(24), a.dp(20), a.dp(8))
        })
    }

    private fun group(vararg children: View) {
        val box = a.column().apply {
            setPadding(a.dp(20), 0, a.dp(20), a.dp(8))
        }
        for (child in children) {
            box.addView(child)
            (child.layoutParams as LinearLayout.LayoutParams).apply {
                width = MATCH_PARENT
                topMargin = 0
            }
        }
        content.addView(box)
    }

    private fun caption(text: Int) = a.label(a.getString(text), 16f, p.label).apply {
        setPadding(0, a.dp(16), 0, a.dp(8))
    }

    private fun note(text: String) = a.label(text, 13f, p.secondary).apply {
        setLineSpacing(0f, 1.15f)
        setPadding(0, a.dp(4), 0, a.dp(4))
    }

    private fun homePageField(s: Settings): View {
        val error = note(a.getString(R.string.home_page_invalid)).apply {
            setTextColor(p.danger)
            visibility = View.GONE
        }
        val field = EditText(a).apply {
            setText(s.homePage)
            hint = a.getString(R.string.home_page_hint)
            setHintTextColor(p.tertiary)
            setTextColor(p.label)
            textSize = 16f
            isSingleLine = true
            background = rounded(p.field, a.dp(8))
            setPadding(a.dp(12), a.dp(10), a.dp(12), a.dp(10))
            imeOptions = EditorInfo.IME_ACTION_DONE or EditorInfo.IME_FLAG_NO_EXTRACT_UI
            inputType = android.text.InputType.TYPE_CLASS_TEXT or android.text.InputType.TYPE_TEXT_VARIATION_URI
        }
        fun commit() {
            val text = field.text.toString().trim()
            val valid = text.isEmpty() || Address.webUrl(text)?.let { it.length <= Address.URL_LIMIT } == true
            // Like the desktop: an invalid entry shows an error and keeps the typed text.
            error.visibility = if (valid) View.GONE else View.VISIBLE
            if (valid) update(JSONObject().put("HomePage", text))
        }
        field.setOnEditorActionListener { v, action, _ ->
            if (action == EditorInfo.IME_ACTION_DONE) {
                commit()
                if (error.visibility == View.GONE) v.clearFocus()
            }
            false
        }
        field.setOnFocusChangeListener { _, focused -> if (!focused) commit() }
        return a.column(field, error).also { field.layoutParams = LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT) }
    }

    private fun newTabPicture(s: Settings): View {
        val chosen = s.newTabImage.isNotEmpty()
        val thumb = ImageView(a).apply {
            scaleType = ImageView.ScaleType.CENTER_CROP
            background = rounded(p.raised, a.dp(6), p.line, a.dp(1))
            clipToOutline = true
            setImageBitmap(a.newTabPicture)
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }
        val choose = textButton(a.getString(if (chosen) R.string.picture_change else R.string.picture_choose), p.accent) { a.chooseNewTabPicture() }
        val row = a.row(thumb, View(a), choose)
        thumb.layoutParams = size(a.dp(64), a.dp(40))
        row.getChildAt(1).layoutParams = weight()
        if (chosen) row.addView(textButton(a.getString(R.string.picture_remove), p.danger) { a.removeNewTabPicture() })
        return a.column(row, note(a.getString(if (chosen) R.string.picture_set_note else R.string.picture_note)))
    }

    private fun textButton(text: String, color: Int, onClick: () -> Unit) = a.label(text, 16f, color, bold = true).apply {
        gravity = Gravity.CENTER
        minimumHeight = a.dp(48)
        setPadding(a.dp(12), 0, a.dp(12), 0)
        background = pressable(p)
        setOnClickListener { onClick() }
    }

    private fun swatches(selected: Accent): View {
        val row = a.row()
        for (accent in Accent.entries) {
            val color = Palette.accentColor(accent, p.dark)
            val chosen = accent == selected
            val dot = View(a).apply {
                background = rounded(color, a.dp(14), if (chosen) p.label else color, a.dp(if (chosen) 2.5f else 0f))
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
            }
            val cell = FrameLayout(a).apply {
                addView(dot, FrameLayout.LayoutParams(a.dp(28), a.dp(28), Gravity.CENTER))
                contentDescription = a.getString(when (accent) {
                    Accent.Blue -> R.string.accent_blue
                    Accent.Purple -> R.string.accent_purple
                    Accent.Pink -> R.string.accent_pink
                    Accent.Orange -> R.string.accent_orange
                    Accent.Green -> R.string.accent_green
                    Accent.Graphite -> R.string.accent_graphite
                })
                isSelected = chosen
                background = pressable(p)
                isFocusable = true
                setOnClickListener { update(JSONObject().put("Accent", accent.name)) }
            }
            row.addView(cell, weight(1f, a.dp(48)))
        }
        return row
    }

    private fun <T> preference(title: Int, options: List<Pair<String, T>>, selected: T, onPick: (T) -> Unit): View {
        val heading = a.label(a.getString(title), 16f, p.label)
        val value = a.label(options.firstOrNull { it.second == selected }?.first.orEmpty(), 14f, p.secondary).apply {
            setPadding(0, a.dp(4), 0, 0)
        }
        return a.column(heading, value).apply {
            gravity = Gravity.CENTER_VERTICAL
            isScreenReaderFocusable = true
            minimumHeight = a.dp(72)
            setPadding(0, a.dp(12), 0, a.dp(12))
            background = pressable(p)
            isFocusable = true
            setOnClickListener {
                AlertDialog.Builder(a)
                    .setTitle(title)
                    .setSingleChoiceItems(options.map { it.first }.toTypedArray(), options.indexOfFirst { it.second == selected }) { dialog, index ->
                        dialog.dismiss()
                        onPick(options[index].second)
                    }
                    .setNegativeButton(android.R.string.cancel, null)
                    .show()
            }
        }
    }

    private fun switchRow(text: String, checked: Boolean, onChange: (Boolean) -> Unit): View {
        val title = a.label(text, 16f, p.label)
        val toggle = Switch(a).apply {
            isChecked = checked
            contentDescription = text
            val states = arrayOf(intArrayOf(android.R.attr.state_checked), intArrayOf())
            thumbTintList = ColorStateList(states, intArrayOf(Color.WHITE, Color.WHITE))
            trackTintList = ColorStateList(states, intArrayOf(p.accent, p.tertiary))
            setOnCheckedChangeListener { _, value -> onChange(value) }
        }
        return a.row(title, toggle).apply {
            minimumHeight = a.dp(56)
            background = pressable(p)
            isFocusable = true
            setOnClickListener { toggle.isChecked = !toggle.isChecked }
            title.layoutParams = weight()
            toggle.layoutParams = size(WRAP_CONTENT, a.dp(48)).apply { marginStart = a.dp(16) }
        }
    }

    private fun checkRow(text: Int, checked: Boolean, onChange: (Boolean) -> Unit): View = CheckBox(a).apply {
        this.text = a.getString(text)
        isChecked = checked
        setTextColor(p.label)
        textSize = 16f
        minimumHeight = a.dp(48)
        buttonTintList = ColorStateList(
            arrayOf(intArrayOf(android.R.attr.state_checked), intArrayOf()),
            intArrayOf(p.accent, p.secondary),
        )
        setOnCheckedChangeListener { _, value -> onChange(value) }
    }
}

object BuildInfo {
    fun versionName(context: android.content.Context): String = try {
        context.packageManager.getPackageInfo(context.packageName, 0).versionName ?: "?"
    } catch (e: Exception) {
        "?"
    }
}
