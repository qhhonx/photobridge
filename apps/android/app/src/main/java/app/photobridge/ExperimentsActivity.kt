package app.photobridge

import android.content.Intent
import android.os.Bundle
import android.provider.Settings
import android.view.View
import android.widget.Button
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.appcompat.app.AppCompatDelegate
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import com.google.android.material.dialog.MaterialAlertDialogBuilder
import kotlinx.coroutines.launch

class ExperimentsActivity : AppCompatActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        delegate.localNightMode = getSharedPreferences("appearance", MODE_PRIVATE).getInt("mode", AppCompatDelegate.MODE_NIGHT_FOLLOW_SYSTEM)
        super.onCreate(savedInstanceState)
        nativeDetailPage(R.string.experiments_title) { panel ->
            label(panel, getString(R.string.experiments_description), 16, secondaryColor())
            card(panel) { body ->
                label(body, getString(R.string.photos_probe_title), 20)
                label(body, getString(R.string.photos_probe_summary), 15, secondaryColor())
                action(body, R.string.photos_probe_open) { startActivity(Intent(this, PhotosProbeActivity::class.java)) }
            }
        }
    }
}

class PhotosProbeActivity : AppCompatActivity() {
    private val diagnostics get() = intent.getBooleanExtra("diagnostics", false)
    private lateinit var status: TextView
    private lateinit var evidence: TextView
    private lateinit var hint: TextView
    private lateinit var start: Button
    private lateinit var stop: Button
    private lateinit var permission: Button
    private lateinit var cleanupStatus: TextView
    private lateinit var bind: Button
    private lateinit var clean: Button
    private lateinit var reviewed: Button
    private lateinit var automatic: com.google.android.material.switchmaterial.SwitchMaterial
    private val cleanup get() = PhotosCleanup(this)
    private val prefs get() = getSharedPreferences("photos_probe", MODE_PRIVATE)
    private val export = registerForActivityResult(androidx.activity.result.contract.ActivityResultContracts.CreateDocument("text/plain")) { uri ->
        if (uri != null) lifecycleScope.launch(kotlinx.coroutines.Dispatchers.IO) {
            val snapshot = PhotosProbeService.state.value
            val report = "PhotoBridge Google Photos diagnostics\n" +
                "version=${photosVersion()}\nobservation=${snapshot.observation}\n" +
                "seen=${snapshot.seen.sorted().joinToString(",")}\n" +
                "cleanup_enabled=${cleanup.enabled}\ncleanup_pending=${cleanup.pending}\ncleanup_result=${cleanup.reason}\naccount_bound=${cleanup.account.isNotEmpty()}\ncloud_backup_verified=false\n"
            val ok = runCatching { checkNotNull(contentResolver.openOutputStream(uri, "wt")).use { it.write(report.toByteArray()) } }.isSuccess
            kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.Main) {
                android.widget.Toast.makeText(this@PhotosProbeActivity, if (ok) R.string.logs_exported else R.string.settings_failed, android.widget.Toast.LENGTH_LONG).show()
            }
        }
    }
    override fun onCreate(savedInstanceState: Bundle?) {
        delegate.localNightMode = getSharedPreferences("appearance", MODE_PRIVATE).getInt("mode", AppCompatDelegate.MODE_NIGHT_FOLLOW_SYSTEM)
        super.onCreate(savedInstanceState)
        nativeDetailPage(if (diagnostics) R.string.photos_diagnostics_title else R.string.photos_probe_title) { panel ->
            if (diagnostics) {
                label(panel, getString(R.string.photos_diagnostics_description), 16, secondaryColor())
                card(panel) { body ->
                    status = label(body, "", 18)
                    hint = label(body, "", 15, secondaryColor())
                    evidence = label(body, "", 15, secondaryColor())
                    label(body, getString(R.string.photos_probe_version, photosVersion()), 14, secondaryColor())
                }
                card(panel) { body ->
                    label(body, getString(R.string.photos_probe_steps), 15)
                    permission = action(body, R.string.photos_probe_permission) {
                        consent { runCatching { startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)) }.onFailure { showUnavailable() } }
                    }
                    start = action(body, R.string.photos_probe_start, primary = true) { consent { begin() } }
                    stop = action(body, R.string.photos_probe_stop) { PhotosProbeService.stop() }
                }
                card(panel) { body ->
                    label(body, getString(R.string.photos_probe_limits), 14, secondaryColor())
                    action(body, R.string.photos_probe_export) { export.launch("PhotoBridge-photos-check.txt") }
                }
            } else {
                card(panel) { body ->
                    label(body, getString(R.string.cleanup_title), 20)
                    label(body, getString(R.string.cleanup_description), 15, secondaryColor())
                    cleanupStatus = label(body, "", 15)
                    permission = action(body, R.string.photos_probe_permission) {
                        cleanupConsent { runCatching { startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS)) }.onFailure { showUnavailable() } }
                    }
                    bind = action(body, R.string.cleanup_bind) { cleanupConsent { PhotosProbeService.clean(bind = true) } }
                    automatic = com.google.android.material.switchmaterial.SwitchMaterial(this).apply {
                        setText(R.string.cleanup_automatic); isChecked = cleanup.enabled
                        setOnCheckedChangeListener { _, checked ->
                            if (checked == cleanup.enabled) return@setOnCheckedChangeListener
                            if (checked) { isChecked = false; cleanupConsent { cleanup.enabled = true; isChecked = true } }
                            else { cleanup.enabled = false; PhotosProbeService.stop() }
                        }
                    }
                    body.addView(automatic)
                    clean = action(body, R.string.cleanup_now, primary = true) { cleanupConsent { PhotosProbeService.clean() } }
                    stop = action(body, R.string.cleanup_stop) { PhotosProbeService.stop() }
                    reviewed = action(body, R.string.cleanup_reviewed) {
                        MaterialAlertDialogBuilder(this).setTitle(R.string.cleanup_reviewed).setMessage(R.string.cleanup_reviewed_help)
                            .setNegativeButton(R.string.receiver_close, null).setPositiveButton(R.string.cleanup_reviewed) { _, _ ->
                                cleanup.reviewed(); render(PhotosProbeService.state.value)
                            }.show()
                    }
                    val threshold = label(body, getString(R.string.history_loading), 14, secondaryColor())
                    lifecycleScope.launch {
                        val reserve = kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) { runCatching {
                            val value = NativeBridge.request(org.json.JSONObject().put("op", "receiver_settings").put("root", "$filesDir/receiver")) as org.json.JSONObject
                            value.getJSONObject("settings").getLong("min_free_bytes")
                        }.getOrNull() }
                        threshold.text = if (reserve == null) getString(R.string.cleanup_threshold_unavailable)
                            else getString(R.string.cleanup_threshold, android.text.format.Formatter.formatFileSize(this@PhotosProbeActivity, cleanup.trigger(reserve)))
                    }
                }
                card(panel) { body ->
                    label(body, getString(R.string.photos_diagnostics_description), 14, secondaryColor())
                    action(body, R.string.photos_diagnostics_title) {
                        startActivity(Intent(this, PhotosProbeActivity::class.java).putExtra("diagnostics", true))
                    }
                }
            }
        }
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                PhotosProbeService.state.collect { render(it) }
            }
        }
    }
    private fun consent(action: () -> Unit) {
        if (prefs.getBoolean("disclosure_v1", false)) { action(); return }
        MaterialAlertDialogBuilder(this).setTitle(R.string.photos_probe_disclosure_title)
            .setMessage(R.string.photos_probe_disclosure)
            .setNegativeButton(R.string.receiver_close, null)
            .setPositiveButton(R.string.photos_probe_consent) { _, _ -> prefs.edit().putBoolean("disclosure_v1", true).apply(); action() }.show()
    }
    private fun cleanupConsent(action: () -> Unit) {
        if (prefs.getBoolean("cleanup_disclosure_v2", false)) { action(); return }
        MaterialAlertDialogBuilder(this).setTitle(R.string.cleanup_title).setMessage(R.string.cleanup_disclosure)
            .setNegativeButton(R.string.receiver_close, null)
            .setPositiveButton(R.string.photos_probe_consent) { _, _ -> prefs.edit().putBoolean("cleanup_disclosure_v2", true).apply(); action() }.show()
    }
    private fun begin() {
        val launch = packageManager.getLaunchIntentForPackage(PhotosProbeService.PHOTOS_PACKAGE)
        if (launch == null) { showUnavailable(); return }
        if (!PhotosProbeService.begin()) { render(PhotosProbeService.state.value); return }
        runCatching { startActivity(launch) }.onFailure { PhotosProbeService.stop(); showUnavailable() }
    }
    private fun showUnavailable() {
        android.widget.Toast.makeText(this, R.string.photos_probe_unavailable, android.widget.Toast.LENGTH_LONG).show()
    }
    @Suppress("DEPRECATION") private fun photosVersion(): String = runCatching {
        packageManager.getPackageInfo(PhotosProbeService.PHOTOS_PACKAGE, 0).versionName ?: "unknown"
    }.getOrDefault(getString(R.string.photos_probe_missing))
    private fun render(state: PhotosProbeState) {
        if (!diagnostics) {
            val result = if (state.mode == "probe") cleanup.reason else state.observation
            val resource = resources.getIdentifier("cleanup_result_$result", "string", packageName)
            cleanupStatus.text = getString(when {
                !state.connected -> R.string.photos_probe_disabled
                resource != 0 -> resource
                cleanup.account.isNotEmpty() -> R.string.cleanup_result_bound
                else -> R.string.cleanup_result_ready
            })
            permission.visibility = if (!state.connected) View.VISIBLE else View.GONE
            bind.isEnabled = state.connected && !state.running && !cleanup.pending
            clean.setText(if (cleanup.pending) R.string.cleanup_check_pending else R.string.cleanup_now)
            clean.isEnabled = state.connected && !state.running && cleanup.account.isNotEmpty()
            automatic.isEnabled = state.connected && cleanup.account.isNotEmpty() && !state.running
            reviewed.visibility = if (cleanup.pending && !state.running) View.VISIBLE else View.GONE
            stop.visibility = if (state.running) View.VISIBLE else View.GONE
            return
        }
        if (state.mode != "probe") {
            status.text = getString(if (state.connected) R.string.photos_probe_ready else R.string.photos_probe_disabled)
            hint.visibility = View.GONE; evidence.visibility = View.GONE
            start.isEnabled = state.connected && !state.running
            permission.visibility = if (state.running) View.GONE else View.VISIBLE
            stop.visibility = if (state.running) View.VISIBLE else View.GONE
            return
        }
        evidence.visibility = View.VISIBLE
        val message = when {
            !state.connected -> R.string.photos_probe_disabled
            state.running -> R.string.photos_probe_running
            state.observation == "confirmation" -> R.string.photos_probe_confirmation
            state.observation == "nothing_to_free" -> R.string.photos_probe_nothing
            state.observation == "completed" -> R.string.photos_probe_completed
            state.observation == "locked" -> R.string.photos_probe_locked
            state.observation == "expired" -> R.string.photos_probe_expired
            state.observation in setOf("unavailable", "interrupted", "disconnected") -> R.string.photos_probe_unavailable
            state.observation == "stopped" -> R.string.photos_probe_stopped
            else -> R.string.photos_probe_ready
        }
        status.text = if (state.running) getString(message, state.secondsLeft) else getString(message)
        hint.text = getString(when (state.observation) {
            "other_app", "waiting" -> R.string.photos_probe_waiting_page
            "unknown", "incomplete" -> R.string.photos_probe_unknown_page
            else -> R.string.photos_probe_follow_steps
        })
        hint.visibility = if (state.running) View.VISIBLE else View.GONE
        val stages = listOf("account_entry" to R.string.photos_probe_seen_account, "cleanup_entry" to R.string.photos_probe_seen_entry,
            "confirmation" to R.string.photos_probe_seen_confirmation, "nothing_to_free" to R.string.photos_probe_seen_empty,
            "releasing" to R.string.photos_probe_seen_progress, "completed" to R.string.photos_probe_seen_completed)
        evidence.text = if (state.seen.isEmpty()) getString(R.string.photos_probe_no_evidence)
            else stages.filter { it.first in state.seen }.joinToString("\n") { getString(it.second) }
        start.isEnabled = state.connected && !state.running
        permission.visibility = if (state.running) View.GONE else View.VISIBLE
        stop.visibility = if (state.running) View.VISIBLE else View.GONE
    }
}
