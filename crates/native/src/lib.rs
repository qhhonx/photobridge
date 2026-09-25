//! Native host boundary. JSON is an FFI wire format, not a second backup engine.
//! All calls are thread-safe; blocking calls belong on a native worker thread.
mod background;
mod maintenance;
mod pairing;
mod receiver_storage;
mod source_locations;
pub use background::NativeRequest;

use base64::{engine::general_purpose::STANDARD, Engine};
use photobridge_core::*;
use photobridge_sender::{Attempt, Failure, Job, JobQuery, Sender};
use photobridge_store::Receiver;
use photobridge_transport::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{c_char, CStr, CString},
    fs::{self, File, OpenOptions},
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pairing {
    pub version: u32,
    pub receiver_id: String,
    pub endpoint: String,
    pub certificate: String,
    pub token: String,
    /// Original certificate SAN; absent in older pairing payloads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub certificate_name: Option<String>,
}
impl Pairing {
    pub fn client(&self) -> Result<Client> {
        let der = STANDARD
            .decode(&self.certificate)
            .map_err(|_| Error::Invalid("pairing certificate".into()))?;
        if self.version != PROTOCOL_VERSION
            || self.receiver_id != digest(&der)
            || !self.endpoint.starts_with("https://")
            || der.len() > 16384
        {
            return Err(Error::Invalid("pairing identity".into()));
        }
        Client::with_certificate_name(
            &self.endpoint,
            &self.token,
            Some(&der),
            self.certificate_name.as_deref(),
        )
    }
}
fn relocated_pairing(saved: &Pairing, endpoint: &str) -> Result<Pairing> {
    saved.client()?;
    pairing::validate_local_endpoint(endpoint)?;
    let mut updated = saved.clone();
    if updated.certificate_name.is_none() {
        let origin = reqwest::Url::parse(&saved.endpoint)
            .map_err(|_| Error::Invalid("pairing origin".into()))?;
        updated.certificate_name = Some(
            origin
                .host_str()
                .ok_or_else(|| Error::Invalid("pairing host".into()))?
                .trim_matches(['[', ']'])
                .to_string(),
        );
    }
    updated.endpoint = endpoint.into();
    updated.client()?;
    Ok(updated)
}

#[derive(Serialize, Deserialize)]
struct Identity {
    pairing: Pairing,
    key_pem: String,
    cert_pem: String,
}
fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut o = OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}
fn receiver_connection_info(root: &Path) -> Result<Value> {
    let hosts = HOSTS.lock().map_err(lock)?;
    let endpoint = hosts
        .receiver
        .as_ref()
        .filter(|host| host.root == root)
        .map(|host| host.pairing.endpoint.as_str());
    Ok(json!({"endpoint": endpoint}))
}

fn identity(root: &Path, addr: SocketAddr) -> Result<Identity> {
    let path = root.join("identity.json");
    let endpoint = format!("https://{addr}");
    if path.exists() {
        let mut old: Identity = serde_json::from_slice(&fs::read(&path)?)?;
        old.pairing.client()?;
        if old.pairing.endpoint != endpoint {
            old.pairing = relocated_pairing(&old.pairing, &endpoint)?;
        }
        // identity.json remains the original trust anchor. Network changes never
        // rewrite its key, certificate, receiver ID or token.
        return Ok(old);
    }
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec![addr.ip().to_string()])
            .map_err(|_| Error::Storage("TLS identity generation".into()))?;
    let mut random = [0; 32];
    getrandom::fill(&mut random).map_err(|_| Error::Storage("random source".into()))?;
    let token = random
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect::<String>();
    let der = cert.der();
    let i = Identity {
        pairing: Pairing {
            version: 1,
            receiver_id: digest(der),
            endpoint,
            certificate: STANDARD.encode(der),
            token,
            certificate_name: None,
        },
        key_pem: key_pair.serialize_pem(),
        cert_pem: cert.pem(),
    };
    private_write(&path, &serde_json::to_vec(&i)?)?;
    Ok(i)
}
pub struct ReceiverHost {
    root: PathBuf,
    pub receiver: Arc<Mutex<Receiver>>,
    pub pairing: Pairing,
    maintenance: Arc<Mutex<maintenance::Maintenance>>,
    handle: axum_server::Handle,
    task: tokio::task::JoinHandle<()>,
}
impl ReceiverHost {
    pub async fn start(root: &Path, addr: SocketAddr, capacity: u64) -> Result<Self> {
        if addr.ip().is_unspecified() || addr.port() == 0 {
            return Err(Error::Invalid(
                "concrete receiver interface and port required".into(),
            ));
        }
        let _ = rustls::crypto::ring::default_provider().install_default();
        fs::create_dir_all(root)?;
        let receiver = Arc::new(Mutex::new(Receiver::open(root.join("store"), capacity)?));
        let ident = identity(root, addr)?;
        let mut maintenance = maintenance::Maintenance::open(root)?;
        if !root.join("maintenance.initialized").exists() {
            let mut initial = maintenance.settings.clone();
            initial.receiver_budget_bytes = capacity;
            maintenance.save(initial)?;
            private_write(&root.join("maintenance.initialized"), b"1")?;
        }
        receiver.lock().map_err(lock)?.configure_storage(
            maintenance.settings.receiver_budget_bytes,
            maintenance.settings.min_free_bytes,
        )?;
        maintenance.log("receiver_started", None, None)?;
        let config = axum_server::tls_rustls::RustlsConfig::from_pem(
            ident.cert_pem.into_bytes(),
            ident.key_pem.into_bytes(),
        )
        .await?;
        let maintenance = Arc::new(Mutex::new(maintenance));
        let diagnostic_log = maintenance.clone();
        let observer: photobridge_transport::Observer = Arc::new(move |event| {
            if let Ok(log) = diagnostic_log.lock() {
                let context = maintenance::EventContext {
                    request_id: Some(event.request_id),
                    observed_at_ms: Some(event.observed_at_ms),
                    duration_ms: Some(event.duration_ms),
                    bytes_received: Some(event.bytes_received),
                    first_body_at_ms: event.first_body_at_ms,
                    body_complete: Some(event.body_complete),
                    http_status: event.status,
                    ..Default::default()
                };
                let _ = log.log_context(event.event, None, None, Some(&context));
            }
        });
        let router = photobridge_transport::shared_router_observed(
            receiver.clone(),
            &ident.pairing.token,
            Some(observer),
        )?;
        let listener = std::net::TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        let handle = axum_server::Handle::new();
        let server = axum_server::from_tcp_rustls(listener, config).handle(handle.clone());
        let task = tokio::spawn(async move {
            let _ = server
                .serve(router.into_make_service_with_connect_info::<std::net::SocketAddr>())
                .await;
        });
        Ok(Self {
            root: root.into(),
            receiver,
            pairing: ident.pairing,
            maintenance,
            handle,
            task,
        })
    }
    pub fn healthy(&self) -> bool {
        !self.task.is_finished()
    }
}
impl Drop for ReceiverHost {
    fn drop(&mut self) {
        self.handle.shutdown();
        self.task.abort();
    }
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}

