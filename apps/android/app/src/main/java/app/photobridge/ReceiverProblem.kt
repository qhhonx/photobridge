package app.photobridge

internal enum class ReceiverRecovery { NETWORK, STORAGE, RESUME, CONNECTION_HELP, DIAGNOSTICS_HELP }
internal data class ReceiverProblem(val message: Int, val action: Int, val recovery: ReceiverRecovery)

/** Presentation of fixed error categories only; never expose exception text. */
internal fun receiverProblem(snapshot: ReceiverSnapshot): ReceiverProblem? = when (snapshot.error) {
    "wifi_required" -> ReceiverProblem(R.string.recovery_wifi, R.string.recovery_network_action, ReceiverRecovery.NETWORK)
    "receiver_address_changed" -> ReceiverProblem(R.string.recovery_address, R.string.help_connection_title, ReceiverRecovery.CONNECTION_HELP)
    "system_time_limit", "foreground_start_blocked" -> ReceiverProblem(R.string.recovery_system_limit, R.string.receiver_start, ReceiverRecovery.RESUME)
    "processing_low_space", "low_space", "capacity" -> ReceiverProblem(R.string.recovery_space, R.string.nav_storage, ReceiverRecovery.STORAGE)
    null -> null
    else -> ReceiverProblem(R.string.recovery_general, R.string.help_recovery_title, ReceiverRecovery.DIAGNOSTICS_HELP)
}
