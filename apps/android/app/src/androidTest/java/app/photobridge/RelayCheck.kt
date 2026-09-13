package app.photobridge

import android.app.Instrumentation
import android.net.Uri
import kotlinx.coroutines.delay
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

internal suspend fun Instrumentation.checkRelayRetention(root: File, photo: File, movie: File, pairing: JSONObject, publishedIds: MutableList<String>) {
    val receiverRoot = File(root,"receiver").path
    fun settings() = (NativeBridge.request(JSONObject().put("op","receiver_settings").put("root",receiverRoot)) as JSONObject).getJSONObject("settings")
    fun relay(enabled: Boolean) { NativeBridge.request(JSONObject().put("op","receiver_settings").put("root",receiverRoot).put("settings",settings().put("receiver_relay",enabled))) }
    check(!settings().getBoolean("receiver_relay")) { "relay_enabled_by_default" }
    fun resource(file: File, role: String, type: String) = JSONObject().put("path",file.path).put("role",role).put("filename",file.name).put("media_type",type)
    val ids=mutableListOf<String>(); val copies=mutableListOf<GalleryCopy>()
    for (kind in listOf("photo","video","motion","burst-primary","burst-secondary")) {
        val resources=JSONArray().put(resource(if(kind=="video") movie else photo,if(kind=="video") "video" else "photo",if(kind=="video") "video/mp4" else "image/jpeg"))
        if(kind=="motion") resources.put(resource(movie,"paired_video","video/mp4"))
        val metadata=if(kind.startsWith("burst-")) NativeBridge.request(JSONObject().put("op","burst_metadata").put("identifier","relay-${root.name}").put("primary",kind=="burst-primary")) else JSONObject()
        NativeBridge.request(JSONObject().put("op","enqueue").put("receiver_id",pairing.getString("receiver_id")).put("source_id","relay-$kind-${root.name}")
            .put("revision","1").put("kind",if(kind.startsWith("burst-")) "photo" else kind).put("metadata",metadata).put("resources",resources))
        check((NativeBridge.request(JSONObject().put("op","run_sender").put("pairing",pairing)) as JSONObject).getString("state")=="received")
        val items=NativeBridge.request(JSONObject().put("op","publications")) as JSONArray
        check(items.length()==1)
        val item=items.getJSONObject(0); val id=item.getString("id"); ids+=id; publishedIds+=id
        val copy=MediaPublisher.publish(targetContext,item); copies+=copy
        // The persisted pre-write evidence makes replay independent of codecs.
        val prepared=NativeBridge.request(JSONObject().put("op","gallery_evidence").put("id",id)) as JSONObject
        check(!prepared.getBoolean("confirmed") && GalleryCopy.parse(prepared.getJSONObject("copy"))==copy)
        check(MediaPublisher.publish(targetContext,item)==copy)
        NativeBridge.request(JSONObject().put("op","gallery_publication").put("id",id).put("copy",copy.json()))
    }
    fun history() = (NativeBridge.request(JSONObject().put("op","receiver_history").put("root",receiverRoot).put("state","all").put("kind","all")) as JSONObject).getJSONArray("items")
    fun released(id:String): Boolean { val h=history(); return (0 until h.length()).map(h::getJSONObject).first{it.getString("id")==id}.getBoolean("originals_released") }
    check(ids.none(::released)) { "disabled_relay_removed_originals" }
    targetContext.contentResolver.openOutputStream(Uri.parse(copies[0].locator),"wt")!!.use { it.write(byteArrayOf(1,2,3)) }
    targetContext.contentResolver.delete(Uri.parse(copies[1].locator),null,null)
    check(runCatching { MediaPublisher.verify(targetContext,copies[0]) }.isFailure)
    check(runCatching { MediaPublisher.verify(targetContext,copies[1]) }.isFailure)
    relay(true)
    val sweep=GalleryRetention(receiverRoot)
    repeat(4) { sweep.step(targetContext) }
    check(!released(ids[0]) && !released(ids[1])) { "damaged_or_missing_copy_reclaimed" }
    check(ids.drop(2).all(::released)) { "verified_motion_or_burst_not_reclaimed" }
    val h=history()
    check((0 until h.length()).map(h::getJSONObject).filter{it.getString("id") in ids.drop(2)}.all{it.getString("release_reason")=="gallery"})
    relay(false)
    check((NativeBridge.request(JSONObject().put("op","gallery_candidates").put("root",receiverRoot)) as JSONArray).length()==0)
    check(runCatching { NativeBridge.request(JSONObject().put("op","release_gallery").put("id",ids[0]).put("copy",copies[0].json())) }.isFailure)
    check(!released(ids[0]))
    // Restore only the synthetic photo. Disabled policy must still keep its source.
    targetContext.contentResolver.openOutputStream(Uri.parse(copies[0].locator),"wt")!!.use { out -> photo.inputStream().use { it.copyTo(out) } }
    MediaPublisher.verify(targetContext,copies[0]); sweep.step(targetContext); check(!released(ids[0]))
    relay(true); repeat(4) { sweep.step(targetContext) }; check(released(ids[0]) && !released(ids[1]))
    NativeBridge.request(JSONObject().put("op","stop_receiver"))
    var restarted: JSONObject?=null
    repeat(40) { if(restarted==null) { restarted=runCatching { NativeBridge.request(JSONObject().put("op","start_receiver").put("root",receiverRoot).put("listen","127.0.0.1:38484").put("capacity",32*1024*1024)) as JSONObject }.getOrNull(); if(restarted==null) delay(50) } }
    checkNotNull(restarted)
    check(settings().getBoolean("receiver_relay") && ids.drop(2).all(::released))
    check(NativeBridge.request(JSONObject().put("op","run_sender").put("pairing",pairing))==JSONObject.NULL) { "relay_requeued_receipts" }
    val fresh = File(root,"relay-fresh.jpg")
    val bitmap=android.graphics.Bitmap.createBitmap(64,64,android.graphics.Bitmap.Config.ARGB_8888).apply { eraseColor(android.graphics.Color.MAGENTA) }
    fresh.outputStream().use { check(bitmap.compress(android.graphics.Bitmap.CompressFormat.JPEG,95,it)) }; bitmap.recycle()
    NativeBridge.request(JSONObject().put("op","enqueue").put("receiver_id",pairing.getString("receiver_id")).put("source_id","relay-fresh-${root.name}").put("revision","1").put("kind","photo")
        .put("resources",JSONArray().put(resource(fresh,"photo","image/jpeg"))))
    check((NativeBridge.request(JSONObject().put("op","run_sender").put("pairing",pairing)) as JSONObject).getString("state")=="received")
    val item=(NativeBridge.request(JSONObject().put("op","publications")) as JSONArray).getJSONObject(0)
    val freshID=item.getString("id");publishedIds+=freshID
    val freshCopy=MediaPublisher.publish(targetContext,item)
    val result=NativeBridge.request(JSONObject().put("op","gallery_publication").put("id",freshID).put("copy",freshCopy.json())) as JSONObject
    check(result.getLong("bytes")==fresh.length() && released(freshID)) { "new_publication_not_automatically_reclaimed" }
    MediaPublisher.verify(targetContext,freshCopy)
    check(NativeBridge.request(JSONObject().put("op","run_sender").put("pairing",pairing))==JSONObject.NULL)

}
