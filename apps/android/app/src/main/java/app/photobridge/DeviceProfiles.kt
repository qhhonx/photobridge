package app.photobridge

import android.content.Context
import android.text.format.DateUtils
import android.view.View
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import androidx.lifecycle.lifecycleScope
import com.google.android.material.dialog.MaterialAlertDialogBuilder
import com.google.android.material.textfield.TextInputEditText
import com.google.android.material.textfield.TextInputLayout
import kotlinx.coroutines.*
import org.json.JSONObject
import java.util.Locale

internal object DeviceProfiles {
    fun read(context: Context, name: String? = null): JSONObject {
        val request = JSONObject().put("op", "device_status")
            .put("root", "${context.filesDir}/receiver/store").put("language", Locale.getDefault().language)
        if (name != null) request.put("name", name)
        return NativeBridge.request(request) as JSONObject
    }
}

internal class DevicePanels(private val activity: MainActivity) {
    private val nameLabels = mutableListOf<TextView>()
    private var peersLabel: TextView? = null
    private var peersButton: Button? = null
    private var state: JSONObject? = null
    private var work: Job? = null
    val name: String? get() = state?.optJSONObject("device")?.optString("name")

    fun receiver(parent: LinearLayout) = activity.card(parent) { body ->
        nameLabels += activity.label(body, activity.getString(R.string.device_loading), 20)
        activity.label(body, activity.getString(R.string.device_known_senders), 14, activity.secondaryColor())
        peersLabel = activity.label(body, activity.getString(R.string.device_no_senders), 15)
        peersButton = activity.action(body, R.string.device_all_senders) { showPeers() }.apply {
            visibility = View.GONE
        }
    }
    fun settings(parent: LinearLayout) = activity.card(parent) { body ->
        activity.label(body, activity.getString(R.string.device_this_device), 14, activity.secondaryColor())
        nameLabels += activity.label(body, activity.getString(R.string.device_loading), 20)
        activity.action(body, R.string.device_rename) { rename() }
        activity.label(body, activity.getString(R.string.device_name_note), 14, activity.secondaryColor())
    }
    fun refresh() {
        if (work?.isActive == true) return
        work = activity.lifecycleScope.launch {
            runCatching { withContext(Dispatchers.IO) { DeviceProfiles.read(activity) } }
                .onSuccess { render(it) }
                .onFailure { if (state == null) nameLabels.forEach { it.setText(R.string.device_load_failed) } }
        }
    }
    private fun render(next: JSONObject) {
        state = next
        val name = next.getJSONObject("device").getString("name")
        nameLabels.forEach { if (it.text.toString() != name) it.text = name }
        val peers = next.getJSONArray("peers")
        peersButton?.visibility = if (peers.length() == 0) View.GONE else View.VISIBLE
        val text = if (peers.length() == 0) activity.getString(R.string.device_no_senders)
            else (0 until minOf(3, peers.length())).joinToString("\n") {
                peerSummary(peers.getJSONObject(it))
            }
        if (peersLabel?.text?.toString() != text) peersLabel?.text = text
    }
    private fun peerSummary(peer: JSONObject): String {
        val profile = peer.getJSONObject("profile")
        val kind = peer.optString("device_type").takeIf { it.isNotBlank() && it != "null" }
        val ip = peer.optString("ip").takeIf { it.isNotBlank() && it != "null" }
        return listOfNotNull(profile.getString("name"), kind, ip?.let { activity.getString(R.string.device_last_ip, it) },
            if (!peer.optBoolean("enabled", true)) activity.getString(R.string.device_disabled) else null).joinToString(" · ")
    }
    private fun showPeers() {
        val peers = state?.optJSONArray("peers") ?: return
        val rows = (0 until peers.length()).map { index ->
            val peer = peers.getJSONObject(index)
            peerSummary(peer) + "\n" + DateUtils.getRelativeTimeSpanString(peer.getLong("last_seen") * 1000)
        }.toTypedArray()
        MaterialAlertDialogBuilder(activity).setTitle(R.string.device_known_senders).setItems(rows) { _, which ->
            val peer = peers.getJSONObject(which)
            val enabled = peer.optBoolean("enabled", true)
            MaterialAlertDialogBuilder(activity).setTitle(peer.getJSONObject("profile").getString("name"))
                .setMessage(R.string.device_disable_help)
                .setNegativeButton(R.string.device_cancel, null)
                .setPositiveButton(if (enabled) R.string.device_disable else R.string.device_enable) { _, _ ->
                    activity.lifecycleScope.launch {
                        runCatching { withContext(Dispatchers.IO) {
                            NativeBridge.request(JSONObject().put("op", "set_sender_enabled")
                                .put("root", "${activity.filesDir}/receiver/store")
                                .put("id", peer.getJSONObject("profile").getString("id")).put("enabled", !enabled))
                        } }.onSuccess { refresh() }.onFailure {
                            android.widget.Toast.makeText(activity,R.string.device_save_failed,android.widget.Toast.LENGTH_LONG).show()
                        }
                    }
                }.show()
        }.setPositiveButton(R.string.receiver_close, null).show()
    }
    private fun rename() {
        val current = name ?: run { refresh(); return }
        val panel = TextInputLayout(activity).apply {
            hint = activity.getString(R.string.device_name)
            setPadding(activity.dp(24), activity.dp(12), activity.dp(24), 0)
        }
        val input = TextInputEditText(panel.context).apply { setText(current); isSingleLine = true; selectAll() }
        panel.addView(input)
        val dialog = MaterialAlertDialogBuilder(activity).setTitle(R.string.device_rename).setView(panel)
            .setNegativeButton(R.string.device_cancel, null).setPositiveButton(R.string.device_save, null).create()
        dialog.setOnShowListener {
            dialog.getButton(android.app.AlertDialog.BUTTON_POSITIVE).setOnClickListener {
                val value = input.text.toString()
                panel.error = null
                input.isEnabled = false
                val save = dialog.getButton(android.app.AlertDialog.BUTTON_POSITIVE)
                val cancel = dialog.getButton(android.app.AlertDialog.BUTTON_NEGATIVE)
                save.isEnabled = false; cancel.isEnabled = false; dialog.setCancelable(false)
                activity.lifecycleScope.launch {
                    val result = withContext(Dispatchers.IO) { runCatching { DeviceProfiles.read(activity, value) } }
                    input.isEnabled = true; save.isEnabled = true; cancel.isEnabled = true; dialog.setCancelable(true)
                    result.onSuccess { render(it); dialog.dismiss() }.onFailure {
                        panel.error = activity.getString(if (it.message == "invalid_input") R.string.device_name_invalid else R.string.device_save_failed)
                    }
                }
            }
        }
        dialog.show()
    }
}
