package app.photobridge

import android.app.Instrumentation
import android.os.Bundle
import java.io.File
import java.security.MessageDigest

/** Uses an isolated installed APK and real PackageManager certificate parsing. */
internal fun Instrumentation.checkAppUpdates(arguments: Bundle): String {
    val context = targetContext
    check(context.packageName == "app.photobridge.updatefixture")
    val root = File(context.filesDir, "update-fixtures")
    val candidate = File(root, "next.apk")
    val wrongSignature = File(root, "other-key.apk")
    fun manifest(file: File, build: Long = 2) = AppRelease("fixture", build, "https://example.invalid/app.apk",
        MessageDigest.getInstance("SHA-256").digest(file.readBytes()).joinToString("") { "%02x".format(it) }, file.length())
    val accepted = manifest(candidate)
    AppUpdates.verify(context, candidate, accepted)
    check(runCatching { AppUpdates.verify(context, candidate, accepted.copy(sha256 = "0".repeat(64))) }.isFailure)
    check(runCatching { AppUpdates.verify(context, candidate, accepted.copy(size = accepted.size + 1)) }.isFailure)
    check(runCatching { AppUpdates.verify(context, candidate, accepted.copy(build = 1)) }.isFailure)
    check(runCatching { AppUpdates.verify(context, wrongSignature, manifest(wrongSignature)) }.isFailure)
    return "PASS: genuine higher build accepted; digest, size, rollback and foreign signing identity rejected"
}