pub struct SenderHost {
    sender: Arc<Mutex<Sender>>,
    cancelled: Arc<AtomicBool>,
    busy: AtomicBool,
    request_root: PathBuf,
    export_root: PathBuf,
    maintenance: Mutex<maintenance::Maintenance>,
}
impl SenderHost {
    pub fn open(root: &Path) -> Result<Self> {
        let request_root = root.join("requests");
        fs::create_dir_all(&request_root)?;
        let maintenance = maintenance::Maintenance::open(root)?;
        maintenance.log("sender_started", None, None)?;
        let host = Self {
            export_root: root.parent().unwrap_or(root).join("exports"),
            maintenance: Mutex::new(maintenance),
            sender: Arc::new(Mutex::new(Sender::open(root)?)),
            cancelled: Arc::new(AtomicBool::new(false)),
            busy: AtomicBool::new(false),
            request_root,
        };
        host.restore_export_locations()?;
        Ok(host)
    }
    pub fn enqueue(
        &self,
        receiver: &str,
        asset: Asset,
        sources: BTreeMap<String, String>,
    ) -> Result<Job> {
        self.sender
            .lock()
            .map_err(lock)?
            .enqueue(receiver, asset, sources)
    }
    pub fn list(&self, after: i64) -> Result<Vec<Job>> {
        self.sender.lock().map_err(lock)?.list(after, 200)
    }
    pub fn pause(&self, paused: bool) -> Result<()> {
        self.cancelled.store(paused, Ordering::SeqCst);
        self.sender.lock().map_err(lock)?.set_paused(paused)
    }
    pub fn retry(&self, id: i64) -> Result<()> {
        self.sender.lock().map_err(lock)?.retry(id)
    }
    /// One queued asset per call. Native hosts schedule invocations; the Rust
    /// queue chooses due work and owns retry policy, receipt and progress.
    pub async fn run_once(&self, pairing: &Pairing) -> Result<Option<Job>> {
        if self.busy.swap(true, Ordering::SeqCst) {
            return Err(Error::Conflict("sender already running".into()));
        }
        struct Guard<'a>(&'a AtomicBool);
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _guard = Guard(&self.busy);
        // Construct runtime-bound clients inside Tokio, not on native UI threads.
        let client = pairing.client()?;
        let job = self
            .sender
            .lock()
            .map_err(lock)?
            .claim(&pairing.receiver_id, now())?;
        let Some(job) = job else {
            return Ok(None);
        };
        let attempt = job.attempt();
        let paths = job
            .sources
            .iter()
            .map(|(h, p)| (h.clone(), PathBuf::from(p)))
            .collect();
        let mut progress_error = None;
        let transfer = client.send(&job.asset, &paths, &self.cancelled, |status| {
            if let Err(e) = self
                .sender
                .lock()
                .map_err(lock)
                .and_then(|mut s| s.acknowledge(&attempt, status))
            {
                progress_error = Some(e);
                self.cancelled.store(true, Ordering::SeqCst);
            }
        });
        let result = tokio::select! {
            result = transfer => result,
            _ = async {
                loop { if self.cancelled.load(Ordering::SeqCst) {break;}
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            } => Err(Error::Cancelled),
        };
        let mut sender = self.sender.lock().map_err(lock)?;
        if let Some(error) = progress_error {
            let _ = sender.interrupted(&attempt);
            return Err(error);
        }
        match result {
            Ok(_) => {} // The final receiver receipt was persisted by progress().
            Err(Error::Cancelled) => sender.interrupted(&attempt)?,
            Err(e) => sender.fail(&attempt, Failure::from_error(&e), now())?,
        }
        let result = sender.job(job.id)?;
        drop(sender);
        self.record_result(&result);
        Ok(Some(result))
    }
}
fn lock<T>(_: std::sync::PoisonError<T>) -> Error {
    Error::Storage("host lock unavailable".into())
}

fn default_motion_video_mime() -> String {
    "video/mp4".into()
}
fn default_heic_motion_video_mime() -> String {
    "video/quicktime".into()
}

