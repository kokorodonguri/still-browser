package app.still.browser

import android.content.res.ColorStateList
import android.graphics.Color
import android.text.TextUtils
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.view.ViewGroup.LayoutParams.MATCH_PARENT
import android.view.ViewGroup.LayoutParams.WRAP_CONTENT
import android.widget.BaseAdapter
import android.widget.FrameLayout
import android.widget.GridView
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView

/** A dimmed backdrop and a surface attached to the bottom of the available screen. */
@android.annotation.SuppressLint("ViewConstructor")
class BrowserSheet(a: MainActivity, tall: Boolean, onClose: () -> Unit) : FrameLayout(a) {
    val body = object : LinearLayout(a) {
        override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
            val height = (MeasureSpec.getSize(heightMeasureSpec) * 0.9f).toInt()
            super.onMeasure(widthMeasureSpec, MeasureSpec.makeMeasureSpec(height, if (tall) MeasureSpec.EXACTLY else MeasureSpec.AT_MOST))
        }
    }.apply {
        orientation = LinearLayout.VERTICAL
        background = rounded(a.palette.surface, 0).apply {
            val radius = a.dp(16).toFloat()
            cornerRadii = floatArrayOf(radius, radius, radius, radius, 0f, 0f, 0f, 0f)
        }
        clipToOutline = true
        isClickable = true
        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        elevation = a.dp(12).toFloat()
    }

    init {
        addView(View(a).apply {
            setBackgroundColor(0x66000000)
            contentDescription = a.getString(R.string.dismiss)
            isFocusable = true
            setOnClickListener { onClose() }
        }, LayoutParams(MATCH_PARENT, MATCH_PARENT))
        addView(body, LayoutParams(MATCH_PARENT, WRAP_CONTENT, Gravity.BOTTOM))
        body.addView(FrameLayout(a).apply {
            addView(View(a).apply {
                background = rounded(a.palette.tertiary, a.dp(2))
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
            }, LayoutParams(a.dp(32), a.dp(4), Gravity.CENTER))
        }, LinearLayout.LayoutParams(MATCH_PARENT, a.dp(24)))
        accessibilityPaneTitle = a.getString(if (tall) R.string.tabs else R.string.menu)
    }
}

class MenuSheet(a: MainActivity, onBookmark: () -> Unit, onShare: () -> Unit, onReload: () -> Unit) {
    private val p = a.palette
    val view = BrowserSheet(a, false) { a.closeMenu() }

    init {
        val items = a.column()
        val web = Address.isWeb(a.tabs.active.url)
        val marked = a.tabs.active.url in a.bookmarks
        val actions = a.row()
        for ((icon, title, action) in listOf(
            Triple(if (marked) R.drawable.ic_star_filled else R.drawable.ic_star,
                if (marked) R.string.bookmark_remove else R.string.bookmark_add, onBookmark),
            Triple(R.drawable.ic_share, R.string.share, onShare),
            Triple(R.drawable.ic_reload, R.string.reload, onReload),
        )) {
            val image = ImageView(a).apply {
                setImageResource(icon)
                imageTintList = ColorStateList.valueOf(p.label)
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
            }
            val text = a.label(a.getString(title), 12f, p.label).apply {
                gravity = Gravity.CENTER
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
            }
            actions.addView(a.column(image, text).apply {
                gravity = Gravity.CENTER
                setPadding(a.dp(4), a.dp(8), a.dp(4), a.dp(8))
                contentDescription = a.getString(title)
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_YES
                isFocusable = true
                isEnabled = web
                alpha = if (web) 1f else 0.35f
                background = pressable(p, radius = a.dp(8))
                setOnClickListener { a.closeMenu(); action() }
                image.layoutParams = size(a.dp(24), a.dp(24))
                text.layoutParams = LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT).apply { topMargin = a.dp(6) }
            }, weight(1f, a.dp(88)))
        }
        items.addView(actions)
        items.addView(hairline(a, p))
        for ((title, icon, action) in listOf(
            Triple(R.string.new_tab, R.drawable.ic_plus) { a.newTab() },
            Triple(R.string.bookmarks, R.drawable.ic_bookmarks) { a.openPanel(PanelKind.Bookmarks) },
            Triple(R.string.history, R.drawable.ic_history) { a.openPanel(PanelKind.History) },
            Triple(R.string.downloads, R.drawable.ic_download) { a.openPanel(PanelKind.Downloads) },
            Triple(R.string.settings, R.drawable.ic_settings) { a.openPanel(PanelKind.Settings) },
        )) {
            val image = ImageView(a).apply {
                setImageResource(icon)
                imageTintList = ColorStateList.valueOf(p.label)
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
            }
            items.addView(a.row(image, a.label(a.getString(title), 16f, p.label)).apply {
                setPadding(a.dp(20), 0, a.dp(20), 0)
                background = pressable(p)
                isFocusable = true
                isScreenReaderFocusable = true
                setOnClickListener { a.closeMenu(); action() }
                image.layoutParams = size(a.dp(24), a.dp(24)).apply { marginEnd = a.dp(24) }
            }, LinearLayout.LayoutParams(MATCH_PARENT, a.dp(56)))
        }
        view.body.addView(ScrollView(a).apply { addView(items) })
    }
}

