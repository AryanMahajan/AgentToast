//! The HTTP server the phone talks to.
//!
//! # What defends this
//!
//! This is the only part of AgentToast that accepts a connection from another
//! machine, and what it accepts is "run that command". So the guards are worth
//! stating plainly:
//!
//! - **Off until switched on.** Nothing listens until someone enables it.
//! - **A per-device token, delivered by pairing.** The QR carries a one-time
//!   code, not the token; the token comes back in an `HttpOnly` cookie, so it
//!   is never in a URL, a QR screenshot or the page's own JavaScript.
//! - **`SameSite=Strict`.** Another site open on the same phone cannot make the
//!   browser attach the cookie to a request it forged.
//! - **A custom header on every API call.** A cross-origin request carrying one
//!   needs a CORS preflight, and nothing here answers a preflight — so a
//!   malicious page cannot reach the API even by hand.
//! - **`Host` must be an IP literal.** DNS rebinding — where a hostile site
//!   points its own domain at your LAN address so the browser treats it as
//!   same-origin — needs a domain name to work. There isn't one to use.
//!
//! # What does not defend it
//!
//! Plain HTTP. Anyone able to watch traffic on the network can read the cookie
//! and everything else. On a home or office network that is an acceptable
//! trade for not training people to click through certificate warnings; on
//! untrusted wifi it is not, and the dashboard says so.

use agenttoast_core::event::{ActionType, UserResponse};
use agenttoast_core::router::ActionRouter;
use agenttoast_core::session::SessionRegistry;
use anyhow::{Context, Result};
use axum::body::Body;
use axum::extract::{Query, Request, State};
use axum::http::{HeaderMap, Response as HttpResponse, StatusCode, header};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router, middleware};
use serde::Deserialize;
use serde_json::json;
use std::net::SocketAddr;
use tracing::{info, warn};

use crate::pairing::Pairing;
use crate::store::Store;
use crate::view;

/// Cookie holding the device token.
pub const COOKIE_NAME: &str = "agenttoast_device";

/// Header every API call must carry. The value is not checked — its presence is
/// the whole point, because a cross-origin request cannot set it without a
/// preflight this server refuses.
pub const API_HEADER: &str = "x-agenttoast";

/// A year. Long enough that a phone stays paired; the dashboard's Revoke is the
/// intended way to end it, not expiry.
const COOKIE_MAX_AGE: i64 = 60 * 60 * 24 * 365;

const PAGE: &str = include_str!("page.html");
const UNPAIRED: &str = include_str!("unpaired.html");

/// Everything the handlers need.
#[derive(Clone)]
pub struct RemoteState {
    pub sessions: SessionRegistry,
    pub router: ActionRouter,
    pub store: Store,
    pub pairing: Pairing,
}

/// A listening server. Dropping this stops it.
///
/// Tying the lifetime to the handle means "turn the remote off" is `= None` on
/// one field, and there is no way to leave a socket listening after the state
/// says it should not be.
pub struct Running {
    pub addr: SocketAddr,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Running {
    /// Stop the server and wait for nothing — in-flight requests finish on
    /// their own.
    pub fn stop(mut self) {
        self.signal();
    }

    fn signal(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
            info!(addr = %self.addr, "Remote server stopped");
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.signal();
    }
}

/// Bind and start serving.
///
/// Binds `0.0.0.0` so the phone can reach it on whichever interface it shares
/// with the machine — picking one would mean guessing between wifi, ethernet
/// and a VPN, and guessing wrong looks exactly like the feature being broken.
pub async fn start(state: RemoteState, port: u16) -> Result<Running> {
    let listener = tokio::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port)))
        .await
        .with_context(|| format!("Could not listen on port {port}"))?;

    let addr = listener.local_addr()?;
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();

    let app = app(state);
    tokio::spawn(async move {
        let served = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = rx.await;
            })
            .await;

        if let Err(e) = served {
            warn!(error = %e, "Remote server stopped unexpectedly");
        }
    });

    info!(%addr, "Remote server listening");
    Ok(Running {
        addr,
        shutdown: Some(tx),
    })
}

