package app.photobridge

import android.app.Instrumentation
import android.net.ConnectivityManager
import android.net.NetworkCapabilities
import android.os.Bundle
import org.json.JSONObject
import java.io.File
import java.net.Inet4Address
import java.util.UUID

/** Synthetic receiver advertised for cross-platform discovery acceptance. */
internal fun Instrumentation.checkReceiverDiscovery(): String {
    check(targetContext.packageName.endsWith(".validation"))
    val connectivity = targetContext.getSystemService(ConnectivityManager::class.java)
    val network = connectivity.activeNetwork
    check(connectivity.getNetworkCapabilities(network)?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true)
    val ip = connectivity.getLinkProperties(network)?.linkAddresses?.map { it.address }?.filterIsInstance<Inet4Address>()?.firstOrNull()?.hostAddress
        ?: error("wifi_required")
    val root = File(targetContext.filesDir, "discovery-${UUID.randomUUID()}").apply { mkdirs() }
    val fixture = File(targetContext.cacheDir, "receiver-discovery-fixture.json")
    var advertisement: ReceiverAdvertisement? = null
    try {
        val saved = NativeBridge.request(JSONObject().put("op", "start_receiver").put("root", root.path).put("listen", "127.0.0.1:38584").put("capacity", 1024 * 1024)) as JSONObject
        NativeBridge.request(JSONObject().put("op", "stop_receiver"))
        val current = NativeBridge.request(JSONObject().put("op", "start_receiver").put("root", root.path).put("listen", "$ip:38584").put("capacity", 1024 * 1024)) as JSONObject
        check(current.getString("receiver_id") == saved.getString("receiver_id"))
        check(current.getString("certificate_name") == "127.0.0.1")
        val checked = NativeBridge.request(JSONObject().put("op", "relocate_pairing").put("pairing", saved).put("endpoint", current.getString("endpoint"))) as JSONObject
        check(checked.getString("receiver_id") == saved.getString("receiver_id"))
        fixture.writeText(JSONObject().put("saved", saved).put("endpoint", current.getString("endpoint")).toString())
        advertisement = ReceiverAdvertisement(targetContext, current.getString("receiver_id"), 38584)
        sendStatus(0, Bundle().apply { putString("result", "READY: isolated receiver discovery fixture; no production state used") })
        Thread.sleep(120_000) // Bounded lifetime for the external Apple acceptance test.
        return "PASS: same identity at new network address, authenticated native TLS and bounded Bonjour advertisement"
    } finally {
        advertisement?.close()
        runCatching { NativeBridge.request(JSONObject().put("op", "stop_receiver")) }
        fixture.delete(); root.deleteRecursively()
    }
}
