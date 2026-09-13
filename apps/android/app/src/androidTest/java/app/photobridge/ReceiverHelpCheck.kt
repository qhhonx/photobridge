package app.photobridge

import android.app.Activity
import android.app.Application
import java.util.concurrent.atomic.AtomicReference
import android.app.Instrumentation
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.Rect
import android.os.Bundle
import android.view.View
import android.view.ViewGroup
import android.view.WindowManager
import android.widget.Button
import android.widget.ScrollView
import android.widget.TextView
import androidx.appcompat.app.AppCompatDelegate
import androidx.core.os.LocaleListCompat
import java.io.File

/** Isolated native help navigation. Never grants permission or modifies real receiver data. */
internal fun Instrumentation.checkReceiverHelp(args: Bundle): String {
    check(targetContext.packageName.endsWith(".validation")) { "validation_package_required" }
    val language = args.getString("language") ?: "en"
    val dark = args.getString("dark") == "true"
    val prefs = targetContext.getSharedPreferences("appearance", 0)
    val oldMode = prefs.getInt("mode", AppCompatDelegate.MODE_NIGHT_FOLLOW_SYSTEM)
    prefs.edit().putInt("mode", if (dark) AppCompatDelegate.MODE_NIGHT_YES else AppCompatDelegate.MODE_NIGHT_NO).commit()
    val oldLocales = AppCompatDelegate.getApplicationLocales()
    runOnMainSync { AppCompatDelegate.setApplicationLocales(LocaleListCompat.forLanguageTags(language)) }
    fun views(view: View): List<View> = listOf(view) + if (view is ViewGroup) (0 until view.childCount).flatMap { views(view.getChildAt(it)) } else emptyList()
    fun onMain(checks: () -> Unit) {
        var result: Result<Unit>? = null
        runOnMainSync { result = runCatching(checks) }
        checkNotNull(result).getOrThrow()
    }
    fun shot(name: String) {
        waitForIdleSync(); Thread.sleep(250)
        val image = checkNotNull(uiAutomation.takeScreenshot())
        File(targetContext.cacheDir, "help-$language-${if (dark) "dark" else "light"}-$name.png").outputStream().use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
        image.recycle()
    }
    val resumed = AtomicReference<ReceiverHelpActivity?>()
    val app = targetContext.applicationContext as Application
    val callbacks = object : Application.ActivityLifecycleCallbacks {
        override fun onActivityResumed(activity: Activity) { if (activity is ReceiverHelpActivity) resumed.set(activity) }
        override fun onActivityCreated(activity: Activity, state: Bundle?) = Unit
        override fun onActivityStarted(activity: Activity) = Unit
        override fun onActivityPaused(activity: Activity) = Unit
        override fun onActivityStopped(activity: Activity) = Unit
        override fun onActivitySaveInstanceState(activity: Activity, state: Bundle) = Unit
        override fun onActivityDestroyed(activity: Activity) { resumed.compareAndSet(activity as? ReceiverHelpActivity, null) }
    }
    app.registerActivityLifecycleCallbacks(callbacks)
    fun currentPage(topic: String?): ReceiverHelpActivity {
        for (attempt in 0 until 50) {
            val candidate = resumed.get()
            var ready = false
            onMain { ready = candidate != null && !candidate.isDestroyed && candidate.intent.getStringExtra("topic") == topic && candidate.window.decorView.hasWindowFocus() }
            if (ready) return checkNotNull(candidate)
            Thread.sleep(100)
        }
        error("help_page_not_resumed:$topic")
    }
    startActivitySync(Intent(targetContext, ReceiverHelpActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    var root = currentPage(null)
    var detail: Activity? = null
    try {
        runOnMainSync { root.window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON) }
        waitForIdleSync(); shot("topics")
        for (topic in ReceiverHelpTopic.entries) {
            run {
                root = currentPage(null)
                onMain {
                    val button = views(root.window.decorView).filterIsInstance<Button>().single { it.text.toString() == root.getString(topic.title) }
                    // Bring the actual control into the native viewport before activating it.
                    button.requestRectangleOnScreen(Rect(0, 0, button.width, button.height), true)
                    check(button.isEnabled)
                    button.performClick()
                }
                detail = currentPage(topic.key)
                waitForIdleSync()
                onMain {
                    detail!!.window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                    check(detail!!.intent.getStringExtra("topic") == topic.key) { "wrong_help_topic" }
                    val content = views(detail!!.window.decorView).filterIsInstance<TextView>().map { it.text.toString() }
                    topic.sections.forEach { (_, body) -> check(detail!!.getString(body) in content) { "missing_help_body:${topic.key}" } }
                }
                waitForIdleSync()
                var bottomVisible = false
                for (attempt in 0 until 30) {
                    onMain {
                        val scroll = views(detail!!.window.decorView).filterIsInstance<ScrollView>().single()
                        if (scroll.isLaidOut && scroll.hasWindowFocus()) scroll.fullScroll(View.FOCUS_DOWN)
                        val last = views(detail!!.window.decorView).filterIsInstance<TextView>().single { it.text.toString() == detail!!.getString(topic.sections.last().second) }
                        val rect = Rect()
                        val position = IntArray(2); last.getLocationOnScreen(position)
                        bottomVisible = last.getGlobalVisibleRect(rect) && rect.bottom >= position[1] + last.height - 1
                    }
                    if (bottomVisible) break
                    Thread.sleep(100)
                }
                shot(topic.key)
                check(bottomVisible) { "help_bottom_clipped:${topic.key}" }
                sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
                waitForIdleSync()
                detail = null
            }
        }
        for ((error, expected) in listOf("wifi_required" to ReceiverRecovery.NETWORK, "receiver_address_changed" to ReceiverRecovery.CONNECTION_HELP,
            "system_time_limit" to ReceiverRecovery.RESUME, "foreground_start_blocked" to ReceiverRecovery.RESUME,
            "processing_low_space" to ReceiverRecovery.STORAGE, "capacity" to ReceiverRecovery.STORAGE, "storage" to ReceiverRecovery.DIAGNOSTICS_HELP)) {
            check(receiverProblem(ReceiverSnapshot(phase = "waiting", error = error))?.recovery == expected)
        }
        check(receiverProblem(ReceiverSnapshot()) == null)
        return "PASS: five help destinations, full final-paragraph visibility, native back navigation and fixed recovery routes ($language, dark=$dark)"
    } finally {
        runOnMainSync { detail?.finish(); root.finish(); AppCompatDelegate.setApplicationLocales(oldLocales) }
        app.unregisterActivityLifecycleCallbacks(callbacks)
        prefs.edit().putInt("mode", oldMode).commit()
    }
}