fn app(state: RemoteState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/pair", get(pair))
        .route("/api/state", get(api_state))
        .route("/api/respond", post(api_respond))
        .fallback(not_found)
        .layer(middleware::from_fn(require_literal_host))
        .with_state(state)
}

/* ----------------------------------------------------------------- guards --- */

/// Refuse anything addressed by name rather than by address.
///
/// This is the DNS-rebinding guard. An attacker's page can make a browser send
/// requests here, but only ever to `something.example.com`; there is no way to
/// make a browser send `Host: 192.168.1.40` for a page loaded from a domain.
async fn require_literal_host(req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    if !host_is_literal(host) {
        warn!(host, "Refused a request addressed by hostname");
        return (
            StatusCode::MISDIRECTED_REQUEST,
            "Reach AgentToast by IP address, not by name.",
        )
            .into_response();
    }

    next.run(req).await
}

/// Whether a `Host` header names an address rather than a domain.
pub fn host_is_literal(host: &str) -> bool {
    let host = host.trim();
    if host.is_empty() {
        return false;
    }

    // `[::1]:8787` — the brackets exist precisely because the colons are
    // otherwise ambiguous with the port separator.
    let name = match host.strip_prefix('[') {
        Some(rest) => match rest.split_once(']') {
            Some((inner, _)) => inner,
            None => return false,
        },
        None => host.rsplit_once(':').map_or(host, |(name, _)| name),
    };

    name.eq_ignore_ascii_case("localhost") || name.parse::<std::net::IpAddr>().is_ok()
}

/// Read one cookie out of a request.
fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| key.trim() == name)
        .map(|(_, value)| value.trim().to_string())
}

/// The device behind a request, if it is paired.
async fn device_for(state: &RemoteState, headers: &HeaderMap) -> Option<crate::store::Device> {
    let token = cookie(headers, COOKIE_NAME)?;
    state.store.authenticate(&token).await
}

/// The device behind an *API* request, which additionally has to prove it was
/// made by our own page rather than by a form on someone else's.
async fn api_caller(state: &RemoteState, headers: &HeaderMap) -> Option<crate::store::Device> {
    if !headers.contains_key(API_HEADER) {
        return None;
    }
    device_for(state, headers).await
}

/* --------------------------------------------------------------- handlers --- */

