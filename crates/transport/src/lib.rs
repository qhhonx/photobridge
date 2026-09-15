//! Shared HTTP protocol host and reference Rust client. Native background upload
//! engines can use the same wire contract and core::next_action instead.
mod bundle;
mod diagnostics;
pub use diagnostics::{Observer, RequestObservation};
mod pinned_tls;
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Json, Router,
};
use photobridge_core::*;
use photobridge_store::Receiver;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::Semaphore;

#[derive(Clone)]
struct ServerState {
    receiver: Arc<Mutex<Receiver>>,
    token_digest: String,
    sender_id: Option<String>,
    permits: Arc<Semaphore>,
    observer: Option<Observer>,
}
#[derive(Serialize, Deserialize)]
struct Problem {
    code: String,
    message: String,
}
struct ApiError(Error);
impl From<Error> for ApiError {
    fn from(value: Error) -> Self {
        Self(value)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code) = match self.0 {
            Error::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid"),
            Error::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            Error::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Error::Capacity => (StatusCode::INSUFFICIENT_STORAGE, "capacity"),
            Error::LowSpace => (StatusCode::INSUFFICIENT_STORAGE, "low_space"),
            Error::Integrity => (StatusCode::UNPROCESSABLE_ENTITY, "integrity"),
            Error::Unsupported(_) => (StatusCode::UNPROCESSABLE_ENTITY, "unsupported"),
            Error::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        };
        // Never return local filesystem paths, SQL details or bearer credentials.
        (
            status,
            [("x-photobridge-reason", code)],
            Json(Problem {
                code: code.into(),
                message: code.into(),
            }),
        )
            .into_response()
    }
}
fn validate_token(token: &str) -> Result<()> {
    if !valid_digest(token) {
        return Err(Error::Invalid(
            "token must be 32 random bytes encoded as lowercase hex".into(),
        ));
    }
    Ok(())
}
pub fn router(receiver: Receiver, token: &str) -> Result<Router> {
    shared_router(Arc::new(Mutex::new(receiver)), token)
}
pub fn shared_router(receiver: Arc<Mutex<Receiver>>, token: &str) -> Result<Router> {
    shared_router_observed(receiver, token, None)
}
pub fn shared_router_observed(
    receiver: Arc<Mutex<Receiver>>,
    token: &str,
    observer: Option<Observer>,
) -> Result<Router> {
    validate_token(token)?;
    let state = ServerState {
        receiver,
        token_digest: digest(token.as_bytes()),
        sender_id: None,
        permits: Arc::new(Semaphore::new(8)),
        observer,
    };
    Ok(Router::new()
        .route("/v1/capabilities", get(capabilities))
        .route("/v1/device-profile", post(exchange_device_profile))
        .route("/v1/assets", post(register))
        .route("/v1/bundles", post(bundle::receive))
        .route("/v1/assets/{id}", get(status))
        .route("/v1/assets/{id}/commit", post(commit))
        .route("/v1/assets/{id}/resources/{hash}", put(upload))
        .layer(DefaultBodyLimit::max(MAX_CHUNK_BYTES))
        .layer(middleware::from_fn_with_state(state.clone(), authorize))
        .with_state(state))
}
async fn authorize(
    State(mut state): State<ServerState>,
    mut request: Request,
    next: Next,
) -> Response {
    let supplied = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .unwrap_or("");
    let actual = digest(supplied.as_bytes());
    let difference = actual
        .bytes()
        .zip(state.token_digest.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b));
    if difference != 0 {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if let Some(id) = request
        .headers()
        .get("x-photobridge-sender")
        .and_then(|h| h.to_str().ok())
    {
        if !valid_digest(id) {
            return StatusCode::BAD_REQUEST.into_response();
        }
        let id = id.to_string();
        let kind = request
            .headers()
            .get("x-photobridge-device-type")
            .and_then(|h| h.to_str().ok())
            .map(str::to_owned);
        let ip = request
            .extensions()
            .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
            .map(|c| c.0.ip());
        let result = state
            .receiver
            .lock()
            .map_err(|_| Error::Storage("receiver lock".into()))
            .and_then(|r| {
                r.observe_sender(&id, ip, kind.as_deref())?;
                if request.uri().path() != "/v1/device-profile"
                    && request.uri().path() != "/v1/capabilities"
                {
                    r.check_sender(&id)?;
                }
                Ok(())
            });
        if let Err(error) = result {
            return ApiError(error).into_response();
        }
        state.sender_id = Some(id);
    }
    request.extensions_mut().insert(state.clone());
    let Ok(_permit) = state.permits.try_acquire() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    if state.observer.is_some() {
        return diagnostics::observe(request, next, state.observer.clone()).await;
    }
    // Streaming bundles enforce a bounded idle timeout and declared lengths in
    // their reader. A total 60-second limit would abort healthy large uploads.
    if request.uri().path() == "/v1/bundles" {
        return next.run(request).await;
    }
    match tokio::time::timeout(Duration::from_secs(60), next.run(request)).await {
        Ok(response) => response,
        Err(_) => StatusCode::REQUEST_TIMEOUT.into_response(),
    }
}
async fn with_receiver<T: Send + 'static>(
    state: ServerState,
    f: impl FnOnce(&mut Receiver) -> Result<T> + Send + 'static,
) -> std::result::Result<Json<T>, ApiError> {
    tokio::task::spawn_blocking(move || {
        let mut receiver = state
            .receiver
            .lock()
            .map_err(|_| Error::Storage("receiver lock poisoned".into()))?;
        if let Some(id) = &state.sender_id {
            receiver.check_sender(id)?;
        }
        f(&mut receiver).map(Json)
    })
    .await
    .map_err(|_| ApiError(Error::Storage("worker unavailable".into())))?
    .map_err(ApiError)
}
async fn capabilities() -> Json<Capabilities> {
    Json(Capabilities::default())
}
async fn exchange_device_profile(
    State(s): State<ServerState>,
    body: Bytes,
) -> std::result::Result<Json<DeviceProfile>, ApiError> {
    if body.len() > 2048 {
        return Err(Error::Invalid("device profile size".into()).into());
    }
    let profile: DeviceProfile = serde_json::from_slice(&body).map_err(Error::from)?;
    profile.validate()?;
    with_receiver(s, move |r| r.exchange_device_profile(&profile)).await
}
async fn register(
    axum::Extension(s): axum::Extension<ServerState>,
    body: Bytes,
) -> std::result::Result<Json<AssetStatus>, ApiError> {
    if body.len() > MAX_MANIFEST_BYTES {
        return Err(Error::Invalid("manifest size".into()).into());
    }
    let asset: Asset = serde_json::from_slice(&body).map_err(Error::from)?;
    let sender = s.sender_id.clone();
    with_receiver(s, move |r| {
        let status = r.register(asset)?;
        if let Some(id) = sender {
            r.attribute_sender(&status.asset_id, &id)?;
        }
        Ok(status)
    })
    .await
}
async fn status(
    axum::Extension(s): axum::Extension<ServerState>,
    Path(id): Path<String>,
) -> std::result::Result<Json<AssetStatus>, ApiError> {
    with_receiver(s, move |r| r.status(&id)).await
}
async fn commit(
    axum::Extension(s): axum::Extension<ServerState>,
    Path(id): Path<String>,
) -> std::result::Result<Json<AssetStatus>, ApiError> {
    with_receiver(s, move |r| r.commit(&id)).await
}
#[derive(Deserialize)]
struct ChunkQuery {
    offset: u64,
    sha256: String,
}
async fn upload(
    axum::Extension(s): axum::Extension<ServerState>,
    Path((id, hash)): Path<(String, String)>,
    Query(q): Query<ChunkQuery>,
    body: Bytes,
) -> std::result::Result<Json<AssetStatus>, ApiError> {
    with_receiver(s, move |r| r.append(&id, &hash, q.offset, &body, &q.sha256)).await
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: String,
    token: String,
}
impl Client {
    pub fn new(base: &str, token: &str) -> Result<Self> {
        Self::with_certificate(base, token, None)
    }
    pub fn with_certificate(
        base: &str,
        token: &str,
        certificate_der: Option<&[u8]>,
    ) -> Result<Self> {
        Self::with_certificate_name(base, token, certificate_der, None)
    }
    /// The certificate authority name is independent of a receiver's current
    /// network route. It must come from saved pairing, never discovery metadata.
    pub fn with_certificate_name(
        base: &str,
        token: &str,
        certificate_der: Option<&[u8]>,
        certificate_name: Option<&str>,
    ) -> Result<Self> {
        validate_token(token)?;
        let url = reqwest::Url::parse(base).map_err(|e| Error::Invalid(e.to_string()))?;
        let local = matches!(
            url.host_str(),
            Some("127.0.0.1" | "localhost" | "[::1]" | "::1")
        );
        if !(url.scheme() == "https" || url.scheme() == "http" && local)
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(Error::Invalid(
                "use an HTTPS origin (HTTP is only allowed on loopback)".into(),
            ));
        }
        let mut builder = reqwest::Client::builder();
        if let Some(der) = certificate_der {
            let host = url
                .host_str()
                .ok_or_else(|| Error::Invalid("pairing host".into()))?;
            builder = builder
                .use_preconfigured_tls(pinned_tls::configuration(
                    der,
                    certificate_name.unwrap_or(host).trim_matches(['[', ']']),
                )?)
                .no_proxy();
        } else if certificate_name.is_some() {
            return Err(Error::Invalid(
                "certificate name requires pinned certificate".into(),
            ));
        }
        let http = builder
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(network)?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').into(),
            token: token.into(),
        })
    }
    async fn decode<T: serde::de::DeserializeOwned>(response: reqwest::Response) -> Result<T> {
        let status = response.status();
        if !status.is_success() {
            return Err(match status.as_u16() {
                401 | 403 => Error::Unauthorized,
                404 => Error::NotFound,
                409 => Error::Conflict("receiver state changed; query status before retry".into()),
                507 if response
                    .headers()
                    .get("x-photobridge-reason")
                    .is_some_and(|v| v == "low_space") =>
                {
                    Error::LowSpace
                }
                507 => Error::Capacity,
                422 => Error::Integrity,
                _ => Error::Transport(format!("receiver returned HTTP {}", status.as_u16())),
            });
        }
        response.json().await.map_err(network)
    }
    pub async fn capabilities(&self) -> Result<Capabilities> {
        Self::decode(
            self.http
                .get(format!("{}/v1/capabilities", self.base))
                .bearer_auth(&self.token)
                .send()
                .await
                .map_err(network)?,
        )
        .await
    }
    /// Optional display metadata, exchanged only over the authenticated channel.
    /// Legacy receivers return 404; this never changes the transfer protocol.
    pub async fn exchange_device_profile(
        &self,
        profile: &DeviceProfile,
    ) -> Result<Option<DeviceProfile>> {
        self.exchange_device_profile_with_type(profile, None).await
    }
    pub async fn exchange_device_profile_with_type(
        &self,
        profile: &DeviceProfile,
        device_type: Option<&str>,
    ) -> Result<Option<DeviceProfile>> {
        profile.validate()?;
        let mut request = self
            .http
            .post(format!("{}/v1/device-profile", self.base))
            .bearer_auth(&self.token)
            .timeout(Duration::from_secs(5))
            .json(profile)
            .header("x-photobridge-sender", &profile.id);
        if let Some(kind) = device_type {
            request = request.header("x-photobridge-device-type", kind);
        }
        let mut response = request.send().await.map_err(network)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Self::decode(response).await;
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(network)? {
            if bytes.len() + chunk.len() > 2048 {
                return Err(Error::Invalid("device profile size".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        let peer: DeviceProfile = serde_json::from_slice(&bytes)?;
        peer.validate()?;
        Ok(Some(peer))
    }
    pub async fn register(&self, asset: &Asset) -> Result<AssetStatus> {
        asset.validate()?;
        Self::decode(
            self.http
                .post(format!("{}/v1/assets", self.base))
                .bearer_auth(&self.token)
                .json(asset)
                .send()
                .await
                .map_err(network)?,
        )
        .await
    }
    pub async fn status(&self, id: &str) -> Result<AssetStatus> {
        if !valid_digest(id) {
            return Err(Error::Invalid("asset id".into()));
        }
        Self::decode(
            self.http
                .get(format!("{}/v1/assets/{id}", self.base))
                .bearer_auth(&self.token)
                .send()
                .await
                .map_err(network)?,
        )
        .await
    }
    pub async fn append(
        &self,
        id: &str,
        hash: &str,
        offset: u64,
        bytes: Vec<u8>,
    ) -> Result<AssetStatus> {
        if !valid_digest(id) || !valid_digest(hash) {
            return Err(Error::Invalid("identity".into()));
        }
        let chunk = digest(&bytes);
        Self::decode(
            self.http
                .put(format!("{}/v1/assets/{id}/resources/{hash}", self.base))
                .bearer_auth(&self.token)
                .query(&[("offset", offset.to_string()), ("sha256", chunk)])
                .body(bytes)
                .send()
                .await
                .map_err(network)?,
        )
        .await
    }
    pub async fn commit(&self, id: &str) -> Result<AssetStatus> {
        if !valid_digest(id) {
            return Err(Error::Invalid("asset id".into()));
        }
        Self::decode(
            self.http
                .post(format!("{}/v1/assets/{id}/commit", self.base))
                .bearer_auth(&self.token)
                .send()
                .await
                .map_err(network)?,
        )
        .await
    }
    /// Reference foreground executor. Native background engines use the same actions.
    /// Re-running after interruption resumes from receiver-confirmed offsets.
    pub async fn send(
        &self,
        asset: &Asset,
        paths: &BTreeMap<String, PathBuf>,
        cancelled: &AtomicBool,
        mut progress: impl FnMut(&AssetStatus),
    ) -> Result<AssetStatus> {
        let caps = self.capabilities().await?;
        if caps.version != PROTOCOL_VERSION {
            return Err(Error::Unsupported("protocol version".into()));
        }
        let chunk = caps.max_chunk_bytes.min(MAX_CHUNK_BYTES);
        let mut state = self.register(asset).await?;
        loop {
            if cancelled.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            progress(&state);
            match next_action(asset, &state, chunk)? {
                TransferAction::Done => return Ok(state),
                TransferAction::Commit => state = self.commit(&state.asset_id).await?,
                TransferAction::Upload {
                    sha256,
                    offset,
                    length,
                } => {
                    let path = paths
                        .get(&sha256)
                        .ok_or_else(|| Error::Invalid("missing local resource".into()))?
                        .clone();
                    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
                        let mut file = File::open(path)?;
                        file.seek(SeekFrom::Start(offset))?;
                        let mut bytes = vec![0; length as usize];
                        file.read_exact(&mut bytes)?;
                        Ok(bytes)
                    })
                    .await
                    .map_err(|_| Error::Storage("reader unavailable".into()))??;
                    if cancelled.load(Ordering::Relaxed) {
                        return Err(Error::Cancelled);
                    }
                    state = self.append(&state.asset_id, &sha256, offset, bytes).await?;
                }
            }
        }
    }
}
fn network(_: reqwest::Error) -> Error {
    Error::Transport("HTTP request failed; retry by querying receiver state".into())
}
