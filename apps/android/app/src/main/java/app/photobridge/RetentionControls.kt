package app.photobridge

import android.widget.LinearLayout
import android.widget.Toast
import androidx.lifecycle.lifecycleScope
import com.google.android.material.dialog.MaterialAlertDialogBuilder
import com.google.android.material.materialswitch.MaterialSwitch
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

internal class RetentionControls(private val activity: MainActivity, parent: LinearLayout) {
    private var applying = false
    private var saving = false
    private val toggle = MaterialSwitch(activity).apply {
        setText(R.string.relay_toggle); minimumHeight = activity.dp(56)
    }
    init {
        parent.addView(toggle)
        activity.label(parent, activity.getString(R.string.relay_note), 14, activity.secondaryColor())
        toggle.setOnCheckedChangeListener { _, checked ->
            if (!applying && !saving) {
                if (checked) {
                    render(false)
                    MaterialAlertDialogBuilder(activity).setTitle(R.string.relay_confirm_title)
                        .setMessage(R.string.relay_confirm_note)
                        .setNegativeButton(R.string.receiver_close, null)
                        .setPositiveButton(R.string.relay_enable) { _, _ -> save(true) }.show()
                } else save(false)
            }
        }
    }
    fun render(enabled: Boolean) {
        if (saving) return
        applying = true; toggle.isChecked = enabled; applying = false
    }
    private fun save(enabled: Boolean) {
        saving = true; toggle.isEnabled = false
        activity.lifecycleScope.launch {
            val result = withContext(Dispatchers.IO) { runCatching {
                val root = "${activity.filesDir}/receiver"
                val state = NativeBridge.request(JSONObject().put("op", "receiver_settings").put("root", root)) as JSONObject
                val settings = state.getJSONObject("settings").put("receiver_relay", enabled)
                NativeBridge.request(JSONObject().put("op", "receiver_settings").put("root", root).put("settings", settings))
            } }
            saving = false; toggle.isEnabled = true
            render(if (result.isSuccess) enabled else !enabled)
            Toast.makeText(activity, if (result.isSuccess) (if (enabled) R.string.relay_enabled else R.string.relay_disabled) else R.string.settings_failed, Toast.LENGTH_LONG).show()
        }
    }
}
