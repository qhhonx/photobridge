package app.photobridge

import android.app.Activity
import android.app.KeyguardManager
import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.os.PowerManager
import android.view.WindowManager
import android.widget.TextView
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import java.lang.ref.WeakReference
import java.util.UUID

/** Only a live, in-process cleanup request may dismiss a non-secure keyguard. */
internal object CleanupUnlock {
    private var request: String? = null
    private var result: CompletableDeferred<Boolean>? = null
    private var activity = WeakReference<CleanupUnlockActivity>(null)

    internal fun needsManualUnlock(locked: Boolean, secure: Boolean) = locked && secure

    suspend fun ensureReady(context: Context): Boolean = withContext(Dispatchers.Main.immediate) {
        val keyguard = context.getSystemService(KeyguardManager::class.java)
        val power = context.getSystemService(PowerManager::class.java)
        if (needsManualUnlock(keyguard.isKeyguardLocked, keyguard.isKeyguardSecure)) return@withContext false
        if (!keyguard.isKeyguardLocked && power.isInteractive) return@withContext true
        check(request == null) { "unlock_busy" }
        val id = UUID.randomUUID().toString()
        val completion = CompletableDeferred<Boolean>()
        request = id; result = completion
        try {
            context.startActivity(Intent(context, CleanupUnlockActivity::class.java)
                .putExtra("request", id).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            withTimeoutOrNull(12_000) { completion.await() } == true
                && !keyguard.isKeyguardLocked && power.isInteractive
        } finally {
            request = null; result = null
            activity.get()?.finish(); activity.clear()
        }
    }
    fun attach(id: String?, owner: CleanupUnlockActivity): Boolean {
        if (id == null || id != request) return false
        activity = WeakReference(owner)
        return true
    }
    fun complete(id: String?, success: Boolean) {
        if (id != null && id == request) result?.complete(success)
    }
}

/** Visible only while waking; it never displays or enters credentials. */
class CleanupUnlockActivity : Activity() {
    private var requested = false
    private val requestID get() = intent.getStringExtra("request")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (!CleanupUnlock.attach(requestID, this)) { finish(); return }
        val keyguard = getSystemService(KeyguardManager::class.java)
        if (CleanupUnlock.needsManualUnlock(keyguard.isKeyguardLocked, keyguard.isKeyguardSecure)) {
            complete(false); return
        }
        setShowWhenLocked(true)
        setTurnScreenOn(true)
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        setContentView(TextView(this).apply {
            setText(R.string.cleanup_result_waking)
            gravity = android.view.Gravity.CENTER
            textSize = 18f
            setPadding(32, 32, 32, 32)
        })
    }
    override fun onPostResume() {
        super.onPostResume()
        if (requested || isFinishing) return
        requested = true
        window.decorView.post {
            if (isFinishing) return@post
            val keyguard = getSystemService(KeyguardManager::class.java)
            if (CleanupUnlock.needsManualUnlock(keyguard.isKeyguardLocked, keyguard.isKeyguardSecure)) {
                complete(false)
            } else if (!keyguard.isKeyguardLocked) {
                complete(getSystemService(PowerManager::class.java).isInteractive)
            } else {
                keyguard.requestDismissKeyguard(this, object : KeyguardManager.KeyguardDismissCallback() {
                    override fun onDismissSucceeded() = complete(!keyguard.isKeyguardLocked)
                    override fun onDismissCancelled() = complete(false)
                    override fun onDismissError() = complete(false)
                })
            }
        }
    }
    private fun complete(success: Boolean) {
        CleanupUnlock.complete(requestID, success)
        finish()
    }
    override fun onDestroy() {
        CleanupUnlock.complete(requestID, false)
        super.onDestroy()
    }
}
