package app.photobridge

import android.accessibilityservice.AccessibilityService
import android.app.KeyguardManager
import android.content.Intent
import kotlinx.coroutines.sync.withLock
import android.os.SystemClock
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.json.JSONArray
import org.json.JSONObject

internal data class PhotosProbeState(
    val connected: Boolean = false,
    val running: Boolean = false,
    val observation: String = "idle",
    val seen: Set<String> = emptySet(),
    val secondsLeft: Int = 0,
    val mode: String = "probe",
)

/** Opt-in, bounded Google Photos adapter. Rust chooses actions; Android supplies fresh UI and durable fencing. */
class PhotosProbeService : AccessibilityService() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var observationJob: Job? = null
    private var cleanupJob: Job? = null
    private val cleanup get() = PhotosCleanup(this)
    companion object {
        const val PHOTOS_PACKAGE = "com.google.android.apps.photos"
        private var instance: PhotosProbeService? = null
        private val mutableState = MutableStateFlow(PhotosProbeState())
        internal val state = mutableState.asStateFlow()
        internal fun begin(): Boolean = instance?.let { it.observe(); true } ?: false
        internal fun stop() { instance?.end("stopped") }
        internal val cleanupRunning: Boolean get() = mutableState.value.running && mutableState.value.mode == "cleanup"
        internal fun clean(bind: Boolean = false): Boolean = instance?.let {
            if (!mutableState.value.running) { it.operate(bind); true } else false
        } ?: false
        private val fields = setOf("selected_account_disc", "og_bento_card_title", "safetyTip", "free_up_button",
            "free_up_space_completed_title", "done_button", "title", "close_button", "free_up_space_progress_text")
        private val recognized = setOf("account_entry", "cleanup_entry", "confirmation", "nothing_to_free", "releasing", "completed")
    }
    override fun onServiceConnected() {
        instance = this
        mutableState.value = PhotosProbeState(connected = true)
    }
    // Events are deliberately unused. A bounded sampling loop only runs after the user's request.
    override fun onAccessibilityEvent(event: AccessibilityEvent?) = Unit
    override fun onInterrupt() { end("interrupted") }
    override fun onDestroy() {
        end("disconnected")
        scope.cancel()
        if (instance === this) instance = null
        mutableState.value = mutableState.value.copy(connected = false)
        super.onDestroy()
    }
    private fun end(reason: String) {
        observationJob?.cancel(); observationJob = null
        cleanupJob?.cancel(); cleanupJob = null
        mutableState.value = mutableState.value.copy(running = false, secondsLeft = 0, observation = reason)
    }
    private fun observe() {
        if (mutableState.value.running) return
        observationJob?.cancel()
        mutableState.value = PhotosProbeState(connected = true, running = true, secondsLeft = 120, observation = "waiting")
        observationJob = scope.launch {
            val deadline = SystemClock.elapsedRealtime() + 120_000
            while (isActive && SystemClock.elapsedRealtime() < deadline) {
                // Only the selected UI-control texts enter memory. No screenshots, account
                // descriptions, photo labels or raw tree data enter diagnostics or storage.
                val snapshot = runCatching { capture() }.getOrNull()
                if (snapshot == null) {
                    mutableState.value = mutableState.value.copy(running = false, secondsLeft = 0, observation = "unavailable")
                    return@launch
                }
                val result = withContext(Dispatchers.Default) {
                    runCatching { NativeBridge.request(JSONObject().put("op", "photos_probe").put("snapshot", snapshot)) as String }
                        .getOrDefault("unavailable")
                }
                ensureActive()
                val old = mutableState.value
                val seen = if (result in recognized) old.seen + result else old.seen
                val terminal = result in setOf("locked", "confirmation", "nothing_to_free", "completed", "unavailable")
                mutableState.value = old.copy(observation = result, seen = seen, running = !terminal,
                    secondsLeft = if (terminal) 0 else ((deadline - SystemClock.elapsedRealtime()).coerceAtLeast(0) / 1000).toInt())
                if (terminal) return@launch
                delay(800)
            }
            mutableState.value = mutableState.value.copy(running = false, secondsLeft = 0, observation = "expired")
        }
    }
    private fun operate(bind: Boolean) {
        if (!bind && cleanup.account.isEmpty()) { cleanup.report("account_unavailable"); return }
        mutableState.value = PhotosProbeState(connected = true, running = true, mode = if (bind) "bind" else "cleanup", observation = "opening")
        cleanupJob = scope.launch {
            try {
                ReceiverState.mediaOperations.withLock {
                    if (!bind) withContext(Dispatchers.IO) {
                        val receiver = NativeBridge.request(JSONObject().put("op", "receiver_status")) as JSONObject
                        if (receiver.getBoolean("running")) {
                            NativeBridge.request(JSONObject().put("op", "receiver_transfer_hold").put("held", true))
                        }
                    }
                    val launch = packageManager.getLaunchIntentForPackage(PHOTOS_PACKAGE) ?: error("unavailable")
                    startActivity(launch.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                    delay(1_200)
                    var state = if (cleanup.pending) cleanup.state else JSONObject().put("phase", "home").put("backs", 0).put("actions", 0)
                    val deadline = SystemClock.elapsedRealtime() + if (bind) 30_000 else 300_000
                    var bindBacks = 0
                    var candidateAccount = ""
                    while (isActive && SystemClock.elapsedRealtime() < deadline) {
                        val snapshot = capture(includeAccount = true)
                        val observed = withContext(Dispatchers.Default) {
                            NativeBridge.request(JSONObject().put("op", "photos_probe").put("snapshot", snapshot)) as String
                        }
                        ensureActive()
                        if (bind && candidateAccount.isEmpty()) {
                            if (observed == "account_entry" && snapshot.has("account")) {
                                candidateAccount = snapshot.getString("account")
                            } else {
                                if (observed in setOf("locked", "other_app", "incomplete", "releasing") || bindBacks >= 3) error(if (observed == "locked") "locked" else "account_unavailable")
                                bindBacks++
                                performGlobalAction(GLOBAL_ACTION_BACK)
                                delay(1_000); continue
                            }
                        }
                        val decision = withContext(Dispatchers.Default) {
                            NativeBridge.request(JSONObject().put("op", "photos_cleanup_step").put("state", state)
                                .put("snapshot", snapshot).put("expected", if (bind) candidateAccount else cleanup.account)) as JSONObject
                        }
                        ensureActive()
                        val action = decision.getString("action")
                        val reason = decision.getString("reason")
                        // No suspension between a fresh equivalent snapshot, the durable
                        // commit fence, and the actual click. Stale pages cause a retry.
                        if (action !in setOf("wait", "stop", "finish") && capture(true).toString() != snapshot.toString()) {
                            delay(800); continue
                        }
                        state = decision.getJSONObject("state")
                        if (bind && action in setOf("confirm", "close")) {
                            // Binding also verifies the whole navigation path. It stops
                            // before release and never turns automatic cleanup on.
                            cleanup.account = candidateAccount
                            cleanup.report("bound"); updateOperation("bound", false)
                            performGlobalAction(GLOBAL_ACTION_BACK)
                            return@withLock
                        }
                        if (bind && action in setOf("wait", "done", "finish")) error("pending")
                        if (!bind) cleanup.save(state, reason)
                        updateOperation(reason, action !in setOf("stop", "finish"))
                        when (action) {
                            "stop" -> return@withLock
                            "finish" -> { cleanup.cooldown(15 * 60_000); return@withLock }
                            "back" -> if (!performGlobalAction(GLOBAL_ACTION_BACK)) error("unrecognized")
                            "account" -> if (!click("selected_account_disc")) error("unrecognized")
                            "entry" -> if (!click("og_bento_card_title", setOf("Free up space on this device", "释放此设备的空间"))) error("unrecognized")
                            "confirm" -> if (!click("free_up_button")) error("pending")
                            "done" -> if (!click("done_button")) error("return_home_failed")
                            "close" -> if (!click("close_button")) error("return_home_failed")
                        }
                        delay(if (action == "wait") 1_500 else 1_200)
                    }
                    cleanup.report(if (cleanup.pending) "pending" else "expired")
                }
            } catch (cancelled: CancellationException) { cleanup.report(if (cleanup.pending) "pending" else "stopped"); throw cancelled }
            catch (error: Exception) { cleanup.report(safeError(error)) }
            finally {
                mutableState.value = mutableState.value.copy(running = false, observation = cleanup.reason)
                withContext(NonCancellable + Dispatchers.IO) { runCatching {
                    NativeBridge.request(JSONObject().put("op", "receiver_transfer_hold").put("held", cleanup.held))
                } }
            }
        }
    }
    private fun updateOperation(reason: String, running: Boolean) {
        if (mutableState.value.observation != reason) scope.launch(Dispatchers.IO) {
            runCatching { NativeBridge.request(JSONObject().put("op", "record_event").put("receiver", true).put("code", "photos_cleanup_" + reason)) }
        }
        mutableState.value = mutableState.value.copy(observation = reason, running = running)
    }
    @Suppress("DEPRECATION")
    private fun click(suffix: String, labels: Set<String>? = null): Boolean {
        if (getSystemService(KeyguardManager::class.java).isKeyguardLocked) return false
        val root = rootInActiveWindow ?: return false
        try {
            if (root.packageName?.toString() != PHOTOS_PACKAGE) return false
            val matches = root.findAccessibilityNodeInfosByViewId("$PHOTOS_PACKAGE:id/$suffix")
            try {
                val visible = matches.filter { it.isVisibleToUser && it.isEnabled && (labels == null || it.text?.toString()?.trim() in labels) }
                if (visible.size != 1) return false
                var candidate = AccessibilityNodeInfo.obtain(visible.single())
                try {
                    repeat(5) {
                        if (!candidate.refresh() || candidate.packageName?.toString() != PHOTOS_PACKAGE || !candidate.isVisibleToUser || !candidate.isEnabled) return false
                        if (candidate.isClickable) return candidate.performAction(AccessibilityNodeInfo.ACTION_CLICK)
                        val parent = candidate.parent ?: return false
                        candidate.recycle(); candidate = parent
                    }
                    return false
                } finally { candidate.recycle() }
            } finally { matches.forEach { it.recycle() } }
        } finally { root.recycle() }
    }
    @Suppress("DEPRECATION")
    private fun capture(includeAccount: Boolean = false): JSONObject {
        val result = JSONObject().put("package", "").put("locked", getSystemService(KeyguardManager::class.java).isKeyguardLocked)
            .put("complete", false).put("nodes", JSONArray())
        if (result.getBoolean("locked")) return result
        val root = rootInActiveWindow ?: return result
        val pending = java.util.ArrayDeque<AccessibilityNodeInfo>()
        pending.add(root)
        val nodes = result.getJSONArray("nodes")
        var visited = 0
        var complete = true
        try {
            val rootPackage = root.packageName?.toString() ?: ""
            result.put("package", rootPackage)
            if (rootPackage != PHOTOS_PACKAGE) return result
            while (pending.isNotEmpty() && visited < 512) {
                val node = pending.removeFirst()
                try {
                    visited++
                    if (node.isVisibleToUser && !node.isPassword) {
                        if (node.packageName?.toString() != PHOTOS_PACKAGE) { complete = false; break }
                        val id = node.viewIdResourceName.orEmpty()
                        if (id.startsWith("$PHOTOS_PACKAGE:id/") && id.substringAfter(":id/") in fields) {
                            if (nodes.length() >= 64) { complete = false; break }
                            // Only cleanup opt-in reads the avatar account into memory; persist a digest,
                            // never the email, display name or content description.
                            if (includeAccount && id.endsWith("/selected_account_disc")) {
                                val matches = Regex("[A-Z0-9._%+\\-]+@[A-Z0-9.\\-]+\\.[A-Z]{2,}", RegexOption.IGNORE_CASE)
                                    .findAll(node.contentDescription?.toString().orEmpty()).toList()
                                if (matches.size == 1) {
                                    val bytes = matches.single().value.lowercase(java.util.Locale.ROOT).toByteArray()
                                    result.put("account", java.security.MessageDigest.getInstance("SHA-256").digest(bytes).joinToString("") { "%02x".format(it) })
                                }
                            }
                            val rawText = if (id.endsWith("/selected_account_disc")) "" else node.text?.toString().orEmpty()
                            val text = when {
                                id.endsWith("/title") && rawText !in setOf("Nothing to free up", "没有可释放的空间") -> ""
                                id.endsWith("/og_bento_card_title") && rawText !in setOf("Free up space on this device", "释放此设备的空间") -> ""
                                else -> rawText
                            }
                            if (text.length > 512) { complete = false; break }
                            nodes.put(JSONObject().put("id", id).put("text", text).put("actionable", node.isEnabled && node.isClickable))
                        }
                        for (index in 0 until node.childCount) {
                            if (pending.size + visited >= 512) { complete = false; break }
                            node.getChild(index)?.let(pending::addLast)
                        }
                    }
                } finally { node.recycle() }
            }
            if (pending.isNotEmpty()) complete = false
            return result.put("complete", complete)
        } finally {
            while (pending.isNotEmpty()) pending.removeFirst().recycle()
        }
    }
}
