# PhotoBridge

<img src="assets/AppIcon.png" width="112" alt="PhotoBridge icon">

Back up original photos, videos and supported Live Photos directly from iPhone or
Mac to your Android receiver. No PhotoBridge account or cloud subscription is
required for local transfers.

[Official website](https://photobridge-app.vercel.app) ·
[Downloads](https://github.com/qhhonx/photobridge/releases) ·
[Installation](docs/installation.md) · [Architecture](docs/architecture.md)

## License and commercial use

**Source available. Free for noncommercial use. Commercial rights are reserved.**
This checkout is offered under the [PolyForm Noncommercial License 1.0.0](LICENSE).
Commercial use outside the license's permitted purposes requires separate written
permission from the relevant PhotoBridge copyright holders. This applies to
individuals as well as companies, including paid App Store distribution,
subscriptions, advertising-supported repackaging and paid hosted services.

See the [licensing policy](docs/licensing.md) for permitted uses and commercial
licensing requests.

## What is available

- Native SwiftUI iOS/macOS senders with PhotoKit libraries, reusable thumbnail
  grids, new-photo backup, existing-library import and visible transfer progress.
- A Kotlin Android receiver with gallery publication, supported motion-photo
  conversion, burst metadata and per-sender transfer history.
- A shared Rust engine with paired HTTPS, certificate pinning, original-resource
  checksums, resumable uploads, persistent queues and automatic transient retries.
- Configurable storage budgets, private-staging reclamation and diagnostics.
- Signed Sparkle updates on Mac and verified, user-confirmed APK updates on Android.
- An English/Simplified Chinese website and native interfaces.

**Early beta:** preserve your source library and another trusted backup. Receiver
receipt and Google Photos cloud backup are separate states. iOS background work is
system-scheduled. Experimental Google Photos cleanup may need attention when its
interface changes. See [validation and limits](docs/native-validation.md).

Mac requires Apple Silicon and macOS 14+. Android requires arm64 and Android 10+.
iOS 17+ is available from source; there is no App Store/TestFlight release yet.
Windows, NAS and hosted receivers are not shipped. Native hosts own platform media,
UI and background execution; the Rust core owns transfer and durable state.

## Try a local backup

Requires Rust 1.90 (pinned in `rust-toolchain.toml`). Run from this checkout:

```sh
cargo build --workspace --locked
mkdir -p runtime/source
cargo run -p photobridge -- init-token runtime/local.token
cargo run -p photobridge -- serve --root runtime/receiver --token-file runtime/local.token
```

In another terminal, put an original `photo.jpg` in `runtime/source`, then run:

```sh
cargo run -p photobridge -- manifest --source-id example-photo --file runtime/source/photo.jpg --output runtime/photo.json
cargo run -p photobridge -- send --server http://127.0.0.1:8484 --token-file runtime/local.token --manifest runtime/photo.json --files runtime/source
```

For a motion asset, add its original paired video with
`--paired-video runtime/source/paired.mov` when creating the manifest. Both
resources are preserved byte-for-byte. This command groups the supplied resources;
it does not validate Apple pairing metadata or convert them to Motion Photo yet.

Stop and restart the receiver, then repeat `send` with the same manifest. The
sender queries persisted receiver offsets and sends only missing bytes. A received
asset returns immediately even if the source files have since moved.

The CLI writes content-addressed originals under `runtime/receiver/blobs` and
keeps manifests and status in `receiver.sqlite3`. **Received** means this receiver
saved and verified all resources. It does not mean Google Photos backed them up.
There are no source deletion or device cleanup commands.

Tokens are generated into a file and are never printed. Do not commit or share
them. Non-loopback HTTP destinations are rejected, redirects are disabled, and
unauthenticated requests are rejected. Native hosts use a pinned certificate and private bearer pairing code over HTTPS.
The CLI harness remains loopback-only.

## Validation

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
python3 tests/cli_smoke.py
```

Tests cover interrupted transfers, process restart, repeated chunks, corrupt data,
capacity rollback, whole-motion-asset commit, independent target processing,
unauthorized requests, cancellation, request-size limits, and real HTTP transfer.
No personal photos, device connections, or external accounts are used.

## Next milestones

1. Validate locked-device scheduling and network interruptions over extended runs.
2. Extend physical-device coverage for receiver discovery, network changes and background recovery.
3. Validate storage policies and native processing recovery with real mixed libraries.
4. Refine native library UX and add Windows/Android sender hosts against the same engine.

NAS and online deployments remain unspecified until concrete requirements exist.
