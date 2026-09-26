# Beta 17

- Folder backup rules support per-source glob include/exclude patterns, including configuration before the first scan. Exclude takes priority; empty include accepts all supported media. Invalid patterns leave previous rules unchanged.
- Rules apply to scanning, indexed file browsing and preparation, including both resources of exported Live Photos. Saving requests a full rescan; queued transfers and received receipts are preserved.
- Failed files have a separate page with the sidebar and a return action. Ignore This Version removes a failure without deleting the original; bulk retries do not revive that revision. Changed versions become eligible again.

Mac requires Apple Silicon and macOS 14+. Android requires arm64 and Android 10+.
iOS 17+ is available from source. Receiver delivery and cloud backup remain separate states.
