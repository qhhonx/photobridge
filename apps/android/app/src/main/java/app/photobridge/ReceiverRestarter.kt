package app.photobridge

import android.app.*
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Build

internal object ReceiverPreferences {
    private fun prefs(context: Context) = context.getSharedPreferences("receiver_lifecycle", Context.MODE_PRIVATE)
    fun enabled(context: Context) = prefs(context).getBoolean("enabled", false)
    fun setEnabled(context: Context, enabled: Boolean) { prefs(context).edit().putBoolean("enabled", enabled).commit() }
    fun restore(context: Context) = prefs(context).getBoolean("restore", true)
    fun setRestore(context: Context, enabled: Boolean) { prefs(context).edit().putBoolean("restore", enabled).apply() }
}
class ReceiverRestarter : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action !in listOf(Intent.ACTION_BOOT_COMPLETED, Intent.ACTION_MY_PACKAGE_REPLACED)) return
        if (!ReceiverPreferences.enabled(context) || !ReceiverPreferences.restore(context)) return
        if (Build.VERSION.SDK_INT < 35) {
            runCatching { context.startForegroundService(Intent(context, ReceiverService::class.java)) }
        } else {
            // Android 15 prohibits dataSync FGS launches from BOOT_COMPLETED.
            val notifications = context.getSystemService(NotificationManager::class.java)
            notifications.createNotificationChannel(NotificationChannel("receiver", context.getString(R.string.app_name), NotificationManager.IMPORTANCE_LOW))
            val open = PendingIntent.getActivity(context, 0, Intent(context, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
            runCatching { notifications.notify(2, Notification.Builder(context, "receiver").setSmallIcon(android.R.drawable.stat_sys_upload)
                .setContentTitle(context.getString(R.string.app_name)).setContentText(context.getString(R.string.receiver_resume_notice)).setContentIntent(open).setAutoCancel(true).build()) }
        }
    }
}
