package app.still.browser

import android.content.Context
import android.view.MotionEvent
import android.view.ViewConfiguration
import android.view.inputmethod.InputMethodManager
import android.widget.EditText

/** Waits for an unfocused touch to finish before opening the URL editor. */
internal class SwipeAddressEditText(context: Context) : EditText(context) {
    var onSwipe: (Int) -> Unit = {}
    var onSwipeUp: () -> Unit = {}
    private val gesture = AddressSwipeGesture(
        50f * resources.displayMetrics.density,
        ViewConfiguration.get(context).scaledTouchSlop.toFloat(),
    )
    private var tracking = false

    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (event.actionMasked == MotionEvent.ACTION_DOWN) {
            tracking = !hasFocus() && event.pointerCount == 1
            gesture.cancel()
            if (tracking) gesture.start(event.x, event.y)
        }
        // Once editing, retain EditText's cursor, selection and scrolling gestures.
        if (!tracking) return super.onTouchEvent(event)
        if (event.pointerCount > 1 || hasFocus()) gesture.cancel()
        when (event.actionMasked) {
            MotionEvent.ACTION_MOVE -> gesture.move(event.x, event.y)
            MotionEvent.ACTION_CANCEL -> {
                gesture.cancel()
                tracking = false
            }
            MotionEvent.ACTION_UP -> {
                tracking = false
                when (gesture.finish(event.x, event.y)) {
                    AddressSwipeGesture.Result.Previous -> onSwipe(-1)
                    AddressSwipeGesture.Result.Next -> onSwipe(1)
                    AddressSwipeGesture.Result.Up -> onSwipeUp()
                    AddressSwipeGesture.Result.Tap -> performClick()
                    AddressSwipeGesture.Result.None -> Unit
                }
            }
        }
        return true
    }

    override fun performClick(): Boolean {
        super.performClick()
        requestFocus()
        context.getSystemService(InputMethodManager::class.java).showSoftInput(this, 0)
        return true
    }

    override fun onDetachedFromWindow() {
        gesture.cancel()
        tracking = false
        super.onDetachedFromWindow()
    }
}
