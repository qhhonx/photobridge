package app.photobridge

import android.content.Intent
import android.os.Bundle
import androidx.appcompat.app.AppCompatActivity
import androidx.appcompat.app.AppCompatDelegate
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import org.json.JSONObject

internal enum class ReceiverHelpTopic(val key: String, val title: Int, val description: Int, val sections: List<Pair<Int, Int>>) {
    CONNECTION("connection", R.string.help_connection_title, R.string.help_connection_summary, listOf(
        R.string.help_pair_title to R.string.help_pair_body,
        R.string.help_network_title to R.string.help_network_body,
        R.string.help_address_title to R.string.help_address_body)),
    BACKGROUND("background", R.string.help_background_title, R.string.help_background_summary, listOf(
        R.string.help_keep_receiving_title to R.string.help_keep_receiving_body,
        R.string.help_restart_title to R.string.help_restart_body,
        R.string.help_sender_background_title to R.string.help_sender_background_body)),
    RESULTS("results", R.string.help_results_title, R.string.help_results_summary, listOf(
        R.string.help_receipt_title to R.string.help_receipt_body,
        R.string.help_gallery_title to R.string.help_gallery_body,
        R.string.help_cloud_title to R.string.help_cloud_body)),
    STORAGE("storage", R.string.help_storage_title, R.string.help_storage_summary, listOf(
        R.string.help_space_limits_title to R.string.help_space_limits_body,
        R.string.help_keep_originals_title to R.string.help_keep_originals_body,
        R.string.help_relay_title to R.string.help_relay_body)),
    RECOVERY("recovery", R.string.help_recovery_title, R.string.help_recovery_summary, listOf(
        R.string.help_retry_title to R.string.help_retry_body,
        R.string.help_report_title to R.string.help_report_body,
        R.string.help_update_title to R.string.help_update_body)),
}

class ReceiverHelpActivity : AppCompatActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        delegate.localNightMode = getSharedPreferences("appearance", MODE_PRIVATE).getInt("mode", AppCompatDelegate.MODE_NIGHT_FOLLOW_SYSTEM)
        super.onCreate(savedInstanceState)
        val topic = ReceiverHelpTopic.entries.firstOrNull { it.key == intent.getStringExtra("topic") }
        nativeDetailPage(topic?.title ?: R.string.receiver_help_title) { panel ->
            if (topic == null) {
                label(panel, getString(R.string.receiver_help_intro), 16, secondaryColor())
                ReceiverHelpTopic.entries.forEach { entry ->
                    card(panel) { body ->
                        label(body, getString(entry.description), 15, secondaryColor())
                        action(body, entry.title) {
                            startActivity(Intent(this, ReceiverHelpActivity::class.java).putExtra("topic", entry.key))
                        }
                    }
                }
            } else {
                label(panel, getString(topic.description), 16, secondaryColor())
                if (topic == ReceiverHelpTopic.CONNECTION) {
                    card(panel) { body ->
                        val address = label(body, getString(R.string.help_address_loading), 15)
                        address.setTextIsSelectable(true)
                        lifecycleScope.launch {
                            val result = withContext(Dispatchers.IO) {
                                runCatching { NativeBridge.request(JSONObject().put("op", "receiver_connection_info").put("root", "$filesDir/receiver")) as JSONObject }
                            }
                            address.text = result.fold(onSuccess = {
                                if (it.isNull("endpoint")) getString(R.string.help_address_missing)
                                else getString(R.string.help_address_saved, it.getString("endpoint"))
                            }, onFailure = { getString(R.string.help_address_unavailable) })
                        }
                    }
                }
                topic.sections.forEach { (title, text) ->
                    section(panel, title)
                    card(panel) { body -> label(body, getString(text), 16).setTextIsSelectable(true) }
                }
            }
        }
    }
}
