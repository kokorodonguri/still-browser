package app.still.browser

import app.still.browser.AddressSwipeGesture.Result
import org.junit.Assert.assertEquals
import org.junit.Test

class AddressSwipeGestureTest {
    private val gesture = AddressSwipeGesture(50f, 8f)

    @Test fun horizontalSwipesMoveOneTabOnRelease() {
        gesture.start(100f, 0f)
        gesture.move(0f, 0f)
        assertEquals(Result.Next, gesture.finish(-100f, 0f))
        assertEquals(Result.None, gesture.finish(-100f, 0f))
        gesture.start(0f, 0f)
        assertEquals(Result.Previous, gesture.finish(50f, 0f))
    }

    @Test fun shortVerticalAndDiagonalDragsDoNotSwitchOrClick() {
        for ((x, y) in listOf(49f to 0f, 0f to 100f, 60f to 41f)) {
            gesture.start(0f, 0f)
            assertEquals(Result.None, gesture.finish(x, y))
        }
        gesture.start(0f, 0f)
        assertEquals(Result.Previous, gesture.finish(60f, 40f))
    }

    @Test fun upwardSwipeOpensTabsButDownwardAndDiagonalDragsDoNot() {
        gesture.start(0f, 100f)
        assertEquals(Result.Up, gesture.finish(0f, 50f))
        gesture.start(0f, 0f)
        assertEquals(Result.Up, gesture.finish(40f, -60f))
        for ((x, y) in listOf(0f to -49f, 0f to 100f, 60f to -60f, -50f to -70f)) {
            gesture.start(0f, 0f)
            assertEquals(Result.None, gesture.finish(x, y))
        }
    }

    @Test fun tapToleratesFingerJitterButReturningDragIsNotATap() {
        gesture.start(100f, 100f)
        assertEquals(Result.Tap, gesture.finish(104f, 103f))
        gesture.start(100f, 100f)
        gesture.move(0f, 100f)
        assertEquals(Result.None, gesture.finish(100f, 100f))
    }

    @Test fun cancellationAndMultiTouchInvalidateTheWholeGesture() {
        gesture.start(100f, 0f)
        gesture.cancel()
        gesture.move(0f, 0f)
        assertEquals(Result.None, gesture.finish(0f, 0f))
        gesture.start(100f, 0f)
        assertEquals(Result.Next, gesture.finish(0f, 0f))
    }

    @Test fun thresholdScalesWithDensity() {
        val dense = AddressSwipeGesture(150f, 24f)
        dense.start(0f, 0f)
        assertEquals(Result.None, dense.finish(149f, 0f))
        dense.start(0f, 0f)
        assertEquals(Result.Previous, dense.finish(150f, 0f))
    }
}
