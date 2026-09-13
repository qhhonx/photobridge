package app.photobridge

import android.widget.*
import androidx.lifecycle.lifecycleScope
import com.google.android.material.dialog.MaterialAlertDialogBuilder
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

internal enum class StorageSection { LIMITS, LOGS }

/** Separate, small native dialogs. Rust validates and persists every change. */
internal fun MainActivity.showStorageControls(section: StorageSection, saved: () -> Unit) {
    lifecycleScope.launch {
        try {
            val state = withContext(Dispatchers.IO) {
                NativeBridge.request(JSONObject().put("op", "receiver_settings").put("root", "$filesDir/receiver")) as JSONObject
            }
            val settings = state.getJSONObject("settings")
            val panel = LinearLayout(this@showStorageControls).apply {
                orientation = LinearLayout.VERTICAL; setPadding(dp(24), dp(12), dp(24), dp(16))
            }
            fun options(title: Int, choices: List<Long>, current: Long, format: (Long) -> String): Spinner {
                label(panel, getString(title), 14, secondaryColor())
                val values = (choices + current).distinct().sorted()
                return Spinner(this@showStorageControls).also { spinner ->
                    spinner.contentDescription = getString(title)
                    spinner.adapter = ArrayAdapter(this@showStorageControls, android.R.layout.simple_spinner_dropdown_item, values.map(format))
                    spinner.setSelection(values.indexOf(current)); spinner.tag = values
                    panel.addView(spinner, LinearLayout.LayoutParams(-1, dp(52)))
                }
            }
            val controls = linkedMapOf<String, Spinner>()
            if (section == StorageSection.LIMITS) {
                controls["receiver_budget_bytes"] = options(R.string.storage_budget, listOf(2L, 4, 6, 10, 16, 32, 64, 128).map { it shl 30 }, settings.getLong("receiver_budget_bytes")) { android.text.format.Formatter.formatShortFileSize(this@showStorageControls, it) }
                controls["min_free_bytes"] = options(R.string.storage_reserve, listOf(0L, 1, 2, 3, 5, 10).map { it shl 30 }, settings.getLong("min_free_bytes")) { android.text.format.Formatter.formatShortFileSize(this@showStorageControls, it) }
            } else {
                controls["log_days"] = options(R.string.logs_days, listOf(1, 7, 14, 30, 90), settings.getLong("log_days")) { "$it ${getString(R.string.days_unit)}" }
                controls["log_limit"] = options(R.string.logs_limit, listOf(1000, 5000, 10000, 20000), settings.getLong("log_limit")) { it.toString() }
            }
            val dialog = MaterialAlertDialogBuilder(this@showStorageControls)
                .setTitle(if (section == StorageSection.LIMITS) R.string.storage_limits else R.string.logs_retention_settings)
                .setView(ScrollView(this@showStorageControls).apply { addView(panel) })
                .setNegativeButton(R.string.receiver_close, null).setPositiveButton(R.string.settings_save, null).create()
            dialog.setOnShowListener {
                val save = dialog.getButton(android.app.AlertDialog.BUTTON_POSITIVE)
                save.setOnClickListener {
                    controls.forEach { (key, spinner) ->
                        val selected = (spinner.tag as List<*>)[spinner.selectedItemPosition] as Long
                        settings.put(key, selected)
                    }
                    save.isEnabled = false
                    lifecycleScope.launch {
                        val result = withContext(Dispatchers.IO) { runCatching {
                            NativeBridge.request(JSONObject().put("op", "receiver_settings").put("root", "$filesDir/receiver").put("settings", settings))
                        } }
                        if (result.isSuccess) { dialog.dismiss(); saved() }
                        else { save.isEnabled = true; Toast.makeText(this@showStorageControls, R.string.settings_failed, Toast.LENGTH_LONG).show() }
                    }
                }
            }
            dialog.show()
        } catch (_: Exception) { Toast.makeText(this@showStorageControls, R.string.settings_failed, Toast.LENGTH_LONG).show() }
    }
}
