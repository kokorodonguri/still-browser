package app.still.browser

import kotlin.math.abs

/** Coordinates and thresholds use the same units (pixels in the Android view). */
internal class AddressSwipeGesture(private val threshold: Float, private val touchSlop: Float) {
    enum class Result { None, Tap, Previous, Next, Up }

    private var startX = 0f
    private var startY = 0f
    private var active = false
    private var moved = false

    fun start(x: Float, y: Float) {
        startX = x
        startY = y
        active = true
        moved = false
    }

    fun move(x: Float, y: Float) {
        if (abs(x - startX) > touchSlop || abs(y - startY) > touchSlop) moved = true
    }

    fun cancel() { active = false }

    fun finish(x: Float, y: Float): Result {
        if (!active) return Result.None
        active = false
        move(x, y)
        val dx = x - startX
        val dy = y - startY
        return when {
            abs(dx) >= threshold && abs(dx) >= abs(dy) * 1.5f ->
                if (dx < 0) Result.Next else Result.Previous
            -dy >= threshold && abs(dy) >= abs(dx) * 1.5f -> Result.Up
            !moved -> Result.Tap
            else -> Result.None
        }
    }
}