#[derive(Deserialize)]
struct ExportedResource {
    role: ResourceRole,
    filename: String,
    media_type: String,
    path: PathBuf,
}
fn manifest(
    source_id: String,
    revision: String,
    kind: AssetKind,
    metadata: BTreeMap<String, String>,
    resources: Vec<ExportedResource>,
) -> Result<(Asset, BTreeMap<String, String>)> {
    let mut out = Vec::new();
    let mut paths = BTreeMap::new();
    for r in resources {
        let f = File::open(&r.path)?;
        let size = f.metadata()?.len();
        let hash = digest_reader(f)?;
        paths.insert(
            hash.clone(),
            r.path
                .to_str()
                .ok_or_else(|| Error::Invalid("source path encoding".into()))?
                .into(),
        );
        out.push(Resource {
            role: r.role,
            filename: r.filename,
            media_type: r.media_type,
            size,
            sha256: hash,
        });
    }
    let asset = Asset {
        version: 1,
        source_id,
        revision,
        kind,
        metadata,
        resources: out,
    };
    asset.validate()?;
    Ok((asset, paths))
}
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Command {
    ReceiverConnectionInfo {
        root: PathBuf,
    },
    ReceiverTransferHold {
        held: bool,
    },
    PhotosCleanupStep {
        state: photobridge_pixel::photos_cleanup::State,
        snapshot: photobridge_pixel::photos_probe::Snapshot,
        expected: String,
    },
    PhotosProbe {
        snapshot: photobridge_pixel::photos_probe::Snapshot,
    },
    BurstMetadata {
        identifier: String,
        primary: bool,
    },
    PackageBurst {
        jpeg: PathBuf,
        output: PathBuf,
        metadata: BTreeMap<String, String>,
    },
    DeviceStatus {
        root: PathBuf,
        #[serde(default)]
        language: String,
        name: Option<String>,
    },
    SetSenderEnabled {
        root: PathBuf,
        id: String,
        enabled: bool,
    },
    ExchangeDevice {
        pairing: Pairing,
        device_type: Option<String>,
    },
    HistoryStatus {
        receiver_id: String,
    },
    HistoryControl {
        receiver_id: String,
        action: maintenance::HistoryAction,
    },
    HistoryBatch {
        receiver_id: String,
        run: i64,
        sources: Vec<(String, String)>,
        finished: bool,
    },
    ScheduleSources {
        receiver_id: String,
        sources: Vec<String>,
    },
    BrowseSources {
        receiver_id: String,
        history: bool,
        #[serde(default)]
        after: i64,
        #[serde(default)]
        descending: bool,
        #[serde(default)]
        sort: Option<String>,
        #[serde(default)]
        after_value: Option<i64>,
    },
    MissingBrowseDates {
        receiver_id: String,
        history: bool,
    },
    PendingSources {
        receiver_id: String,
    },
    SourceDates {
        receiver_id: String,
        dates: Vec<(String, i64)>,
    },
    SourceResult {
        receiver_id: String,
        source: String,
        complete: bool,
    },
    StorageStatus {
        receiver: bool,
    },
    SaveStorageSettings {
        receiver: bool,
        settings: maintenance::Settings,
    },
    ActivityLog {
        receiver: bool,
        after: Option<i64>,
    },
    RecordEvent {
        receiver: bool,
        root: Option<PathBuf>,
        code: String,
        #[serde(default)]
        job_id: Option<i64>,
        #[serde(default)]
        bytes: Option<u64>,
        #[serde(default)]
        context: Option<Box<maintenance::EventContext>>,
    },
    ReclaimSenderCache,
    RetireDeliveredCache {
        previous_receiver: String,
        current_receiver: String,
    },
    ArchiveBatch {
        root: Option<PathBuf>,
    },
    ReleaseArchived {
        root: Option<PathBuf>,
        ids: Vec<String>,
        verified: std::collections::BTreeSet<String>,
    },
    OpenSender {
        root: PathBuf,
    },
    Enqueue {
        receiver_id: String,
        source_id: String,
        revision: String,
        kind: AssetKind,
        #[serde(default)]
        metadata: BTreeMap<String, String>,
        resources: Vec<ExportedResource>,
    },
    PrepareNative {
        receiver_id: String,
    },
    BindNative {
        attempt: Attempt,
        task_id: String,
    },
    AbandonNative {
        attempt: Attempt,
    },
    FinishNative {
        attempt: Attempt,
        task_id: String,
        status_code: u16,
        #[serde(default)]
        body: String,
        failure: Option<Failure>,
        #[serde(default)]
        cancelled: bool,
    },
    ReconcileNative {
        live: BTreeSet<String>,
    },
    StartDesktopPairing {
        ip: std::net::IpAddr,
    },
    DesktopPairingStatus,
    StopDesktopPairing {
        session_id: String,
    },
    SubmitDesktopPairing {
        invite: pairing::DesktopInvite,
        pairing: Pairing,
    },
    SenderRevision,
    SourceStates {
        #[serde(default)]
        include_pending: bool,
        #[serde(default)]
        include_previous_receipts: bool,
        receiver_id: String,
        sources: Vec<(String, String)>,
    },
    SenderSummary {
        receiver_id: String,
    },
    SenderStatus,
    SetTransferConcurrency {
        limit: u32,
    },
    RecoverConnection {
        receiver_id: String,
    },
    RelocatePairing {
        pairing: Pairing,
        endpoint: String,
    },
    CheckPairing {
        pairing: Pairing,
    },
    RunSender {
        pairing: Pairing,
    },
    PauseSender {
        paused: bool,
    },
    Retry {
        id: i64,
    },
    Jobs {
        #[serde(default)]
        after: i64,
        receiver_id: Option<String>,
        state: Option<String>,
        #[serde(default)]
        descending: bool,
        #[serde(default)]
        sort: Option<String>,
        #[serde(default)]
        after_value: Option<i64>,
    },
    StartReceiver {
        root: PathBuf,
        listen: SocketAddr,
        capacity: u64,
    },
    WritePhotoDate {
        source: PathBuf,
        output: PathBuf,
        date: String,
        #[serde(default)]
        subsecond: u16,
    },
    PackageMotion {
        jpeg: PathBuf,
        mp4: PathBuf,
        output: PathBuf,
        #[serde(default)]
        metadata: BTreeMap<String, String>,
        #[serde(default = "default_motion_video_mime")]
        video_mime: String,
    },
    PackageHeicMotion {
        heic: PathBuf,
        mov: PathBuf,
        output: PathBuf,
        #[serde(default)]
        metadata: BTreeMap<String, String>,
        #[serde(default = "default_heic_motion_video_mime")]
        video_mime: String,
    },
    StopReceiver,
    ReceiverStatus,
    ReceiverOverview,
    ReceiverLogs {
        root: PathBuf,
    },
    GalleryEvidence {
        id: String,
    },
    PrepareGallery {
        id: String,
        copy: photobridge_store::retention::GalleryCopy,
    },
    GalleryCandidates {
        root: Option<PathBuf>,
        #[serde(default)]
        after: String,
    },
    GalleryPublication {
        root: Option<PathBuf>,
        id: String,
        copy: photobridge_store::retention::GalleryCopy,
    },
    ReleaseGallery {
        root: Option<PathBuf>,
        id: String,
        copy: photobridge_store::retention::GalleryCopy,
    },
    ReceiverStorageUsage {
        root: PathBuf,
    },
    ReceiverSettings {
        root: PathBuf,
        settings: Option<maintenance::Settings>,
    },
    ReceiverHistory {
        root: PathBuf,
        sender_id: Option<String>,
        before: Option<i64>,
        state: String,
        kind: String,
    },
    RetryProcessing,
    Publications {
        #[serde(default)]
        after: String,
    },
    Processed {
        id: String,
        success: bool,
    },
}
struct Hosts {
    sender: Option<Arc<SenderHost>>,
    receiver: Option<ReceiverHost>,
    desktop_pairing: Option<pairing::DesktopPairing>,
}
static HOSTS: Mutex<Hosts> = Mutex::new(Hosts {
    sender: None,
    receiver: None,
    desktop_pairing: None,
});
static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("native runtime")
    })
}
fn sender() -> Result<Arc<SenderHost>> {
    HOSTS
        .lock()
        .map_err(lock)?
        .sender
        .clone()
        .ok_or(Error::NotFound)
}
fn dispatch(command: Command) -> Result<Value> {
    match command {
        Command::ReceiverConnectionInfo { root } => receiver_connection_info(&root),
        Command::ReceiverTransferHold { held } => {
            let h = HOSTS.lock().map_err(lock)?;
            let host = h.receiver.as_ref().ok_or(Error::NotFound)?;
            host.receiver.lock().map_err(lock)?.hold_transfers(held);
            Ok(json!({}))
        }
        Command::PhotosCleanupStep {
            state,
            snapshot,
            expected,
        } => Ok(serde_json::to_value(
            photobridge_pixel::photos_cleanup::step(state, &snapshot, &expected),
        )?),
        Command::PhotosProbe { snapshot } => Ok(serde_json::to_value(
            photobridge_pixel::photos_probe::classify(&snapshot),
        )?),
        Command::BurstMetadata {
            identifier,
            primary,
        } => Ok(serde_json::to_value(
            BurstMetadata::from_identifier(&identifier, primary)?.fields(),
        )?),
        Command::WritePhotoDate {
            source,
            output,
            date,
            subsecond,
        } => {
            photobridge_pixel::write_photo_date(&source, &output, &date, subsecond)?;
            Ok(json!({}))
        }
        Command::PackageBurst {
            jpeg,
            output,
            metadata,
        } => {
            let burst = BurstMetadata::from_fields(&metadata)?
                .ok_or_else(|| Error::Invalid("missing burst metadata".into()))?;
            photobridge_pixel::write_jpeg_burst(&jpeg, &output, &burst)?;
            Ok(json!({}))
        }
        Command::DeviceStatus {
            root,
            language,
            name,
        } => {
            let devices = photobridge_store::devices::DeviceDirectory::open(&root, &language)?;
            if let Some(name) = name {
                devices.rename(&name)?;
            }
            Ok(json!({"device":devices.profile()?,"peers":devices.peers()?}))
        }
        Command::SetSenderEnabled { root, id, enabled } => {
            photobridge_store::devices::DeviceDirectory::open(&root, "en")?
                .set_enabled(&id, enabled)?;
            Ok(json!({}))
        }
        Command::ExchangeDevice {
            pairing,
            device_type,
        } => {
            let sender = sender()?;
            let root = sender.request_root.parent().ok_or(Error::NotFound)?;
            let devices = photobridge_store::devices::DeviceDirectory::open(root, "en")?;
            let profile = devices.profile()?;
            let (peer, caps) = runtime().block_on(async {
                let client = pairing.client()?;
                let caps = client.capabilities().await?;
                let peer = client
                    .exchange_device_profile_with_type(&profile, device_type.as_deref())
                    .await?;
                Ok::<_, Error>((peer, caps))
            })?;
            sender.set_bundle_upload(
                &pairing.receiver_id,
                caps.version == PROTOCOL_VERSION && caps.bundle_upload,
            )?;
            if let Some(peer) = peer.as_ref() {
                devices.remember(&pairing.receiver_id, peer)?;
            }
            Ok(serde_json::to_value(peer)?)
        }
        Command::HistoryStatus { receiver_id } => Ok(serde_json::to_value(
            sender()?
                .maintenance
                .lock()
                .map_err(lock)?
                .history_status(&receiver_id)?,
        )?),
        Command::HistoryControl {
            receiver_id,
            action,
        } => Ok(serde_json::to_value(
            sender()?
                .maintenance
                .lock()
                .map_err(lock)?
                .history_control(&receiver_id, action)?,
        )?),
        Command::HistoryBatch {
            receiver_id,
            run,
            sources,
            finished,
        } => {
            let host = sender()?;
            let known = host
                .sender
                .lock()
                .map_err(lock)?
                .source_states(&receiver_id, &sources)?;
            let status = host.maintenance.lock().map_err(lock)?.history_batch(
                &receiver_id,
                run,
                &sources,
                &known,
                finished,
            )?;
            Ok(serde_json::to_value(status)?)
        }
        Command::ScheduleSources {
            receiver_id,
            sources,
        } => {
            sender()?
                .maintenance
                .lock()
                .map_err(lock)?
                .schedule_sources(&receiver_id, &sources)?;
            Ok(json!({}))
        }
        Command::MissingBrowseDates {
            receiver_id,
            history,
        } => Ok(json!(sender()?
            .maintenance
            .lock()
            .map_err(lock)?
            .missing_browse_dates(&receiver_id, history)?)),
        Command::BrowseSources {
            receiver_id,
            history,
            after,
            descending,
            sort,
            after_value,
        } => {
            let host = sender()?;
            let maintenance = host.maintenance.lock().map_err(lock)?;
            let mut page = if let Some(sort) = sort {
                maintenance.source_page_sorted(
                    &receiver_id,
                    history,
                    after,
                    after_value,
                    &sort,
                    descending,
                )?
            } else {
                maintenance.source_page_ordered(&receiver_id, history, after, descending)?
            };
            drop(maintenance);
            if history {
                let items = page["items"]
                    .as_array_mut()
                    .ok_or_else(|| Error::Invalid("source page".into()))?;
                let sources = items
                    .iter()
                    .map(|r| {
                        (
                            r["source"].as_str().unwrap_or_default().to_owned(),
                            r["revision"].as_str().unwrap_or_default().to_owned(),
                        )
                    })
                    .collect::<Vec<_>>();
                let states = host
                    .sender
                    .lock()
                    .map_err(lock)?
                    .source_states(&receiver_id, &sources)?;
                for row in items {
                    row["state"] = json!(states
                        .get(row["source"].as_str().unwrap_or_default())
                        .map(String::as_str)
                        .unwrap_or(if row["retry_at"].is_null() {
                            "scanned"
                        } else {
                            "preparing"
                        }));
                }
            }
            Ok(page)
        }
        Command::PendingSources { receiver_id } => sender()?
            .maintenance
            .lock()
            .map_err(lock)?
            .pending(&receiver_id),
        Command::SourceDates { receiver_id, dates } => {
            sender()?
                .maintenance
                .lock()
                .map_err(lock)?
                .source_dates(&receiver_id, &dates)?;
            Ok(json!({}))
        }
        Command::SourceResult {
            receiver_id,
            source,
            complete,
        } => {
            sender()?.maintenance.lock().map_err(lock)?.source_result(
                &receiver_id,
                &source,
                complete,
            )?;
            Ok(json!({}))
        }
        Command::ArchiveBatch { root } => {
            receiver_storage::with_store(root.as_deref(), |store, _| {
                Ok(serde_json::to_value(store.archive_batch()?)?)
            })
        }
        Command::ReleaseArchived {
            root,
            ids,
            verified,
        } => receiver_storage::with_store(root.as_deref(), |store, maintenance| {
            let bytes = store.release_archived(&ids, &verified)?;
            maintenance.log("originals_reclaimed", None, Some(bytes))?;
            Ok(json!({"bytes":bytes}))
        }),
        Command::StorageStatus { receiver: false } => sender()?.storage_status(),
        Command::StorageStatus { receiver: true } => {
            let hosts = HOSTS.lock().map_err(lock)?;
            let host = hosts.receiver.as_ref().ok_or(Error::NotFound)?;
            let mut status = host.receiver.lock().map_err(lock)?.overview()?;
            status["settings"] =
                serde_json::to_value(&host.maintenance.lock().map_err(lock)?.settings)?;
            Ok(status)
        }
        Command::SaveStorageSettings {
            receiver: false,
            settings,
        } => {
            sender()?.maintenance.lock().map_err(lock)?.save(settings)?;
            Ok(json!({}))
        }
        Command::SaveStorageSettings {
            receiver: true,
            settings,
        } => {
            let hosts = HOSTS.lock().map_err(lock)?;
            let host = hosts.receiver.as_ref().ok_or(Error::NotFound)?;
            host.maintenance
                .lock()
                .map_err(lock)?
                .save(settings.clone())?;
            host.receiver
                .lock()
                .map_err(lock)?
                .configure_storage(settings.receiver_budget_bytes, settings.min_free_bytes)?;
            Ok(json!({}))
        }
        Command::ActivityLog { receiver, after } => {
            if receiver {
                let h = HOSTS.lock().map_err(lock)?;
                let value = h
                    .receiver
                    .as_ref()
                    .ok_or(Error::NotFound)?
                    .maintenance
                    .lock()
                    .map_err(lock)?
                    .event_update(after)?;
                Ok(value)
            } else {
                sender()?
                    .maintenance
                    .lock()
                    .map_err(lock)?
                    .event_update(after)
            }
        }
        Command::RecordEvent {
            receiver,
            root,
            code,
            job_id,
            bytes,
            context,
        } => {
            if receiver {
                receiver_storage::with_maintenance(root.as_deref(), |maintenance| {
                    maintenance.log_context(&code, job_id, bytes, context.as_deref())
                })?;
            } else {
                sender()?.maintenance.lock().map_err(lock)?.log_context(
                    &code,
                    job_id,
                    bytes,
                    context.as_deref(),
                )?;
            }
            Ok(json!({}))
        }
        Command::RetireDeliveredCache {
            previous_receiver,
            current_receiver,
        } => sender()?.retire_delivered_cache(&previous_receiver, &current_receiver),
        Command::ReclaimSenderCache => Ok(json!({"reclaimed_bytes":sender()?.reclaim_received()?})),
        Command::OpenSender { root } => {
            let mut hosts = HOSTS.lock().map_err(lock)?;
            if hosts.sender.is_some() {
                return Err(Error::Conflict("sender already open".into()));
            }
            hosts.sender = Some(Arc::new(SenderHost::open(&root)?));
            Ok(json!({}))
        }
        Command::Enqueue {
            receiver_id,
            source_id,
            revision,
            kind,
            metadata,
            resources,
        } => {
            let (a, p) = manifest(source_id, revision, kind, metadata, resources)?;
            let host = sender()?;
            let job = host.enqueue(&receiver_id, a, p)?;
            host.maintenance
                .lock()
                .map_err(lock)?
                .log("transfer_queued", Some(job.id), None)?;
            Ok(serde_json::to_value(job)?)
        }
        Command::PrepareNative { receiver_id } => Ok(serde_json::to_value(
            sender()?.prepare_native(&receiver_id)?,
        )?),
        Command::BindNative { attempt, task_id } => {
            sender()?.bind_native(&attempt, &task_id)?;
            Ok(json!({}))
        }
        Command::AbandonNative { attempt } => {
            sender()?.abandon_native(&attempt)?;
            Ok(json!({}))
        }
        Command::FinishNative {
            attempt,
            task_id,
            status_code,
            body,
            failure,
            cancelled,
        } => Ok(serde_json::to_value(sender()?.finish_native(
            &attempt,
            &task_id,
            status_code,
            &body,
            failure,
            cancelled,
        )?)?),
        Command::ReconcileNative { live } => {
            Ok(serde_json::to_value(sender()?.reconcile_native(&live)?)?)
        }
        Command::StartDesktopPairing { ip } => {
            let host = runtime().block_on(pairing::DesktopPairing::start(ip))?;
            let invite = serde_json::to_value(&host.invite)?;
            HOSTS.lock().map_err(lock)?.desktop_pairing = Some(host);
            Ok(invite)
        }
        Command::DesktopPairingStatus => HOSTS
            .lock()
            .map_err(lock)?
            .desktop_pairing
            .as_ref()
            .ok_or_else(|| Error::Invalid("pairing closed".into()))?
            .status(),
        Command::StopDesktopPairing { session_id } => {
            let mut hosts = HOSTS.lock().map_err(lock)?;
            if hosts
                .desktop_pairing
                .as_ref()
                .is_some_and(|p| p.invite.connection.receiver_id == session_id)
            {
                hosts.desktop_pairing = None;
            }
            Ok(json!({}))
        }
        Command::SubmitDesktopPairing { invite, pairing } => {
            runtime().block_on(pairing::submit(invite, pairing))?;
            Ok(json!({}))
        }
        Command::SenderRevision => Ok(json!(sender()?.sender.lock().map_err(lock)?.revision()?)),
        Command::SourceStates {
            receiver_id,
            sources,
            include_pending,
            include_previous_receipts,
        } => {
            let host = sender()?;
            let mut states = host
                .sender
                .lock()
                .map_err(lock)?
                .source_states(&receiver_id, &sources)?;
            if include_pending {
                let pending = host
                    .maintenance
                    .lock()
                    .map_err(lock)?
                    .pending_states(&receiver_id, &sources)?;
                for (id, state) in pending {
                    states.entry(id).or_insert(state);
                }
            }
            if include_previous_receipts {
                let previous = host
                    .sender
                    .lock()
                    .map_err(lock)?
                    .previous_receipts(&receiver_id, &sources)?;
                for (id, state) in previous {
                    states.entry(id).or_insert(state);
                }
            }
            Ok(serde_json::to_value(states)?)
        }
        Command::SenderSummary { receiver_id } => {
            sender()?.sender.lock().map_err(lock)?.summary(&receiver_id)
        }
        Command::SenderStatus => {
            let host = sender()?;
            let sender = host.sender.lock().map_err(lock)?;
            Ok(
                json!({"paused":sender.paused()?, "concurrent_uploads":sender.concurrent_uploads()?}),
            )
        }
        Command::SetTransferConcurrency { limit } => {
            sender()?
                .sender
                .lock()
                .map_err(lock)?
                .set_concurrent_uploads(limit)?;
            Ok(json!({}))
        }
        Command::RecoverConnection { receiver_id } => Ok(
            json!({"requeued": sender()?.sender.lock().map_err(lock)?.recover_connection(&receiver_id)?}),
        ),
        Command::RelocatePairing { pairing, endpoint } => {
            let updated = relocated_pairing(&pairing, &endpoint)?;
            runtime().block_on(async {
                let caps = tokio::time::timeout(
                    std::time::Duration::from_secs(8),
                    updated.client()?.capabilities(),
                )
                .await
                .map_err(|_| Error::Transport("receiver discovery timeout".into()))??;
                if caps.version != PROTOCOL_VERSION {
                    return Err(Error::Unsupported("protocol version".into()));
                }
                Ok::<_, Error>(())
            })?;
            Ok(serde_json::to_value(updated)?)
        }
        Command::CheckPairing { pairing } => {
            let caps = runtime().block_on(async {
                tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    pairing.client()?.capabilities(),
                )
                .await
                .map_err(|_| Error::Transport("receiver probe timeout".into()))?
            })?;
            if caps.version != PROTOCOL_VERSION {
                return Err(Error::Unsupported("protocol version".into()));
            }
            if let Ok(host) = sender() {
                host.set_bundle_upload(&pairing.receiver_id, caps.bundle_upload)?;
            }
            Ok(json!({"receiver_id":pairing.receiver_id}))
        }
        Command::RunSender { pairing } => Ok(serde_json::to_value(
            runtime().block_on(sender()?.run_once(&pairing))?,
        )?),
        Command::PauseSender { paused } => {
            sender()?.pause(paused)?;
            sender()?.maintenance.lock().map_err(lock)?.log(
                if paused {
                    "backup_paused"
                } else {
                    "backup_resumed"
                },
                None,
                None,
            )?;
            Ok(json!({}))
        }
        Command::Retry { id } => {
            sender()?.retry(id)?;
            Ok(json!({}))
        }
        Command::Jobs {
            after,
            receiver_id,
            state,
            descending,
            sort,
            after_value,
        } => {
            let host = sender()?;
            let sender = host.sender.lock().map_err(lock)?;
            let jobs = if let Some(sort) = sort {
                sender.browse(JobQuery {
                    after,
                    after_value,
                    limit: 200,
                    receiver: receiver_id.as_deref(),
                    state: state.as_deref(),
                    sort: &sort,
                    descending,
                })?
            } else {
                sender.list_filtered_ordered(
                    after,
                    200,
                    receiver_id.as_deref(),
                    state.as_deref(),
                    descending,
                )?
            };
            Ok(serde_json::to_value(jobs)?)
        }
        Command::StartReceiver {
            root,
            listen,
            capacity,
        } => {
            let mut h = HOSTS.lock().map_err(lock)?;
            if h.receiver.is_some() {
                return Err(Error::Conflict("receiver already running".into()));
            }
            let r = runtime().block_on(ReceiverHost::start(&root, listen, capacity))?;
            let p = serde_json::to_value(&r.pairing)?;
            h.receiver = Some(r);
            Ok(p)
        }
        Command::PackageMotion {
            jpeg,
            mp4,
            output,
            metadata,
            video_mime,
        } => {
            photobridge_pixel::write_jpeg_motion_with_burst_and_video_mime(
                &jpeg,
                &mp4,
                &output,
                None,
                BurstMetadata::from_fields(&metadata)?.as_ref(),
                &video_mime,
            )?;
            Ok(json!({}))
        }
        Command::PackageHeicMotion {
            heic,
            mov,
            output,
            metadata,
            video_mime,
        } => {
            photobridge_pixel::write_heic_motion_with_burst_and_video_mime(
                &heic,
                &mov,
                &output,
                BurstMetadata::from_fields(&metadata)?.as_ref(),
                &video_mime,
            )?;
            Ok(json!({}))
        }
        Command::StopReceiver => {
            HOSTS.lock().map_err(lock)?.receiver = None;
            Ok(json!({}))
        }
        Command::RetryProcessing => {
            let h = HOSTS.lock().map_err(lock)?;
            let host = h.receiver.as_ref().ok_or(Error::NotFound)?;
            let count = host.receiver.lock().map_err(lock)?.retry_processing()?;
            Ok(json!({"count":count}))
        }
        Command::ReceiverLogs { root } => {
            let hosts = HOSTS.lock().map_err(lock)?;
            let value = if let Some(host) = hosts.receiver.as_ref().filter(|h| h.root == root) {
                host.maintenance.lock().map_err(lock)?.events()?
            } else {
                maintenance::Maintenance::open(&root)?.events()?
            };
            Ok(value)
        }
        Command::GalleryEvidence { id } => receiver_storage::with_store(None, |store, _| {
            let confirmed = store.gallery_copy(&id)?;
            let copy = if confirmed.is_some() {
                confirmed.clone()
            } else {
                store.expected_gallery_copy(&id)?
            };
            Ok(json!({"copy":copy,"confirmed":confirmed.is_some()}))
        }),
        Command::PrepareGallery { id, copy } => receiver_storage::with_store(None, |store, _| {
            store.prepare_gallery_copy(&id, &copy)?;
            Ok(json!({}))
        }),
        Command::GalleryCandidates { root, after } => {
            receiver_storage::with_store(root.as_deref(), |store, maintenance| {
                if !maintenance.settings.receiver_relay {
                    return Ok(json!([]));
                }
                Ok(serde_json::to_value(store.gallery_candidates(&after)?)?)
            })
        }
        Command::GalleryPublication { root, id, copy } => {
            receiver_storage::with_store(root.as_deref(), |store, maintenance| {
                store.record_gallery_copy(&id, &copy)?;
                maintenance.log("publication_complete", None, None)?;
                let bytes = if maintenance.settings.receiver_relay {
                    store.release_gallery_copy(&id, &copy, true)?
                } else {
                    0
                };
                if maintenance.settings.receiver_relay {
                    maintenance.log("relay_originals_reclaimed", None, Some(bytes))?;
                }
                Ok(json!({"bytes":bytes}))
            })
        }
        Command::ReleaseGallery { root, id, copy } => {
            receiver_storage::with_store(root.as_deref(), |store, maintenance| {
                let bytes =
                    store.release_gallery_copy(&id, &copy, maintenance.settings.receiver_relay)?;
                maintenance.log("relay_originals_reclaimed", None, Some(bytes))?;
                Ok(json!({"bytes":bytes}))
            })
        }
        Command::ReceiverStorageUsage { root } => Ok(serde_json::to_value(
            photobridge_store::usage::original_usage(&root.join("store"))?,
        )?),
        Command::ReceiverSettings { root, settings } => {
            let was_saved = settings.is_some();
            let hosts = HOSTS.lock().map_err(lock)?;
            let current = if let Some(host) = hosts.receiver.as_ref().filter(|h| h.root == root) {
                let mut maintenance = host.maintenance.lock().map_err(lock)?;
                if let Some(settings) = settings {
                    maintenance.save(settings.clone())?;
                    host.receiver.lock().map_err(lock)?.configure_storage(
                        settings.receiver_budget_bytes,
                        settings.min_free_bytes,
                    )?;
                }
                maintenance.settings.clone()
            } else {
                let mut maintenance = maintenance::Maintenance::open(&root)?;
                if let Some(settings) = settings {
                    maintenance.save(settings)?;
                }
                maintenance.settings.clone()
            };
            if was_saved && !root.join("maintenance.initialized").exists() {
                private_write(&root.join("maintenance.initialized"), b"1")?;
            }
            let catalog = photobridge_store::catalog::Catalog::open(&root.join("store"))?;
            Ok(
                json!({"settings":current,"used_bytes":catalog.reserved_bytes()?,"counts":catalog.counts()?,"free_bytes":fs2::available_space(root)?}),
            )
        }
        Command::ReceiverHistory {
            root,
            sender_id,
            before,
            state,
            kind,
        } => {
            let catalog = photobridge_store::catalog::Catalog::open(&root.join("store"))?;
            Ok(serde_json::to_value(catalog.page_for_sender(
                before,
                &state,
                &kind,
                100,
                sender_id.as_deref(),
            )?)?)
        }
        Command::ReceiverOverview => {
            let h = HOSTS.lock().map_err(lock)?;
            let result = h
                .receiver
                .as_ref()
                .ok_or(Error::NotFound)?
                .receiver
                .lock()
                .map_err(lock)?
                .overview();
            result
        }
        Command::ReceiverStatus => {
            let h = HOSTS.lock().map_err(lock)?;
            Ok(json!({"running":h.receiver.as_ref().is_some_and(|r|r.healthy())}))
        }
        Command::Publications { after } => {
            let h = HOSTS.lock().map_err(lock)?;
            let host = h.receiver.as_ref().ok_or(Error::NotFound)?;
            let r = host.receiver.lock().map_err(lock)?;
            let items = r.publications(&after, 50)?;
            Ok(serde_json::to_value(items)?)
        }
        Command::Processed { id, success } => {
            let h = HOSTS.lock().map_err(lock)?;
            let host = h.receiver.as_ref().ok_or(Error::NotFound)?;
            let mut r = host.receiver.lock().map_err(lock)?;
            let status = r.status(&id)?;
            host.maintenance.lock().map_err(lock)?.log(
                if success {
                    "publication_complete"
                } else {
                    "publication_failed"
                },
                None,
                None,
            )?;
            if status.processing != ProcessingState::Complete {
                r.set_processing(&id, ProcessingState::Pending)?;
                r.set_processing(
                    &id,
                    if success {
                        ProcessingState::Complete
                    } else {
                        ProcessingState::Failed
                    },
                )?;
            }
            Ok(json!({}))
        }
    }
}
fn error_code(e: Error) -> &'static str {
    match e {
        Error::Invalid(_) => "invalid_input",
        Error::Conflict(_) => "conflict",
        Error::NotFound => "not_found",
        Error::Capacity => "capacity",
        Error::LowSpace => "low_space",
        Error::Integrity => "integrity",
        Error::Storage(_) => "storage",
        Error::Transport(_) => "network",
        Error::Unauthorized => "authentication",
        Error::Cancelled => "cancelled",
        Error::Unsupported(_) => "unsupported",
    }
}

