package app.still.browser

import android.content.Context
import org.mozilla.geckoview.GeckoRuntime
import org.mozilla.geckoview.GeckoRuntimeSettings
import org.mozilla.geckoview.GeckoSession
import org.mozilla.geckoview.GeckoSessionSettings

/** Firefox's engine (GeckoView). One runtime serves every tab for the life of the process. */
object Engine {
    private var runtime: GeckoRuntime? = null

    fun runtime(context: Context): GeckoRuntime = runtime ?: GeckoRuntime.create(
        context.applicationContext,
        GeckoRuntimeSettings.Builder()
            .remoteDebuggingEnabled(false)
            .aboutConfigEnabled(false)
            .consoleOutput(false)
            // Android's autofill service (Google Password Manager…) fills logins instead.
            .loginAutofillEnabled(false)
            .build(),
    ).also { runtime = it }

    /** Pages follow the app's theme through `prefers-color-scheme`. */
    fun setTheme(context: Context, theme: Theme) {
        runtime(context).settings.preferredColorScheme = when (theme) {
            Theme.System -> GeckoRuntimeSettings.COLOR_SCHEME_SYSTEM
            Theme.Light -> GeckoRuntimeSettings.COLOR_SCHEME_LIGHT
            Theme.Dark -> GeckoRuntimeSettings.COLOR_SCHEME_DARK
        }
    }

    fun newSession(): GeckoSession = GeckoSession(
        GeckoSessionSettings.Builder()
            .useTrackingProtection(true)
            .suspendMediaWhenInactive(true)
            .build(),
    )

    /** The engine's version, for the About section. */
    val version: String get() = org.mozilla.geckoview.BuildConfig.MOZ_APP_VERSION
}
