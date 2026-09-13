package app.photobridge

import android.app.*
import android.content.Intent
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.IBinder
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import org.json.JSONArray
import org.json.JSONObject
import java.net.Inet4Address

internal data class RecentTransfer(val id: String, val filename: String, val kind: String, val totalBytes: Long,
    val confirmedBytes: Long, val receipt: String, val processing: String)
internal data class ReceiverSnapshot(val phase: String = "idle", val published: Int = 0, val error: String? = null,
    val received: Int = 0, val total: Int = 0, val failed: Int = 0, val reservedBytes: Long = 0,
    val capacityBytes: Long = 0, val recent: List<RecentTransfer> = emptyList(), val processingName: String? = null)
internal object ReceiverState {
    val mutable = MutableStateFlow(ReceiverSnapshot())
    val snapshot = mutable.asStateFlow()
    val lifecycle = Mutex()
    val mediaOperations = Mutex()
    // Deliberately excluded from snapshots and their generated toString methods.
    @Volatile var pairing: String? = null
}

class ReceiverService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var started = false
    private var cursor = ""
    private var spaceBlocked = false
    private val retention = GalleryRetention()
    private val cleanup by lazy { PhotosCleanup(this) }

    override fun onBind(intent: Intent?): IBinder? = null
    override fun onCreate() {
        super.onCreate()
        getSystemService(NotificationManager::class.java).createNotificationChannel(
            NotificationChannel("receiver", getString(R.string.app_name), NotificationManager.IMPORTANCE_LOW)
        )
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        startForeground(1, Notification.Builder(this, "receiver")
            .setContentTitle(getString(R.string.app_name)).setContentText(getString(R.string.receiver_notification))
            .setSmallIcon(android.R.drawable.stat_sys_upload).setContentIntent(open).build())
    }
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (!ReceiverPreferences.enabled(this)) { stopSelf(); return START_NOT_STICKY }
        if (started) return START_STICKY
        started = true
        scope.launch {
            // A replacement service waits for the previous receiver to stop.
            ReceiverState.lifecycle.withLock {
                while (isActive) {
                    ReceiverState.mutable.update { it.copy(phase = "starting") }
                    var opened = false
                    var dashboard: Job? = null
                    var advertisement: ReceiverAdvertisement? = null
                    try {
                        val address = wifiAddress()
                        // Create a locale-appropriate stable name even after boot,
                        // before a sender's first optional profile exchange.
                        runCatching { DeviceProfiles.read(this@ReceiverService) }
                        val pairing = NativeBridge.request(JSONObject().put("op", "start_receiver")
                            .put("root", "$filesDir/receiver").put("listen", "$address:8484")
                            .put("capacity", 6L * 1024 * 1024 * 1024)) as JSONObject
                        opened = true
                        NativeBridge.request(JSONObject().put("op", "receiver_transfer_hold").put("held", cleanup.held))
                        ReceiverState.pairing = pairing.toString()
                        advertisement = runCatching { ReceiverAdvertisement(this@ReceiverService, pairing.getString("receiver_id"), 8484) }.getOrNull()
                        ReceiverState.mutable.value = ReceiverSnapshot(phase = "ready")
                        dashboard = launch {
                            while (isActive) {
                                runCatching {
                                    val summary = NativeBridge.request(JSONObject().put("op", "receiver_overview")) as JSONObject
                                    val rows = summary.getJSONArray("recent")
                                    val recent = (0 until rows.length()).map { index ->
                                        val row = rows.getJSONObject(index)
                                        RecentTransfer(row.getString("id"), row.getString("filename"), row.getString("kind"),
                                            row.getLong("total_bytes"), row.getLong("confirmed_bytes"), row.getString("receipt"), row.getString("processing"))
                                    }
                                    ReceiverState.mutable.update { it.copy(received = summary.getInt("received"), total = summary.getInt("total"),
                                        published = summary.getInt("published"), failed = summary.getInt("failed"), reservedBytes = summary.getLong("reserved_bytes"),
                                        capacityBytes = summary.getLong("capacity_bytes"), recent = recent) }
                                }
                                delay(1_000)
                            }
                        }
                        while (isActive) {
                            val currentAddress = runCatching { wifiAddress() }.getOrNull()
                            if (currentAddress != address) {
                                ReceiverState.mutable.update { it.copy(error = if (currentAddress == null) "wifi_required" else "receiver_address_changed") }
                                break
                            }
                            val healthy = NativeBridge.request(JSONObject().put("op", "receiver_status")) as JSONObject
                            check(healthy.getBoolean("running")) { "receiver_unavailable" }
                            val space = NativeBridge.request(JSONObject().put("op", "receiver_overview")) as JSONObject
                            cleanup.tick(space.getLong("free_bytes"), space.getLong("min_free_bytes"))
                            if (!cleanup.held) ReceiverState.mediaOperations.withLock { retention.step(this@ReceiverService); publish() }
                            delay(5_000)
                        }
                    } catch (cancelled: CancellationException) { throw cancelled }
                    catch (error: Exception) {
                        ReceiverState.mutable.update { it.copy(phase = "waiting", error = safeError(error)) }
                    } finally {
                        withContext(NonCancellable + Dispatchers.IO) {
                            advertisement?.close()
                            dashboard?.cancelAndJoin()
                            ReceiverState.pairing = null
                            if (opened) runCatching { NativeBridge.request(JSONObject().put("op", "stop_receiver")) }
                            ReceiverState.mutable.update { it.copy(phase = if (scope.isActive) "waiting" else "idle", processingName = null) }
                        }
                    }
                    delay(5_000)
                }
            }
        }
        return START_STICKY
    }
    private suspend fun publish() {
        val items = NativeBridge.request(JSONObject().put("op", "publications").put("after", cursor)) as JSONArray
        if (items.length() == 0) { cursor = ""; return }
        for (index in 0 until items.length()) {
            currentCoroutineContext().ensureActive()
            val item = items.getJSONObject(index)
            cursor = item.getString("id")
            if (item.getString("processing") == "failed") continue
            val overview = NativeBridge.request(JSONObject().put("op", "receiver_overview")) as JSONObject
            val resources = item.getJSONObject("asset").getJSONArray("resources")
            var originals = 0L
            for (resource in 0 until resources.length()) originals = Math.addExact(originals, resources.getJSONObject(resource).getLong("size"))
            // Leave room for conversion scratch data and the MediaStore copy.
            val factor = if (item.getJSONObject("asset").getString("kind") == "motion") 3L else 1L
            val needed = Math.addExact(Math.multiplyExact(originals, factor), 32L shl 20)
            if (overview.getLong("free_bytes") - overview.getLong("min_free_bytes") < needed) {
                ReceiverState.mutable.update { it.copy(error = "processing_low_space", processingName = null) }
                if (!spaceBlocked) NativeBridge.request(JSONObject().put("op", "record_event").put("receiver", true).put("code", "processing_low_space"))
                spaceBlocked = true
                continue
            }
            if (spaceBlocked) {
                NativeBridge.request(JSONObject().put("op", "record_event").put("receiver", true).put("code", "processing_resumed"))
                spaceBlocked = false
            }
            val filename = item.getJSONObject("asset").getJSONArray("resources").getJSONObject(0).getString("filename")
            ReceiverState.mutable.update { it.copy(processingName = filename) }
            val success = try {
                val copy = MediaPublisher.publish(this, item)
                NativeBridge.request(JSONObject().put("op", "gallery_publication").put("id", cursor).put("copy", copy.json()))
                ReceiverState.mutable.update { it.copy(error = null) }
                true
            } catch (cancelled: CancellationException) { throw cancelled }
            catch (error: Exception) {
                ReceiverState.mutable.update { it.copy(error = safeError(error)) }
                false
            }
            if (!success) NativeBridge.request(JSONObject().put("op", "processed").put("id", cursor).put("success", false))
            ReceiverState.mutable.update { it.copy(processingName = null) }
        }
    }
    private fun wifiAddress(): String {
        val manager = getSystemService(ConnectivityManager::class.java)
        val network = manager.activeNetwork
        val capabilities = manager.getNetworkCapabilities(network)
        check(capabilities?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true) { "wifi_required" }
        return manager.getLinkProperties(network)?.linkAddresses?.map { it.address }
            ?.filterIsInstance<Inet4Address>()?.firstOrNull { !it.isLoopbackAddress }?.hostAddress
            ?: error("wifi_required")
    }
    override fun onTimeout(startId: Int, fgsType: Int) {
        ReceiverState.mutable.update { it.copy(phase = "waiting", error = "system_time_limit") }
        stopSelf()
    }
    override fun onDestroy() { scope.cancel(); super.onDestroy() }
}

internal fun safeError(error: Exception): String = error.message?.takeIf { it.matches(Regex("[a-z_]{1,40}")) } ?: "operation_failed"
