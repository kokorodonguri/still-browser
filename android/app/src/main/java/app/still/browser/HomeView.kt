package app.still.browser

import android.text.TextUtils
import android.view.Gravity
import android.view.View
import android.view.ViewGroup.LayoutParams.MATCH_PARENT
import android.view.ViewGroup.LayoutParams.WRAP_CONTENT
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.ScrollView

/** Home content stays above the middle of the chosen background, even on short screens. */
@android.annotation.SuppressLint("ViewConstructor")
class HomeView(private val a: MainActivity) : FrameLayout(a) {
    private val p = a.palette
    private val items = a.column()
    private var shown: Pair<List<Shortcut>, Boolean>? = null

    init {
        addView(object : ScrollView(a) {
            override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
                val height = (MeasureSpec.getSize(heightMeasureSpec) * 0.44f).toInt()
                super.onMeasure(widthMeasureSpec, MeasureSpec.makeMeasureSpec(height, MeasureSpec.AT_MOST))
            }
        }.apply {
            clipToPadding = false
            addView(items)
        }, LayoutParams(MATCH_PARENT, WRAP_CONTENT).apply { setMargins(a.dp(16), a.dp(12), a.dp(16), 0) })
    }

    fun update(shortcuts: List<Shortcut>, picture: Boolean) {
        val next = shortcuts to picture
        if (shown == next) return
        shown = next
        items.removeAllViews()
        items.setPadding(a.dp(12), a.dp(8), a.dp(12), a.dp(8))
        items.background = if (picture) rounded((p.surface and 0x00FFFFFF) or 0xE6000000.toInt(), a.dp(16)) else null
        val logo = ImageView(a).apply {
            setImageResource(R.mipmap.ic_launcher)
            importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
        }
        items.addView(a.row(logo, a.label(a.getString(R.string.app_name), 28f, p.label, bold = true)).apply {
            logo.layoutParams = size(a.dp(36), a.dp(36)).apply { marginEnd = a.dp(12) }
        }, LinearLayout.LayoutParams(MATCH_PARENT, a.dp(56)))
        if (shortcuts.isEmpty()) return
        items.addView(a.label(a.getString(R.string.shortcuts), 16f, p.label, bold = true).apply {
            isAccessibilityHeading = true
            gravity = Gravity.CENTER_VERTICAL
        }, LinearLayout.LayoutParams(MATCH_PARENT, a.dp(36)))
        for (group in shortcuts.chunked(4)) {
            val row = a.row(gravity = Gravity.TOP)
            for (shortcut in group) {
                val initial = a.label(shortcut.title.take(1).uppercase(), 24f, p.label, bold = true).apply {
                    gravity = Gravity.CENTER
                    background = rounded(p.field, a.dp(12))
                    importantForAccessibility = View.IMPORTANT_FOR_ACCESSIBILITY_NO
                }
                val title = a.label(shortcut.title, 12f, p.label).apply {
                    gravity = Gravity.CENTER
                    maxLines = 1
                    ellipsize = TextUtils.TruncateAt.END
                    setPadding(0, a.dp(6), 0, 0)
                }
                row.addView(a.column(initial, title).apply {
                    gravity = Gravity.CENTER_HORIZONTAL
                    background = pressable(p, radius = a.dp(8))
                    setPadding(a.dp(4), a.dp(6), a.dp(4), a.dp(6))
                    isFocusable = true
                    setOnClickListener { a.open(shortcut.url) }
                    initial.layoutParams = size(a.dp(48), a.dp(48))
                    title.layoutParams = LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT)
                }, weight())
            }
            repeat(4 - group.size) { row.addView(View(a), weight()) }
            items.addView(row)
        }
    }
}
