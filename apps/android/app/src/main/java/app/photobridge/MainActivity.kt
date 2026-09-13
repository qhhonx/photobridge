package app.photobridge

import android.Manifest
import android.app.AlertDialog
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.Color
import android.os.Build
import android.os.Bundle
import android.view.View
import android.view.WindowManager
import android.widget.*
import androidx.activity.OnBackPressedCallback
import androidx.activity.viewModels
import androidx.appcompat.app.AppCompatActivity
import androidx.appcompat.app.AppCompatDelegate
import com.google.android.material.dialog.MaterialAlertDialogBuilder
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import com.google.android.material.appbar.MaterialToolbar
import com.google.android.material.bottomnavigation.BottomNavigationView
import com.google.android.material.navigation.NavigationBarView
import com.google.android.material.materialswitch.MaterialSwitch
import com.google.zxing.BarcodeFormat
import com.google.zxing.MultiFormatWriter
import com.journeyapps.barcodescanner.ScanContract
import com.journeyapps.barcodescanner.ScanOptions
import kotlinx.coroutines.*
import org.json.JSONObject

class MainActivity : AppCompatActivity() {
    private val history: HistoryModel by viewModels()
    private val receiverRoot get() = "$filesDir/receiver"
    private var selectedPage = 1
    private lateinit var navigation: BottomNavigationView
    private lateinit var status: TextView
    private lateinit var detail: TextView
    private lateinit var recoveryText: TextView
    private lateinit var recoveryAction: Button
    private lateinit var pair: Button
    private lateinit var scanDesktop: Button
    private lateinit var start: Button
    private lateinit var stop: Button
    private lateinit var storagePage: StoragePage
    private lateinit var processing: TextView
    private lateinit var processingProgress: ProgressBar
    private lateinit var historyPage: HistoryPage
    private lateinit var devicePanels: DevicePanels
    private var localTotals: String? = null
    private val scanner = registerForActivityResult(ScanContract()) { result ->
        result.contents?.let(::submitDesktopPairing)
    }
    private val originalArchive = registerForActivityResult(androidx.activity.result.contract.ActivityResultContracts.CreateDocument("application/zip")) { uri ->
        if (uri != null) exportOriginals(uri)
    }
    private val exportLogs = registerForActivityResult(androidx.activity.result.contract.ActivityResultContracts.CreateDocument("application/json")) { uri ->
        if (uri != null) lifecycleScope.launch {
            val result = withContext(Dispatchers.IO) { runCatching {
                val data = NativeBridge.request(JSONObject().put("op", "receiver_logs").put("root", receiverRoot)).toString().toByteArray(Charsets.UTF_8)
                checkNotNull(contentResolver.openOutputStream(uri, "wt")).use { it.write(data) }
            } }
            Toast.makeText(this@MainActivity, if (result.isSuccess) R.string.logs_exported else R.string.settings_failed, Toast.LENGTH_LONG).show()
        }
    }
    override fun onCreate(savedInstanceState: Bundle?) {
        delegate.localNightMode = getSharedPreferences("appearance", MODE_PRIVATE).getInt("mode", AppCompatDelegate.MODE_NIGHT_FOLLOW_SYSTEM)
        super.onCreate(savedInstanceState)
        devicePanels = DevicePanels(this)
        selectedPage = savedInstanceState?.getInt("page", 1) ?: 1
        val root = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        val toolbar = MaterialToolbar(this).apply { title = getString(R.string.nav_receive) }
        root.addView(toolbar, LinearLayout.LayoutParams(-1, dp(64)))
        val host = FrameLayout(this)
        root.addView(host, LinearLayout.LayoutParams(-1, 0, 1f))
        historyPage = HistoryPage(this, history, receiverRoot)
        storagePage = StoragePage(this, receiverRoot, export = {
            MaterialAlertDialogBuilder(this).setTitle(R.string.originals_export).setMessage(R.string.originals_export_note)
                .setNegativeButton(R.string.receiver_close, null).setPositiveButton(R.string.originals_choose) { _, _ ->
                    originalArchive.launch("PhotoBridge-originals-${System.currentTimeMillis()}.zip")
                }.show()
        }, totals = { counts ->
            localTotals = getString(R.string.receiver_totals, counts.getInt("received"), counts.getInt("total"), counts.getInt("published"))
            if (ReceiverState.snapshot.value.phase == "idle") renderReceiver(ReceiverState.snapshot.value)
        })
        val pages = listOf(receiverPage(), historyPage, storagePage.view, settingsPage())
        pages.forEach { host.addView(it, FrameLayout.LayoutParams(-1, -1)) }
        val titles = listOf(R.string.nav_receive, R.string.nav_transfers, R.string.nav_storage, R.string.nav_settings)
        val icons = listOf(R.drawable.ic_receiver, R.drawable.ic_transfers, R.drawable.ic_storage, R.drawable.ic_settings)
        navigation = BottomNavigationView(this).apply {
            id = R.id.main_navigation
            labelVisibilityMode = NavigationBarView.LABEL_VISIBILITY_LABELED
            isItemHorizontalTranslationEnabled = false
            titles.forEachIndexed { index, title -> menu.add(0, index + 1, index, title).setIcon(icons[index]) }
            setOnItemSelectedListener { item ->
                selectedPage = item.itemId
                toolbar.setTitle(titles[selectedPage - 1])
                pages.forEachIndexed { index, page -> page.visibility = if (index + 1 == selectedPage) View.VISIBLE else View.GONE }
                if (selectedPage == 3) storagePage.refresh(force = true)
                true
            }
        }
        root.addView(navigation, LinearLayout.LayoutParams(-1, -2))
        setContentView(root)
        navigation.selectedItemId = selectedPage
        storagePage.refresh(force = true, includeDetails = selectedPage == 3)
        onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
            override fun handleOnBackPressed() {
                if (selectedPage != 1) navigation.selectedItemId = 1 else finish()
            }
        })
        lifecycleScope.launch {
            if (history.state.value.items.isEmpty()) history.select(receiverRoot)
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                launch { history.state.collect { historyPage.render(it) } }
                launch { ReceiverState.snapshot.collect(::renderReceiver) }
                launch {
                    while (isActive) {
                        devicePanels.refresh()
                        if (selectedPage == 2) history.refreshVisible(receiverRoot, historyPage.firstVisible())
                        if (selectedPage == 3) storagePage.refresh()
                        delay(2_000)
                    }
                }
            }
        }
    }
    override fun onSaveInstanceState(outState: Bundle) {
        outState.putInt("page", selectedPage)
        super.onSaveInstanceState(outState)
    }
    override fun onStart() {
        super.onStart()
        if (ReceiverPreferences.enabled(this)) startReceiverService()
        val updates = AppUpdates.preferences(this)
        val now = System.currentTimeMillis()
        if (updates.getBoolean("automatic", true) && now - updates.getLong("checked", 0) > 86_400_000L) {
            updates.edit().putLong("checked", now).apply()
            lifecycleScope.launch {
                val next = withContext(Dispatchers.IO) { runCatching { AppUpdates.latest(this@MainActivity) }.getOrNull() }
                if (next != null && !isFinishing && lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)) {
                    MaterialAlertDialogBuilder(this@MainActivity).setTitle(R.string.updates_title)
                        .setMessage(getString(R.string.updates_available, next.version))
                        .setPositiveButton(R.string.updates_title) { _, _ -> startActivity(Intent(this@MainActivity, UpdatesActivity::class.java)) }
                        .setNegativeButton(R.string.receiver_close, null).show()
                }
            }
        }
    }
    private fun receiverPage(): View = scrollPage { panel ->
        devicePanels.receiver(panel)
        card(panel) { body ->
            status = label(body, getString(R.string.receiver_idle), 22)
            detail = label(body, "", 15, secondaryColor())
            recoveryText = label(body, "", 15, secondaryColor()).apply { visibility = View.GONE }
            recoveryAction = action(body, R.string.receiver_help_title) {
                receiverProblem(ReceiverState.snapshot.value)?.let { problem ->
                    when (problem.recovery) {
                        ReceiverRecovery.NETWORK -> runCatching { startActivity(Intent(android.provider.Settings.ACTION_WIFI_SETTINGS)) }
                            .onFailure { openHelp(ReceiverHelpTopic.CONNECTION) }
                        ReceiverRecovery.STORAGE -> navigation.selectedItemId = 3
                        ReceiverRecovery.RESUME -> beginReceiving()
                        ReceiverRecovery.CONNECTION_HELP -> openHelp(ReceiverHelpTopic.CONNECTION)
                        ReceiverRecovery.DIAGNOSTICS_HELP -> openHelp(ReceiverHelpTopic.RECOVERY)
                    }
                }
            }.apply { visibility = View.GONE }
            processing = label(body, "", 14, accentColor())
            processingProgress = ProgressBar(this, null, android.R.attr.progressBarStyleHorizontal).apply { isIndeterminate = true; visibility = View.GONE }
            body.addView(processingProgress, LinearLayout.LayoutParams(-1, dp(4)))
            start = action(body, R.string.receiver_start, primary = true) {
                beginReceiving()
            }
            stop = action(body, R.string.receiver_stop) {
                ReceiverPreferences.setEnabled(this, false)
                stopService(Intent(this, ReceiverService::class.java))
            }
        }
        section(panel, R.string.receiver_pair_section)
        card(panel) { body ->
            label(body, getString(R.string.receiver_intro), 15, secondaryColor())
            pair = action(body, R.string.receiver_pair, action = ::showPairing)
            scanDesktop = action(body, R.string.receiver_scan_desktop) {
                scanner.launch(ScanOptions().setDesiredBarcodeFormats(ScanOptions.QR_CODE)
                    .setPrompt(getString(R.string.receiver_scan_prompt)).setBeepEnabled(false)
                    .setBarcodeImageEnabled(false).setOrientationLocked(false))
            }
        }
        label(panel, getString(R.string.receiver_cloud_note), 13, secondaryColor())
    }
    private fun settingsPage(): View = scrollPage { panel ->
        devicePanels.settings(panel)
        card(panel) { body ->
            action(body, R.string.settings_appearance) {
                val modes = listOf(AppCompatDelegate.MODE_NIGHT_FOLLOW_SYSTEM, AppCompatDelegate.MODE_NIGHT_NO, AppCompatDelegate.MODE_NIGHT_YES)
                val labels = arrayOf(getString(R.string.appearance_system), getString(R.string.appearance_light), getString(R.string.appearance_dark))
                MaterialAlertDialogBuilder(this).setTitle(R.string.settings_appearance)
                    .setSingleChoiceItems(labels, modes.indexOf(delegate.localNightMode).coerceAtLeast(0)) { dialog, index ->
                        dialog.dismiss()
                        getSharedPreferences("appearance", MODE_PRIVATE).edit().putInt("mode", modes[index]).apply()
                        delegate.localNightMode = modes[index]
                    }.setNegativeButton(R.string.receiver_close, null).show()
            }
        }
        section(panel, R.string.receiver_behavior)
        card(panel) { body ->
            body.addView(MaterialSwitch(this).apply {
                setText(if (Build.VERSION.SDK_INT >= 35) R.string.receiver_restore_reminder else R.string.receiver_restore)
                isChecked = ReceiverPreferences.restore(this@MainActivity)
                setOnCheckedChangeListener { _, checked -> ReceiverPreferences.setRestore(this@MainActivity, checked) }
                minimumHeight = dp(56)
            })
            label(body, getString(if (Build.VERSION.SDK_INT >= 35) R.string.receiver_restore_note_modern else R.string.receiver_restore_note_legacy), 14, secondaryColor())
        }
        section(panel, R.string.settings_diagnostics)
        card(panel) { body ->
            action(body, R.string.logs_retention_settings) { showStorageControls(StorageSection.LOGS) {} }
            action(body, R.string.logs_export) { exportLogs.launch("PhotoBridge-diagnostics.json") }
        }
        card(panel) { body ->
            action(body, R.string.experiments_title) { startActivity(Intent(this, ExperimentsActivity::class.java)) }
        }
        card(panel) { body ->
            action(body, R.string.receiver_help_title) { openHelp() }
        }
        action(panel, R.string.updates_title) { startActivity(Intent(this, UpdatesActivity::class.java)) }
        section(panel, R.string.app_name)
        card(panel) { body ->
            label(body, getString(R.string.settings_version, packageManager.getPackageInfo(packageName, 0).versionName ?: ""), 16)
            label(body, getString(R.string.receiver_motion_note), 14, secondaryColor())
        }
    }
    private fun openHelp(topic: ReceiverHelpTopic? = null) {
        startActivity(Intent(this, ReceiverHelpActivity::class.java).apply { topic?.let { putExtra("topic", it.key) } })
    }
    private fun beginReceiving() {
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED)
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 1)
        ReceiverPreferences.setEnabled(this, true)
        startReceiverService()
    }
    private fun startReceiverService() {
        runCatching { startForegroundService(Intent(this, ReceiverService::class.java)) }.onFailure {
            ReceiverState.mutable.value = ReceiverState.snapshot.value.copy(phase = "waiting", error = "foreground_start_blocked")
        }
    }
    private fun renderReceiver(state: ReceiverSnapshot) {
        val problem = receiverProblem(state)
        recoveryText.visibility = if (problem == null) View.GONE else View.VISIBLE
        recoveryAction.visibility = recoveryText.visibility
        if (problem != null) {
            recoveryText.setText(problem.message)
            recoveryAction.setText(problem.action)
        }
        status.text = when (state.phase) {
            "ready" -> getString(R.string.receiver_ready)
            "starting" -> getString(R.string.receiver_starting)
            "waiting" -> getString(R.string.receiver_waiting)
            "error" -> getString(R.string.settings_failed)
            else -> getString(R.string.receiver_idle)
        }
        detail.text = (if (state.phase == "idle") localTotals ?: getString(R.string.history_loading) else getString(R.string.receiver_totals, state.received, state.total, state.published))
        processing.text = state.processingName?.let { getString(R.string.receiver_processing, it) } ?: ""
        processing.visibility = if (state.processingName == null) View.GONE else View.VISIBLE
        processingProgress.visibility = processing.visibility
        pair.isEnabled = ReceiverState.pairing != null
        scanDesktop.isEnabled = ReceiverState.pairing != null
        val enabled = ReceiverPreferences.enabled(this)
        start.visibility = if (enabled) View.GONE else View.VISIBLE
        stop.visibility = if (enabled) View.VISIBLE else View.GONE
    }
    internal fun openPhotos() {
        val launch = packageManager.getLaunchIntentForPackage("com.google.android.apps.photos")
        if (launch != null) {
            try { startActivity(launch); return }
            catch (_: android.content.ActivityNotFoundException) { /* Removed since lookup. */ }
        }
        Toast.makeText(this, R.string.receiver_photos_missing, Toast.LENGTH_SHORT).show()
    }
    private fun submitDesktopPairing(contents: String) {
        val pairing = ReceiverState.pairing ?: return
        lifecycleScope.launch {
            Toast.makeText(this@MainActivity, R.string.receiver_pairing_connecting, Toast.LENGTH_SHORT).show()
            val result = withContext(Dispatchers.IO) {
                runCatching {
                    require(contents.length <= 32768)
                    NativeBridge.request(JSONObject().put("op", "submit_desktop_pairing")
                        .put("invite", JSONObject(contents)).put("pairing", JSONObject(pairing)))
                }
            }
            Toast.makeText(this@MainActivity,
                if (result.isSuccess) R.string.receiver_desktop_paired else R.string.receiver_desktop_pair_failed,
                Toast.LENGTH_LONG).show()
        }
    }
    private fun showPairing() {
        val payload = ReceiverState.pairing ?: return
        runCatching {
            val matrix = MultiFormatWriter().encode(payload, BarcodeFormat.QR_CODE, 900, 900)
            val pixels = IntArray(900 * 900) { index -> if (matrix[index % 900, index / 900]) Color.BLACK else Color.WHITE }
            val bitmap = Bitmap.createBitmap(pixels, 900, 900, Bitmap.Config.ARGB_8888)
            val panel = LinearLayout(this).apply {
                orientation = LinearLayout.VERTICAL; setPadding(dp(16), dp(16), dp(16), dp(16))
                addView(ImageView(context).apply { setImageBitmap(bitmap); adjustViewBounds = true }, LinearLayout.LayoutParams(-1, dp(300)))
            }
            label(panel, getString(R.string.receiver_pair_instruction), 14, secondaryColor())
            val dialog = AlertDialog.Builder(this).setTitle(devicePanels.name ?: getString(R.string.receiver_pair)).setView(panel).setPositiveButton(R.string.receiver_close, null).create()
            dialog.setOnShowListener { dialog.window?.addFlags(WindowManager.LayoutParams.FLAG_SECURE) }
            dialog.show()
        }.onFailure { Toast.makeText(this, getString(R.string.receiver_error, "pairing_code"), Toast.LENGTH_LONG).show() }
    }
}
