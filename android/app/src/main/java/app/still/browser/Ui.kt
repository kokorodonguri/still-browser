package app.still.browser

import android.content.Context
import android.content.res.ColorStateList
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.ColorDrawable
import android.graphics.drawable.Drawable
import android.graphics.drawable.GradientDrawable
import android.graphics.drawable.RippleDrawable
import android.util.TypedValue
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.ImageButton
import android.widget.LinearLayout
import android.widget.TextView

/** Android chrome colors inspired by Firefox's Acorn palette. */
class Palette(val dark: Boolean, accent: Accent) {
    val canvas = if (dark) 0xFF1C1B22.toInt() else 0xFFF9F9FB.toInt()
    val surface = if (dark) 0xFF2B2A33.toInt() else 0xFFFFFFFF.toInt()
    val field = if (dark) 0xFF42414D.toInt() else 0xFFF0F0F4.toInt()
    val raised = if (dark) 0x14FBFBFE else 0x0C15141A
    val pressed = if (dark) 0x24FBFBFE else 0x1A15141A
    val label = if (dark) 0xFFFBFBFE.toInt() else 0xFF15141A.toInt()
    val secondary = if (dark) 0xFFCFCFD8.toInt() else 0xFF5B5B66.toInt()
    val tertiary = 0xFF8F8F9D.toInt()
    val line = if (dark) 0xFF52525E.toInt() else 0xFFE0E0E6.toInt()
    val danger = if (dark) 0xFFFF848B.toInt() else 0xFFD7264C.toInt()
    val accent = accentColor(accent, dark)

    companion object {
        fun accentColor(accent: Accent, dark: Boolean): Int = when (accent) {
            Accent.Blue -> if (dark) 0xFF45A1FF else 0xFF0060DF
            Accent.Purple -> if (dark) 0xFFAB71FF else 0xFF7542E5
            Accent.Pink -> if (dark) 0xFFFF6BBA else 0xFFE31587
            Accent.Orange -> if (dark) 0xFFFFA436 else 0xFFE25920
            Accent.Green -> if (dark) 0xFF3FE1B0 else 0xFF017A40
            Accent.Graphite -> if (dark) 0xFFCFCFD8 else 0xFF5B5B66
        }.toInt()
    }
}

fun Context.dp(value: Number): Int =
    TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_DIP, value.toFloat(), resources.displayMetrics).toInt()

fun rounded(color: Int, radius: Int, stroke: Int = 0, strokeWidth: Int = 0) = GradientDrawable().apply {
    setColor(color)
    cornerRadius = radius.toFloat()
    if (strokeWidth > 0) setStroke(strokeWidth, stroke)
}

/** A background that darkens while pressed, clipped to [radius]. */
fun pressable(p: Palette, background: Int = Color.TRANSPARENT, radius: Int = 0): Drawable =
    RippleDrawable(ColorStateList.valueOf(p.pressed), rounded(background, radius), rounded(Color.WHITE, radius))

fun Context.label(text: CharSequence = "", size: Float = 17f, color: Int, bold: Boolean = false) = TextView(this).apply {
    this.text = text
    setTextSize(TypedValue.COMPLEX_UNIT_SP, size)
    setTextColor(color)
    if (bold) typeface = Typeface.create(Typeface.DEFAULT, Typeface.BOLD)
    includeFontPadding = false
}

fun Context.iconButton(icon: Int, description: Int, p: Palette, onClick: (View) -> Unit) = ImageButton(this).apply {
    setImageResource(icon)
    contentDescription = context.getString(description)
    imageTintList = ColorStateList.valueOf(p.label)
    background = pressable(p, radius = dp(8))
    scaleType = android.widget.ImageView.ScaleType.CENTER
    isFocusable = true
    setOnClickListener(onClick)
}

fun Context.row(vararg children: View, gravity: Int = Gravity.CENTER_VERTICAL) = LinearLayout(this).apply {
    orientation = LinearLayout.HORIZONTAL
    this.gravity = gravity
    children.forEach(::addView)
}

fun Context.column(vararg children: View) = LinearLayout(this).apply {
    orientation = LinearLayout.VERTICAL
    children.forEach(::addView)
}

fun weight(w: Float = 1f, height: Int = ViewGroup.LayoutParams.WRAP_CONTENT) =
    LinearLayout.LayoutParams(0, height, w)

fun size(width: Int, height: Int) = LinearLayout.LayoutParams(width, height)

fun hairline(context: Context, p: Palette) = View(context).apply {
    background = ColorDrawable(p.line)
    layoutParams = LinearLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, maxOf(1, context.dp(0.5)))
}