async fn index(State(state): State<RemoteState>, headers: HeaderMap) -> Response {
    match device_for(&state, &headers).await {
        Some(_) => Html(PAGE).into_response(),
        // 200 rather than 401: this is a page for a person, not an API, and the
        // body gives nothing away — it says "pair this device from your PC".
        None => Html(UNPAIRED).into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct PairQuery {
    #[serde(default)]
    c: String,
}

async fn pair(
    State(state): State<RemoteState>,
    headers: HeaderMap,
    Query(query): Query<PairQuery>,
) -> Response {
    // Already paired and the QR was scanned again: nothing to do but go in.
    if device_for(&state, &headers).await.is_some() {
        state.pairing.cancel().await;
        return redirect_home(None);
    }

    if !state.pairing.redeem(&query.c).await {
        warn!("Rejected a pairing attempt: the code was wrong, spent or expired");
        return (
            StatusCode::FORBIDDEN,
            Html(UNPAIRED.replace(
                "<!--STATUS-->",
                "<p class=\"status\">That pairing code has expired or was already used.</p>",
            )),
        )
            .into_response();
    }

    let name = device_name(&headers);
    match state.store.add_device(name).await {
        Ok(device) => redirect_home(Some(&device.token)),
        Err(e) => {
            warn!(error = %e, "Could not save the newly paired device");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not save the pairing. Check the AgentToast log.",
            )
                .into_response()
        }
    }
}

/// Send the browser to `/`, optionally handing it its token on the way.
///
/// `HttpOnly` keeps the token out of reach of the page's own script, so an
/// injected script cannot exfiltrate it. `SameSite=Strict` is what stops
/// another site on the same phone from riding the cookie. `Secure` is
/// deliberately absent: this is plain HTTP, and a `Secure` cookie would simply
/// never be stored.
fn redirect_home(token: Option<&str>) -> Response {
    let mut response = HttpResponse::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, "/");

    if let Some(token) = token {
        response = response.header(
            header::SET_COOKIE,
            format!(
                "{COOKIE_NAME}={token}; Path=/; Max-Age={COOKIE_MAX_AGE}; HttpOnly; SameSite=Strict"
            ),
        );
    }

    response
        .body(Body::empty())
        .map(IntoResponse::into_response)
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn api_state(State(state): State<RemoteState>, headers: HeaderMap) -> Response {
    let Some(device) = api_caller(&state, &headers).await else {
        return unauthorised();
    };

    let settings = state.store.settings().await;
    let sessions = state.sessions.all().await;

    no_store(Json(view::build(
        &device.name,
        settings.allow_approve,
        sessions,
    )))
}

#[derive(Debug, Deserialize)]
struct RespondBody {
    event_id: String,
    action: String,
}

async fn api_respond(
    State(state): State<RemoteState>,
    headers: HeaderMap,
    Json(body): Json<RespondBody>,
) -> Response {
    let Some(device) = api_caller(&state, &headers).await else {
        return unauthorised();
    };

    let Ok(event_id) = body.event_id.parse::<uuid::Uuid>() else {
        return refuse(StatusCode::BAD_REQUEST, "That is not an event id.");
    };

    let action = match body.action.as_str() {
        "approve" => ActionType::Approve,
        "confirm" => ActionType::Confirm,
        "deny" => ActionType::Deny,
        "reject" => ActionType::Reject,
        // Everything else is desktop-only on purpose — see `view::request_view`.
        other => {
            warn!(device = %device.name, action = other, "Refused an action a phone cannot take");
            return refuse(StatusCode::BAD_REQUEST, "That action cannot be taken remotely.");
        }
    };

    let affirmative = matches!(action, ActionType::Approve | ActionType::Confirm);
    if affirmative && !state.store.settings().await.allow_approve {
        warn!(device = %device.name, "Refused a remote approval: approving is switched off");
        return refuse(
            StatusCode::FORBIDDEN,
            "Approving from a phone is switched off for this machine.",
        );
    }

    let response = UserResponse {
        event_id,
        action: action.clone(),
        text_input: None,
    };

    match state.router.resolve(response).await {
        Ok(()) => {
            // The only durable record that a decision was made from somewhere
            // other than this desk.
            info!(
                device = %device.name,
                device_id = %device.id,
                %event_id,
                action = ?action,
                "Answered from a paired device"
            );
            no_store(Json(json!({ "ok": true })))
        }
        Err(e) => {
            // Almost always a race: answered on the desktop a moment ago, or
            // the agent gave up waiting. Not an error worth alarming anyone
            // about, but the phone needs to know its tap did nothing.
            info!(%event_id, reason = %e, "Nothing was waiting on that answer");
            refuse(StatusCode::CONFLICT, "That request was already answered.")
        }
    }
}

async fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "Not found.").into_response()
}

/* ---------------------------------------------------------------- replies --- */

fn no_store(body: impl IntoResponse) -> Response {
    ([(header::CACHE_CONTROL, "no-store")], body).into_response()
}

fn unauthorised() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "ok": false, "error": "This device is not paired." })),
    )
        .into_response()
}

fn refuse(status: StatusCode, message: &str) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({ "ok": false, "error": message })),
    )
        .into_response()
}

