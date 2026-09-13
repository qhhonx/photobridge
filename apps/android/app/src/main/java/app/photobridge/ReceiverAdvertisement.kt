package app.photobridge

import android.content.Context
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo

/** Public routing hint only. Pairing credentials/certificates never enter DNS-SD. */
internal class ReceiverAdvertisement(context: Context, receiverId: String, port: Int) : AutoCloseable {
    private val manager = context.getSystemService(NsdManager::class.java)
    private var closed = false
    private var registered = false
    private val listener = object : NsdManager.RegistrationListener {
        override fun onServiceRegistered(info: NsdServiceInfo) {
            synchronized(this@ReceiverAdvertisement) {
                registered = true
                if (closed) unregister()
            }
        }
        override fun onRegistrationFailed(info: NsdServiceInfo, code: Int) = Unit
        override fun onServiceUnregistered(info: NsdServiceInfo) = Unit
        override fun onUnregistrationFailed(info: NsdServiceInfo, code: Int) = Unit
    }
    init {
        require(receiverId.matches(Regex("[a-f0-9]{64}")) && port in 1..65535)
        val info = NsdServiceInfo().apply {
            serviceName = "PhotoBridge-${receiverId.take(20)}"
            serviceType = "_photobridge._tcp."
            setPort(port)
            setAttribute("id", receiverId)
        }
        manager.registerService(info, NsdManager.PROTOCOL_DNS_SD, listener)
    }
    private fun unregister() {
        if (registered) {
            registered = false
            runCatching { manager.unregisterService(listener) }
        }
    }
    override fun close() {
        synchronized(this) { closed = true; unregister() }
    }
}
