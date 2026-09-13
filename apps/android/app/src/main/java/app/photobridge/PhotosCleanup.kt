package app.photobridge

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import org.json.JSONObject

/** Receiver-local consent and crash fence. Never stores account text or page dumps. */
internal class PhotosCleanup(private val context: Context) {
    private val prefs = context.getSharedPreferences("photos_cleanup", Context.MODE_PRIVATE)
    var enabled: Boolean
        get() = prefs.getBoolean("enabled", false)
        set(value) { check(prefs.edit().putBoolean("enabled", value).commit()) }
    var account: String
        get() = prefs.getString("account", "").orEmpty()
        set(value) { check(prefs.edit().putString("account", value).commit()) }
    val state: JSONObject get() = runCatching { JSONObject(prefs.getString("state", "{}").orEmpty()) }.getOrDefault(JSONObject())
    val pending: Boolean get() = state.optString("phase") in setOf("pending", "returning")
    val held: Boolean get() = pending || PhotosProbeService.cleanupRunning
    val reason: String get() = prefs.getString("reason", "ready").orEmpty()
    fun save(state: JSONObject, reason: String) {
        check(prefs.edit().putString("state", state.toString()).putString("reason", reason).commit())
    }
    fun report(reason: String) { check(prefs.edit().putString("reason", reason).commit()) }
    fun reviewed() { save(JSONObject(), "reviewed"); cooldown(15 * 60_000) }
    fun cooldown(delay: Long) { prefs.edit().putLong("next", System.currentTimeMillis() + delay).apply() }
    fun trigger(reserve: Long): Long = reserve + (1L shl 30)
    suspend fun tick(free: Long, reserve: Long) {
        if (enabled && account.isNotEmpty() && !PhotosProbeService.state.value.running &&
            (pending || free < trigger(reserve)) && System.currentTimeMillis() >= prefs.getLong("next", 0)) {
            cooldown(if (pending) 60_000 else 5 * 60_000)
            withContext(Dispatchers.Main) { PhotosProbeService.clean() }
        }
        NativeBridge.request(JSONObject().put("op", "receiver_transfer_hold").put("held", held))
    }
}