class TabsPanel(private val a: MainActivity) : Panel(PanelKind.Tabs) {
    private val p = a.palette
    private val title = a.label("", 20f, p.label, bold = true).apply {
        setPadding(a.dp(20), 0, 0, 0)
        text = a.getString(R.string.tabs)
        gravity = Gravity.CENTER_VERTICAL
        isAccessibilityHeading = true
    }
    private val count = a.label("", 14f, p.secondary).apply {
        gravity = Gravity.CENTER_VERTICAL
        setPadding(a.dp(12), 0, a.dp(8), 0)
    }
    private val adapter = Cards()
    private val grid = GridView(a).apply {
        numColumns = if (a.resources.configuration.screenWidthDp >= 600) (a.resources.configuration.screenWidthDp / 200).coerceIn(3, 4) else 2
        horizontalSpacing = a.dp(12)
        verticalSpacing = a.dp(12)
        setPadding(a.dp(16), a.dp(12), a.dp(16), a.dp(96))
        clipToPadding = false
        stretchMode = GridView.STRETCH_COLUMN_WIDTH
        selector = android.graphics.drawable.ColorDrawable(Color.TRANSPARENT)
        adapter = this@TabsPanel.adapter
    }
    override val view = BrowserSheet(a, true) { a.closePanel() }