/// Local FFI callers receive stable, non-sensitive errors. Do not log requests:
/// pairing commands contain the bearer credential.
pub fn call(request: &str) -> String {
    let result = std::panic::catch_unwind(|| -> Result<Value> {
        if request.len() > 1024 * 1024 {
            return Err(Error::Invalid("request size".into()));
        }
        dispatch(serde_json::from_str(request)?)
    });
    match result {
        Ok(Ok(value)) => json!({"ok":true,"value":value}).to_string(),
        Ok(Err(e)) => {
            let code = error_code(e);
            json!({"ok":false,"error":code}).to_string()
        }
        Err(_) => json!({"ok":false,"error":"internal"}).to_string(),
    }
}
/// # Safety
/// `request` must point to a valid, NUL-terminated UTF-8 string for this call.
/// Free the returned allocation exactly once with `photobridge_free`.
#[no_mangle]
pub unsafe extern "C" fn photobridge_call(request: *const c_char) -> *mut c_char {
    let response = if request.is_null() {
        json!({"ok":false,"error":"invalid_input"}).to_string()
    } else {
        match unsafe { CStr::from_ptr(request) }.to_str() {
            Ok(s) => call(s),
            Err(_) => json!({"ok":false,"error":"invalid_input"}).to_string(),
        }
    };
    CString::new(response)
        .expect("JSON contains no raw NUL")
        .into_raw()
}
/// # Safety
/// Pass only a non-null allocation returned by `photobridge_call`, once.
#[no_mangle]
pub unsafe extern "C" fn photobridge_free(value: *mut c_char) {
    if !value.is_null() {
        drop(unsafe { CString::from_raw(value) });
    }
}
#[cfg(target_os = "android")]
#[no_mangle]
pub extern "system" fn Java_app_photobridge_NativeBridge_call(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    request: jni::objects::JString,
) -> jni::sys::jstring {
    let response = match env.get_string(&request) {
        Ok(s) => call(&s.to_string_lossy()),
        Err(_) => return std::ptr::null_mut(),
    };
    env.new_string(response)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

#[cfg(test)]
mod receiver_identity_tests;
