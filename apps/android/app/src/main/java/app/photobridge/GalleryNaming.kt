package app.photobridge

import org.json.JSONObject
import java.time.Instant
import java.time.ZoneOffset
import java.time.format.DateTimeFormatter
import java.util.Locale

/** Stable, readable delivery names. The full asset identity remains in the receiver store. */
internal object GalleryNaming {
    private val dateFormat = DateTimeFormatter.ofPattern("uuuuMMdd_HHmmss'Z'", Locale.ROOT).withZone(ZoneOffset.UTC)
    private val assetID = Regex("[0-9a-f]{64}")
    private val safeExtension = Regex("[A-Za-z0-9]{1,12}")

    private fun extension(item: JSONObject): String {
        val asset = item.getJSONObject("asset")
        if (asset.getString("kind") == "motion" || asset.optJSONObject("metadata")?.has("burst_group_ref") == true) return ".jpg"
        val resource = asset.getJSONArray("resources").getJSONObject(0)
        val raw = resource.getString("filename").substringAfterLast('.', "")
        if (raw.isEmpty()) return ""
        check(safeExtension.matches(raw)) { "unsupported_extension" }
        return ".$raw"
    }

    fun legacyName(item: JSONObject): String = "PB_${item.getString("id")}${extension(item)}"

    fun name(item: JSONObject): String {
        val id = item.getString("id")
        check(assetID.matches(id)) { "invalid_asset_id" }
        val asset = item.getJSONObject("asset")
        val captured = MediaDates.captured(asset.optJSONObject("metadata"))
        val date = captured?.let { dateFormat.format(Instant.ofEpochMilli(it)) } ?: "undated"
        return "PB_${date}_${id.take(16)}${extension(item)}"
    }
}
