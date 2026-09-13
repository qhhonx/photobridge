package app.photobridge

import org.json.JSONObject

object NativeBridge {
    init { System.loadLibrary("photobridge_native") }
    @JvmStatic external fun call(request: String): String

    // Blocking calls belong on Dispatchers.IO. Never log pairing requests.
    fun request(request: JSONObject): Any {
        val response = JSONObject(call(request.toString()))
        check(response.getBoolean("ok")) { response.getString("error") }
        return response.get("value")
    }
}
