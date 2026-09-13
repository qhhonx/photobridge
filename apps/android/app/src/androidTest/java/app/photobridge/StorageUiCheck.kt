package app.photobridge

import android.app.Instrumentation
import android.content.Intent
import android.graphics.Bitmap
import android.os.Bundle
import android.view.View
import android.view.ViewGroup
import android.widget.ScrollView
import android.widget.TextView
import androidx.appcompat.app.AppCompatDelegate
import androidx.core.os.LocaleListCompat
import com.google.android.material.bottomnavigation.BottomNavigationView
import java.io.File
import org.json.JSONObject

/** Production views in the isolated validation app; no user-library fixtures. */
internal fun Instrumentation.checkStorageUI(args: Bundle): String {
    check(targetContext.packageName.endsWith(".validation")) { "validation_package_required" }
    val language = args.getString("language") ?: "en"
    val dark = args.getString("dark") == "true"
    targetContext.getSharedPreferences("appearance", 0).edit().putInt("mode", if (dark) AppCompatDelegate.MODE_NIGHT_YES else AppCompatDelegate.MODE_NIGHT_NO).commit()
    ReceiverPreferences.setEnabled(targetContext, false)
    val settingsRoot = "${targetContext.filesDir}/receiver"
    fun config() = (NativeBridge.request(JSONObject().put("op", "receiver_settings").put("root", settingsRoot)) as JSONObject).getJSONObject("settings")
    fun save(value: JSONObject) { NativeBridge.request(JSONObject().put("op", "receiver_settings").put("root", settingsRoot).put("settings", value)) }
    val previous = config()
    save(JSONObject(previous.toString()).put("receiver_budget_bytes", 123_456_789L).put("min_free_bytes", 456_789L).put("receiver_relay", false))
    runOnMainSync { AppCompatDelegate.setApplicationLocales(LocaleListCompat.forLanguageTags(language)) }
    val activity = startActivitySync(Intent(targetContext, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) as MainActivity
    try {
        fun views(view: View): List<View> = listOf(view) + if (view is ViewGroup) (0 until view.childCount).flatMap { views(view.getChildAt(it)) } else emptyList()
        fun visibleViews() = views(activity.window.decorView).filter { it.isShown }
        fun text(id: Int) = activity.getString(id)
        fun screenshot(name: String) {
            waitForIdleSync(); Thread.sleep(750) // Include the final dialog animation frame.
            val bitmap = checkNotNull(uiAutomation.takeScreenshot())
            File(targetContext.cacheDir, "storage-$language-${if (dark) "dark" else "light"}-$name.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
            bitmap.recycle()
        }
        runOnMainSync { activity.findViewById<BottomNavigationView>(R.id.main_navigation).selectedItemId = 3 }
        var ready = false
        for (attempt in 0 until 100) {
            runOnMainSync { ready = visibleViews().filterIsInstance<TextView>().any { it.text.toString() == text(R.string.storage_snapshot_note) } }
            if (ready) break
            Thread.sleep(100)
        }
        check(ready) { "storage_snapshot_not_rendered" }
        waitForIdleSync(); screenshot("top")
        runOnMainSync { visibleViews().filterIsInstance<TextView>().first { it.text.toString() == text(R.string.storage_limits) }.performClick() }
        waitForIdleSync(); Thread.sleep(700)
        // Check real option selection and save instead of rebuilding settings widgets.
        val roots = uiAutomation.rootInActiveWindow
        check(roots != null && roots.packageName.toString() == targetContext.packageName) { "storage_limits_not_open" }
        screenshot("limits")
        val button = roots.findAccessibilityNodeInfosByText(text(R.string.settings_save)).first { it.isClickable }
        check(button.performAction(android.view.accessibility.AccessibilityNodeInfo.ACTION_CLICK)) { "save_not_clicked" }
        var closed = false
        for (attempt in 0 until 50) {
            if (uiAutomation.rootInActiveWindow?.findAccessibilityNodeInfosByText(text(R.string.settings_save))?.none { it.isClickable && it.className.toString().endsWith("Button") } == true) { closed = true; break }
            Thread.sleep(100)
        }
        check(closed) { "settings_not_saved" }
        check(config().getLong("receiver_budget_bytes") == 123_456_789L && config().getLong("min_free_bytes") == 456_789L) { "custom_limit_was_rounded" }
        runOnMainSync { visibleViews().filterIsInstance<ScrollView>().first().fullScroll(View.FOCUS_DOWN) }
        waitForIdleSync(); Thread.sleep(700); screenshot("bottom")
        val relaySwitch = visibleViews().filterIsInstance<com.google.android.material.materialswitch.MaterialSwitch>().single()
        runOnMainSync { relaySwitch.performClick() }
        waitForIdleSync(); screenshot("relay-confirm")
        check(!config().getBoolean("receiver_relay")) { "relay_enabled_before_confirmation" }
        sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
        check(!config().getBoolean("receiver_relay")) { "cancelled_relay_enabled" }
        runOnMainSync { relaySwitch.performClick() }
        waitForIdleSync(); Thread.sleep(700)
        val enable = uiAutomation.rootInActiveWindow.findAccessibilityNodeInfosByText(text(R.string.relay_enable)).first { it.isClickable && it.className.toString().endsWith("Button") }
        check(enable.performAction(android.view.accessibility.AccessibilityNodeInfo.ACTION_CLICK))
        var enabled = false
        for (attempt in 0 until 50) { enabled = config().getBoolean("receiver_relay"); if (enabled) break; Thread.sleep(100) }
        check(enabled) { "relay_switch_not_saved" }
        waitForIdleSync(); screenshot("relay-enabled")
        runOnMainSync { relaySwitch.performClick() }
        var disabled = false
        for (attempt in 0 until 50) { disabled = !config().getBoolean("receiver_relay"); if (disabled) break; Thread.sleep(100) }
        check(disabled) { "relay_switch_not_disabled" }
        runOnMainSync { visibleViews().filterIsInstance<TextView>().first { it.text.toString() == text(R.string.storage_how_to_free) }.performClick() }
        waitForIdleSync(); screenshot("help")
        sendKeyDownUpSync(android.view.KeyEvent.KEYCODE_BACK)
        return "PASS: native relay consent/cancel/enable/disable, storage page, exact custom limits and retention help rendered ($language, dark=$dark)"
    } finally {
        save(previous)
        runOnMainSync { activity.finish() }
    }
}
