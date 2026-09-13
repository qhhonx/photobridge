package app.photobridge

import android.content.Context
import android.provider.MediaStore
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive

internal data class GalleryUsage(val readyBytes: Long, val readyCount: Long, val pendingBytes: Long, val pendingCount: Long, val unknownSizes: Long)

/** Indexed file sizes for this installation's delivery copies, never cloud state. */
internal object GalleryInventory {
    private val deliveryName = Regex("^PB_[0-9a-f]{64}(?:\\.[^/]+)?$")
    @Suppress("DEPRECATION")
    suspend fun read(context: Context): GalleryUsage {
        var bytes = 0L; var count = 0L; var pendingBytes = 0L; var pendingCount = 0L; var unknown = 0L
        val columns = arrayOf(MediaStore.MediaColumns.DISPLAY_NAME, MediaStore.MediaColumns.SIZE,
            MediaStore.MediaColumns.IS_PENDING, MediaStore.MediaColumns.OWNER_PACKAGE_NAME)
        for (collection in listOf(MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY),
            MediaStore.Video.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY))) {
            currentCoroutineContext().ensureActive()
            // API 29 supports this query parameter; owner and path restrict scope.
            // No shared-library permission or file decoding is needed.
            val uri = MediaStore.setIncludePending(collection)
            val cursor = checkNotNull(context.contentResolver.query(uri, columns,
                "${MediaStore.MediaColumns.RELATIVE_PATH}=?", arrayOf("DCIM/PhotoBridge/"), null))
            cursor.use {
                while (it.moveToNext()) {
                    currentCoroutineContext().ensureActive()
                    if (it.getString(3) != context.packageName || !deliveryName.matches(it.getString(0) ?: "")) continue
                    val size = if (it.isNull(1) || it.getLong(1) < 0) { unknown++; 0L } else it.getLong(1)
                    if (it.getInt(2) == 0) { bytes = Math.addExact(bytes, size); count++ }
                    else { pendingBytes = Math.addExact(pendingBytes, size); pendingCount++ }
                }
            }
        }
        return GalleryUsage(bytes, count, pendingBytes, pendingCount, unknown)
    }
}
