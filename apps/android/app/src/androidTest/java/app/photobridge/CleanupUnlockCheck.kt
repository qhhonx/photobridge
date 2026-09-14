package app.photobridge

import android.accessibilityservice.AccessibilityService
import android.app.Instrumentation
import android.app.KeyguardManager
import android.content.ContextWrapper
import android.content.Intent
import android.os.PowerManager
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull

/** Isolated package: changes only the non-secure screen state, never photos. */
internal fun Instrumentation.checkCleanupUnlock(): String = runBlocking {
    check(targetContext.packageName.endsWith(".validation"))
    val keyguard = targetContext.getSystemService(KeyguardManager::class.java)
    val power = targetContext.getSystemService(PowerManager::class.java)
    check(!keyguard.isKeyguardSecure) { "test_requires_non_secure_keyguard" }
    check(CleanupUnlock.needsManualUnlock(true, true))
    check(!CleanupUnlock.needsManualUnlock(true, false))
    check(!CleanupUnlock.needsManualUnlock(false, true))
    check(uiAutomation.performGlobalAction(AccessibilityService.GLOBAL_ACTION_LOCK_SCREEN))
    repeat(20) { if (!power.isInteractive && keyguard.isKeyguardLocked) return@repeat; delay(100) }
    check(!power.isInteractive && keyguard.isKeyguardLocked) { "test_device_did_not_lock" }
    // Simulate a denied activity launch, then cancellation. The next request
    // must remain usable, with no late callback unlocking an abandoned request.
    val deniedLaunch = object : ContextWrapper(targetContext) {
        override fun startActivity(intent: Intent) = Unit
    }
    check(withTimeoutOrNull(300) { CleanupUnlock.ensureReady(deniedLaunch) } == null)
    check(!power.isInteractive && keyguard.isKeyguardLocked)
    check(CleanupUnlock.ensureReady(targetContext)) { "non_secure_unlock_failed" }
    check(power.isInteractive && !keyguard.isKeyguardLocked)
    // Already unlocked is a no-op. Stale activity callbacks cannot complete a new request.
    withContext(Dispatchers.Main) { CleanupUnlock.complete("stale-request", false) }
    check(CleanupUnlock.ensureReady(targetContext))
    "PASS: secure-lock policy; cancelled/denied launch recovery; real screen-off non-secure unlock; already-unlocked path; no Google Photos actions"
}