    init {
        val dismiss = a.iconButton(R.drawable.ic_close, R.string.dismiss, p) { a.closePanel() }
        view.body.addView(a.row(title, count, dismiss).apply {
            setPadding(0, 0, a.dp(8), 0)
            title.layoutParams = weight()
            count.layoutParams = size(WRAP_CONTENT, MATCH_PARENT)
            dismiss.layoutParams = size(a.dp(48), a.dp(48))
        }, LinearLayout.LayoutParams(MATCH_PARENT, a.dp(48)))
        view.body.addView(FrameLayout(a).apply {
            addView(grid, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT))
            addView(a.label(a.getString(R.string.new_tab), 16f, Color.WHITE, bold = true).apply {
                gravity = Gravity.CENTER
                setPadding(a.dp(20), 0, a.dp(20), 0)
                setCompoundDrawablesRelativeWithIntrinsicBounds(R.drawable.ic_plus, 0, 0, 0)
                compoundDrawablePadding = a.dp(8)
                val darkInk = 0xFF15141A.toInt()
                val accentLuminance = Color.luminance(p.accent)
                val darkContrast = (accentLuminance + 0.05f) / (Color.luminance(darkInk) + 0.05f)
                val lightContrast = 1.05f / (accentLuminance + 0.05f)
                val ink = if (darkContrast > lightContrast) darkInk else Color.WHITE
                setTextColor(ink)
                compoundDrawableTintList = ColorStateList.valueOf(ink)
                background = pressable(p, p.accent, a.dp(16))
                elevation = a.dp(6).toFloat()
                isFocusable = true
                setOnClickListener { a.newTab() }
            }, FrameLayout.LayoutParams(WRAP_CONTENT, a.dp(56), Gravity.BOTTOM or Gravity.END).apply {
                setMargins(a.dp(16), a.dp(16), a.dp(16), a.dp(16))
            })
        }, LinearLayout.LayoutParams(MATCH_PARENT, 0, 1f))
        onModelChanged()
        grid.setSelection(a.tabs.indexOf(a.tabs.activeId))
    }

    override fun onModelChanged() {
        val total = a.tabs.list.size
        count.text = a.resources.getQuantityString(R.plurals.tabs_count, total, total)
        adapter.notifyDataSetChanged()
    }

    private inner class Cards : BaseAdapter() {
        override fun getCount() = a.tabs.list.size
        override fun getItem(position: Int) = a.tabs.list[position]
        override fun getItemId(position: Int) = getItem(position).id
        override fun hasStableIds() = true

        override fun getView(position: Int, convertView: View?, parent: ViewGroup): View {
            val tab = getItem(position)
            val name = if (tab.url == Address.BLANK) a.getString(R.string.new_tab) else tab.title.ifBlank { a.titleFor(tab.url) }
            val bitmap = a.thumbnailFor(tab.id)
            val preview = FrameLayout(a).apply {
                setBackgroundColor(p.field)
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
                if (bitmap != null) addView(ImageView(a).apply {
                    scaleType = ImageView.ScaleType.CENTER_CROP
                    setImageBitmap(bitmap)
                    importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
                }, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT)) else {
                    val initial = a.label(if (tab.url == Address.BLANK) a.getString(R.string.app_name).take(1) else name.take(1).uppercase(), 32f, p.accent, bold = true)
                    val host = a.label(if (tab.url == Address.BLANK) a.getString(R.string.new_tab) else Address.hostLabel(tab.url), 12f, p.secondary).apply {
                        maxLines = 1
                        ellipsize = TextUtils.TruncateAt.END
                    }
                    addView(a.column(initial, host).apply {
                        gravity = Gravity.CENTER
                        importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
                        setPadding(a.dp(8), 0, a.dp(8), 0)
                    }, FrameLayout.LayoutParams(MATCH_PARENT, MATCH_PARENT))
                }
            }
            val text = a.label(name, 14f, p.label).apply {
                maxLines = 1
                ellipsize = TextUtils.TruncateAt.END
                importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
            }
            val close = a.iconButton(R.drawable.ic_close, R.string.close_tab, p) { a.closeTab(tab.id) }.apply {
                contentDescription = a.getString(R.string.close_tab_named, name)
            }
            val footer = a.row(text, close).apply {
                setPadding(a.dp(10), 0, 0, 0)
                text.layoutParams = weight()
                close.layoutParams = size(a.dp(48), a.dp(48))
            }
            val selected = tab.id == a.tabs.activeId
            return a.column(preview, footer).apply {
                // Inset children so the selected outline remains visible on every edge.
                background = rounded(p.canvas, a.dp(12), if (selected) p.accent else p.line, a.dp(if (selected) 2 else 1))
                foreground = pressable(p, radius = a.dp(12))
                setPadding(a.dp(2), a.dp(2), a.dp(2), a.dp(2))
                clipToOutline = true
                // GridView resets each child's selected state to its own selection, so report the active tab directly.
                accessibilityDelegate = object : View.AccessibilityDelegate() {
                    override fun onInitializeAccessibilityNodeInfo(host: View, info: android.view.accessibility.AccessibilityNodeInfo) {
                        super.onInitializeAccessibilityNodeInfo(host, info)
                        info.isSelected = selected
                    }
                }
                contentDescription = a.getString(if (bitmap == null) R.string.tab_placeholder else R.string.tab_preview, name)
                isScreenReaderFocusable = true
                isFocusable = true
                setOnClickListener { a.selectTab(tab.id); a.closePanel() }
                preview.layoutParams = LinearLayout.LayoutParams(MATCH_PARENT, a.dp(132))
                layoutParams = android.widget.AbsListView.LayoutParams(MATCH_PARENT, WRAP_CONTENT)
            }
        }
    }
}
