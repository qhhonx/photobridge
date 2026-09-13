//! Short-lived, single-use desktop pairing. Credentials only cross pinned TLS.
use super::*;
use axum::{
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use std::{
    net::IpAddr,
    time::{Duration, Instant},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DesktopInvite {
    pub kind: String,
    pub connection: Pairing,
}
struct Exchange {
    token: String,
    deadline: Instant,
    busy: AtomicBool,
    accepted: Mutex<Option<Pairing>>,
}
pub struct DesktopPairing {
    pub invite: DesktopInvite,
    exchange: Arc<Exchange>,
    handle: axum_server::Handle,
    task: tokio::task::JoinHandle<()>,
}
fn local_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_private() || v.is_loopback() || v.is_link_local(),
        IpAddr::V6(v) => v.is_loopback() || v.is_unique_local() || v.is_unicast_link_local(),
    }
}
pub(super) fn validate_local_endpoint(endpoint: &str) -> Result<()> {
    let url =
        reqwest::Url::parse(endpoint).map_err(|_| Error::Invalid("pairing endpoint".into()))?;
    let host = url.host_str().unwrap_or_default().trim_matches(['[', ']']);
    let ip: IpAddr = host
        .parse()
        .map_err(|_| Error::Invalid("local pairing address".into()))?;
    if url.scheme() != "https"
        || !local_address(ip)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::Invalid("local pairing endpoint".into()));
    }
    Ok(())
}
impl DesktopPairing {
    pub async fn start(ip: IpAddr) -> Result<Self> {
        Self::start_for(ip, Duration::from_secs(300)).await
    }
    async fn start_for(ip: IpAddr, lifetime: Duration) -> Result<Self> {
        if !local_address(ip) {
            return Err(Error::Invalid("local pairing address".into()));
        }
        let _ = rustls::crypto::ring::default_provider().install_default();
        let listener = std::net::TcpListener::bind(SocketAddr::new(ip, 0))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let rcgen::CertifiedKey { cert, key_pair } =
            rcgen::generate_simple_self_signed(vec![ip.to_string()])
                .map_err(|_| Error::Storage("pairing certificate".into()))?;
        let mut random = [0u8; 32];
        getrandom::fill(&mut random).map_err(|_| Error::Storage("random source".into()))?;
        let token = random
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect::<String>();
        let invite = DesktopInvite {
            kind: "photobridge_desktop_pairing".into(),
            connection: Pairing {
                version: PROTOCOL_VERSION,
                receiver_id: digest(cert.der()),
                endpoint: format!("https://{addr}"),
                certificate: STANDARD.encode(cert.der()),
                token: token.clone(),
                certificate_name: None,
            },
        };
        let exchange = Arc::new(Exchange {
            token,
            deadline: Instant::now() + lifetime,
            busy: AtomicBool::new(false),
            accepted: Mutex::new(None),
        });
        let router = Router::new()
            .route("/pair", post(accept))
            .layer(DefaultBodyLimit::max(32768))
            .with_state(exchange.clone());
        let config = axum_server::tls_rustls::RustlsConfig::from_pem(
            cert.pem().into_bytes(),
            key_pair.serialize_pem().into_bytes(),
        )
        .await?;
        let handle = axum_server::Handle::new();
        let server = axum_server::from_tcp_rustls(listener, config).handle(handle.clone());
        let task = tokio::spawn(async move {
            tokio::select! {
                _ = server.serve(router.into_make_service()) => {},
                _ = tokio::time::sleep(lifetime) => {},
            }
        });
        Ok(Self {
            invite,
            exchange,
            handle,
            task,
        })
    }
    pub fn status(&self) -> Result<Value> {
        Ok(
            json!({"expired": Instant::now() >= self.exchange.deadline, "pairing": self.exchange.accepted.lock().map_err(lock)?.clone()}),
        )
    }
}
impl Drop for DesktopPairing {
    fn drop(&mut self) {
        self.handle.shutdown();
        self.task.abort();
    }
}
async fn accept(
    State(exchange): State<Arc<Exchange>>,
    headers: HeaderMap,
    Json(pairing): Json<Pairing>,
) -> StatusCode {
    let auth = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    if digest(auth.as_bytes()) != digest(format!("Bearer {}", exchange.token).as_bytes()) {
        return StatusCode::UNAUTHORIZED;
    }
    if Instant::now() >= exchange.deadline {
        return StatusCode::GONE;
    }
    if exchange.busy.swap(true, Ordering::SeqCst) {
        return StatusCode::CONFLICT;
    }
    let checked = async {
        validate_local_endpoint(&pairing.endpoint)?;
        let caps = pairing.client()?.capabilities().await?;
        if caps.version != PROTOCOL_VERSION {
            return Err(Error::Unsupported("protocol version".into()));
        }
        Ok::<_, Error>(())
    };
    if !matches!(
        tokio::time::timeout(Duration::from_secs(12), checked).await,
        Ok(Ok(()))
    ) {
        exchange.busy.store(false, Ordering::SeqCst);
        return StatusCode::BAD_REQUEST;
    }
    if Instant::now() >= exchange.deadline {
        return StatusCode::GONE;
    }
    if let Ok(mut value) = exchange.accepted.lock() {
        *value = Some(pairing);
        StatusCode::OK
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}
pub async fn submit(invite: DesktopInvite, pairing: Pairing) -> Result<()> {
    if invite.kind != "photobridge_desktop_pairing" {
        return Err(Error::Invalid("desktop pairing code".into()));
    }
    validate_local_endpoint(&invite.connection.endpoint)?;
    invite.connection.client()?; // Same identity validation as normal pairing.
    pairing.client()?;
    let der = STANDARD
        .decode(&invite.connection.certificate)
        .map_err(|_| Error::Invalid("certificate".into()))?;
    let cert =
        reqwest::Certificate::from_der(&der).map_err(|_| Error::Invalid("certificate".into()))?;
    let client = reqwest::Client::builder()
        .tls_built_in_root_certs(false)
        .add_root_certificate(cert)
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| Error::Invalid("pairing TLS".into()))?;
    let response = client
        .post(format!("{}/pair", invite.connection.endpoint))
        .bearer_auth(&invite.connection.token)
        .json(&pairing)
        .send()
        .await
        .map_err(|_| Error::Invalid("pairing connection".into()))?;
    if !response.status().is_success() {
        return Err(Error::Conflict("pairing rejected or expired".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn desktop_exchange_is_pinned_single_use_and_expires() {
        let root =
            std::env::temp_dir().join(format!("photobridge-pair-test-{}", std::process::id()));
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let receiver = ReceiverHost::start(&root, SocketAddr::from(([127, 0, 0, 1], port)), 1000)
            .await
            .unwrap();
        let desktop = DesktopPairing::start("127.0.0.1".parse().unwrap())
            .await
            .unwrap();
        let mut wrong = desktop.invite.clone();
        wrong.connection.token = digest(b"wrong");
        assert!(submit(wrong, receiver.pairing.clone()).await.is_err());
        assert!(desktop.status().unwrap()["pairing"].is_null());
        let other = DesktopPairing::start("127.0.0.1".parse().unwrap())
            .await
            .unwrap();
        let mut wrong_cert = desktop.invite.clone();
        wrong_cert.connection.certificate = other.invite.connection.certificate.clone();
        wrong_cert.connection.receiver_id = other.invite.connection.receiver_id.clone();
        assert!(submit(wrong_cert, receiver.pairing.clone()).await.is_err());
        submit(desktop.invite.clone(), receiver.pairing.clone())
            .await
            .unwrap();
        assert_eq!(
            desktop.status().unwrap()["pairing"]["receiver_id"],
            receiver.pairing.receiver_id
        );
        assert!(submit(desktop.invite.clone(), receiver.pairing.clone())
            .await
            .is_err());
        let expired = DesktopPairing::start_for("127.0.0.1".parse().unwrap(), Duration::ZERO)
            .await
            .unwrap();
        assert!(submit(expired.invite.clone(), receiver.pairing.clone())
            .await
            .is_err());
        assert!(expired.status().unwrap()["expired"].as_bool().unwrap());
        drop(receiver);
        let _ = fs::remove_dir_all(root);
    }
}