/// A readable name for a device, guessed from its `User-Agent`.
///
/// Only ever a label in a list someone revokes from, so a wrong guess is
/// harmless. It exists because "device a4f9c210" tells nobody which phone that
/// is, and a list you cannot read is a list you never prune.
fn device_name(headers: &HeaderMap) -> String {
    let agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    let device = if agent.contains("iPhone") {
        "iPhone"
    } else if agent.contains("iPad") {
        "iPad"
    } else if agent.contains("Android") {
        "Android"
    } else if agent.contains("Macintosh") {
        "Mac"
    } else if agent.contains("Windows") {
        "Windows PC"
    } else if agent.contains("Linux") {
        "Linux"
    } else {
        "Device"
    };

    // Order matters: Edge's user agent contains "Chrome", and Chrome's contains
    // "Safari", so the most specific claim has to be tested first.
    let browser = if agent.contains("Edg/") {
        Some("Edge")
    } else if agent.contains("Firefox/") {
        Some("Firefox")
    } else if agent.contains("CriOS") || agent.contains("Chrome/") {
        Some("Chrome")
    } else if agent.contains("Safari/") {
        Some("Safari")
    } else {
        None
    };

    match browser {
        Some(browser) => format!("{device} · {browser}"),
        None => device.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(
                header::HeaderName::from_bytes(name.as_bytes()).expect("valid header name"),
                value.parse().expect("valid header value"),
            );
        }
        headers
    }

    #[test]
    fn addresses_are_accepted_and_names_are_not() {
        assert!(host_is_literal("192.168.1.40:8787"));
        assert!(host_is_literal("192.168.1.40"));
        assert!(host_is_literal("127.0.0.1:8787"));
        assert!(host_is_literal("localhost:8787"));
        assert!(host_is_literal("[::1]:8787"));

        // The rebinding vector: a domain the attacker controls, pointed at a
        // LAN address.
        assert!(!host_is_literal("evil.example.com:8787"));
        assert!(!host_is_literal("agenttoast.local"));
        assert!(!host_is_literal(""));
    }

    #[test]
    fn the_right_cookie_is_picked_out_of_several() {
        let headers = headers_with(&[(
            "cookie",
            "theme=dark; agenttoast_device=abc123; other=x",
        )]);
        assert_eq!(cookie(&headers, COOKIE_NAME).as_deref(), Some("abc123"));
        assert_eq!(cookie(&headers, "missing"), None);
    }

    #[test]
    fn a_missing_cookie_header_is_not_an_error() {
        assert_eq!(cookie(&HeaderMap::new(), COOKIE_NAME), None);
    }

    #[test]
    fn device_names_read_like_a_device() {
        let iphone = headers_with(&[(
            "user-agent",
            "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 \
             (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1",
        )]);
        assert_eq!(device_name(&iphone), "iPhone · Safari");

        let android = headers_with(&[(
            "user-agent",
            "Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/120.0.0.0 Mobile Safari/537.36",
        )]);
        assert_eq!(device_name(&android), "Android · Chrome");

        assert_eq!(device_name(&HeaderMap::new()), "Device");
    }

    /// Edge and Chrome both claim to be Safari, and Edge also claims to be
    /// Chrome. Getting this backwards labels every device "Safari".
    #[test]
    fn the_most_specific_browser_claim_wins() {
        let edge = headers_with(&[(
            "user-agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) \
             Chrome/120.0.0.0 Safari/537.36 Edg/120.0.0.0",
        )]);
        assert_eq!(device_name(&edge), "Windows PC · Edge");
    }

    #[test]
    fn the_embedded_pages_are_whole() {
        assert!(PAGE.contains("</html>"));
        assert!(UNPAIRED.contains("</html>"));
        // The page has to send the header the API insists on, or every request
        // it makes will be rejected.
        assert!(PAGE.contains(API_HEADER));
        // And the pairing failure path needs its placeholder to still exist.
        assert!(UNPAIRED.contains("<!--STATUS-->"));
    }

    /// The chime is the only alert a page can give, so the pieces that make it
    /// work are worth pinning: the element ids the script looks up by name are
    /// the kind of thing a careless edit breaks with no visible error.
    #[test]
    fn the_pages_alerting_parts_are_all_present() {
        for id in ["sound", "unlock", "favicon", "mark", "who", "requests", "sessions"] {
            assert!(
                PAGE.contains(&format!("id=\"{id}\"")),
                "the page script looks up #{id}"
            );
        }
        // Audio is blocked until the user taps, so the unlock path has to exist.
        assert!(PAGE.contains("unlockAudio"));
        assert!(PAGE.contains("pointerdown"));
        // And a chime that fires once and never again is missable by design.
        assert!(PAGE.contains("REPEAT_MS"));
    }
}
