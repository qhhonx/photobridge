# Beta 7

- Pause photo scanning and original preparation when the sender is on cellular or cannot reach its paired receiver. Resume after the receiver is verified again.
- Show live per-task upload progress without treating sent bytes as a receiver receipt.
- Keep concurrent upload status independent, avoiding repeated switches between transmitting and system scheduling.
- Show a separate confirmation stage after sending finishes, and limit progress UI updates to avoid excessive redraws.

Existing pairings, pending sources, receipts and manual pause settings are preserved.
Mac and iOS share these fixes. iOS background execution remains scheduled by the system.

Mac requires Apple Silicon and macOS 14+. Android requires arm64 and Android 10+.
iOS 17+ is available from source. The Mac beta is not Apple-notarized.
Receiver delivery and Google Photos cloud backup remain separate states.
