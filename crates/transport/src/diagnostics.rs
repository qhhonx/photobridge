//! Optional bounded metadata observations. No request URLs, bodies or credentials.
use super::*;
use axum::body::Body;
use futures_util::StreamExt;
use std::sync::atomic::AtomicU64;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug)]
pub struct RequestObservation {
    pub request_id: u64,
    pub event: &'static str,
    pub observed_at_ms: u64,
    pub duration_ms: u64,
    pub bytes_received: u64,
    pub first_body_at_ms: Option<u64>,
    pub body_complete: bool,
    pub status: Option<u16>,
}
pub type Observer = Arc<dyn Fn(RequestObservation) + Send + Sync>;
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
struct Observation {
    observer: Observer,
    id: u64,
    start: Instant,
    bytes: AtomicU64,
    first: AtomicU64,
    complete: AtomicBool,
}
impl Observation {
    fn emit(&self, event: &'static str, status: Option<u16>) {
        let first = self.first.load(Ordering::Relaxed);
        (self.observer)(RequestObservation {
            request_id: self.id,
            event,
            observed_at_ms: now_ms(),
            duration_ms: self.start.elapsed().as_millis() as u64,
            bytes_received: self.bytes.load(Ordering::Relaxed),
            first_body_at_ms: (first != 0).then_some(first),
            body_complete: self.complete.load(Ordering::Relaxed),
            status,
        });
    }
}
pub(super) async fn observe(request: Request, next: Next, observer: Option<Observer>) -> Response {
    let id = request
        .headers()
        .get("x-photobridge-request")
        .and_then(|h| h.to_str().ok())
        .filter(|s| s.len() <= 16 && s.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|n| *n > 0 && *n <= (1 << 52));
    let (Some(observer), Some(id)) = (observer, id) else {
        return run(request, next).await;
    };
    let observation = Arc::new(Observation {
        observer,
        id,
        start: Instant::now(),
        bytes: AtomicU64::new(0),
        first: AtomicU64::new(0),
        complete: AtomicBool::new(false),
    });
    observation.emit("receiver_request_started", None);
    let (parts, body) = request.into_parts();
    let stream = futures_util::stream::unfold(
        (body.into_data_stream(), observation.clone()),
        |(mut stream, observation)| async move {
            match stream.next().await {
                Some(frame) => {
                    if let Ok(data) = &frame {
                        observation
                            .bytes
                            .fetch_add(data.len() as u64, Ordering::Relaxed);
                        if !data.is_empty()
                            && observation
                                .first
                                .compare_exchange(0, now_ms(), Ordering::Relaxed, Ordering::Relaxed)
                                .is_ok()
                        {
                            observation.emit("receiver_first_body", None);
                        }
                    }
                    Some((frame, (stream, observation)))
                }
                None => {
                    observation.complete.store(true, Ordering::Relaxed);
                    None
                }
            }
        },
    );
    let request = Request::from_parts(parts, Body::from_stream(stream));
    let response = run(request, next).await;
    // Handler completion is not proof that the peer received the response.
    observation.emit("receiver_response_ready", Some(response.status().as_u16()));
    response
}

async fn run(request: Request, next: Next) -> Response {
    if request.uri().path() == "/v1/bundles" {
        return next.run(request).await;
    }
    match tokio::time::timeout(Duration::from_secs(60), next.run(request)).await {
        Ok(response) => response,
        Err(_) => StatusCode::REQUEST_TIMEOUT.into_response(),
    }
}
