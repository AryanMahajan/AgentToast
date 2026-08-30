//! End-to-end tests over a real socket.
//!
//! The unit tests cover the pieces; these cover the thing that actually
//! matters, which is that an unpaired request cannot reach the API and a paired
//! one can. Those are properties of the assembled server — the router, the
//! middleware and the handlers together — so they are worth testing against a
//! bound port rather than by calling functions.
//!
//! The client is written by hand rather than pulled in: these requests are a
//! handful of lines each, and some of them are deliberately malformed in ways a
//! polite HTTP client would refuse to send.

use agenttoast_core::event::{ActionType, AttentionEvent};
use agenttoast_core::router::ActionRouter;
use agenttoast_core::session::{AgentType, Session, SessionRegistry};
use agenttoast_remote::server::{self, RemoteState, Running};
use agenttoast_remote::{Pairing, Store};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;

/* ------------------------------------------------------------------ client --- */

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Send a raw request and read the whole reply.
///
/// `Connection: close` is what makes reading to EOF the right thing to do; a
/// keep-alive reply would leave the socket open and the read would hang.
fn send(addr: SocketAddr, request: &str) -> Reply {
    let mut stream = TcpStream::connect(addr).expect("the server should be listening");
    stream.write_all(request.as_bytes()).expect("write");
    stream.flush().expect("flush");

    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).expect("read");
    let raw = String::from_utf8_lossy(&raw).into_owned();

    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let mut lines = head.lines();

    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);

    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
        .collect();

    Reply {
        status,
        headers,
        body: body.to_string(),
    }
}

fn request(
    addr: SocketAddr,
    method: &str,
    path: &str,
    host: &str,
    extra: &[(&str, &str)],
    body: Option<&str>,
) -> Reply {
    let mut raw = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for (name, value) in extra {
        raw.push_str(&format!("{name}: {value}\r\n"));
    }
    if let Some(body) = body {
        raw.push_str(&format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ));
    }
    raw.push_str("\r\n");
    if let Some(body) = body {
        raw.push_str(body);
    }

    send(addr, &raw)
}

/* ----------------------------------------------------------------- harness --- */

/// A live server, plus everything needed to drive it.
struct Harness {
    addr: SocketAddr,
    state: RemoteState,
    dir: PathBuf,
    _server: Running,
}

impl Harness {
    async fn start() -> Self {
        // A unique directory per test: the store writes a real file, and tests
        // run in parallel in the same process.
        let dir = std::env::temp_dir().join(format!(
            "agenttoast-remote-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");

        let state = RemoteState {
            sessions: SessionRegistry::new(),
            router: ActionRouter::new(),
            store: Store::load(&dir),
            pairing: Pairing::new(),
        };

        // Port 0 asks the OS for a free one, so tests never collide with each
        // other or with a real AgentToast running on this machine.
        let server = server::start(state.clone(), 0).await.expect("binds");
        // It binds 0.0.0.0; connect over loopback.
        let addr = SocketAddr::from(([127, 0, 0, 1], server.addr.port()));

        Self {
            addr,
            state,
            dir,
            _server: server,
        }
    }

    fn host(&self) -> String {
        self.addr.to_string()
    }

    fn get(&self, path: &str, extra: &[(&str, &str)]) -> Reply {
        request(self.addr, "GET", path, &self.host(), extra, None)
    }

    fn post(&self, path: &str, body: &str, extra: &[(&str, &str)]) -> Reply {
        request(self.addr, "POST", path, &self.host(), extra, Some(body))
    }

    /// Go through pairing the way a phone does, and return its cookie.
    async fn pair(&self) -> String {
        let issued = self.state.pairing.issue().await;
        let reply = self.get(&format!("/pair?c={}", issued.code), &[]);

        assert_eq!(reply.status, 303, "pairing should redirect");
        let cookie = reply
            .header("set-cookie")
            .expect("pairing should set a cookie");

        assert!(cookie.contains("HttpOnly"), "cookie must be HttpOnly");
        assert!(cookie.contains("SameSite=Strict"), "cookie must be SameSite=Strict");

        cookie
            .split(';')
            .next()
            .expect("a cookie pair")
            .trim()
            .to_string()
    }

    /// Put a request in front of the user, and hand back what the agent is
    /// waiting on.
    async fn pending(&self) -> (AttentionEvent, tokio::sync::oneshot::Receiver<agenttoast_core::event::UserResponse>) {
        let event = AttentionEvent::permission_request(
            "session-1",
            "claude",
            "Run cargo test",
            Some("Bash".into()),
        );

        let waiting = self.state.router.register(&event).await;
        self.state
            .sessions
            .register(Session::new("session-1", AgentType::ClaudeCode))
            .await;
        self.state
            .sessions
            .set_attention("session-1", event.clone())
            .await;

        (event, waiting)
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const API: (&str, &str) = ("X-AgentToast", "1");

/* ------------------------------------------------------------------- tests --- */

#[tokio::test(flavor = "multi_thread")]
async fn an_unpaired_browser_is_told_how_to_pair() {
    let app = Harness::start().await;

    let reply = app.get("/", &[]);
    assert_eq!(reply.status, 200);
    assert!(reply.body.contains("Pair this device"));
    // And nothing about the machine leaks into that page.
    assert!(!reply.body.contains("session-1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_addressed_by_hostname_is_refused() {
    let app = Harness::start().await;

    // The DNS-rebinding shape: a browser on a page from evil.example.com, which
    // resolves to this machine's LAN address.
    let reply = request(app.addr, "GET", "/", "evil.example.com", &[], None);
    assert_eq!(reply.status, 421);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_api_refuses_a_browser_that_never_paired() {
    let app = Harness::start().await;

    let reply = app.get("/api/state", &[API]);
    assert_eq!(reply.status, 401);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_api_refuses_a_valid_cookie_without_the_header() {
    let app = Harness::start().await;
    let cookie = app.pair().await;

    // This is the cross-site shape: a form post from another origin carries the
    // cookie but cannot add a custom header without a preflight.
    let reply = app.get("/api/state", &[("Cookie", &cookie)]);
    assert_eq!(reply.status, 401);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paired_device_sees_what_is_waiting() {
    let app = Harness::start().await;
    let cookie = app.pair().await;
    let (event, _waiting) = app.pending().await;

    let reply = app.get("/api/state", &[API, ("Cookie", &cookie)]);
    assert_eq!(reply.status, 200);
    assert!(reply.body.contains(&event.event_id.to_string()));
    assert!(reply.body.contains("Run cargo test"));
    // Answers must never be cached — a stale list would show a request that has
    // already been dealt with.
    assert_eq!(reply.header("cache-control"), Some("no-store"));
}

#[tokio::test(flavor = "multi_thread")]
async fn approving_from_a_phone_unblocks_the_agent() {
    let app = Harness::start().await;
    let cookie = app.pair().await;
    let (event, waiting) = app.pending().await;

    let body = format!(r#"{{"event_id":"{}","action":"approve"}}"#, event.event_id);
    let reply = app.post("/api/respond", &body, &[API, ("Cookie", &cookie)]);

    assert_eq!(reply.status, 200);
    let answer = waiting.await.expect("the agent should get an answer");
    assert_eq!(answer.action, ActionType::Approve);
}

#[tokio::test(flavor = "multi_thread")]
async fn answering_twice_is_reported_rather_than_silently_ignored() {
    let app = Harness::start().await;
    let cookie = app.pair().await;
    let (event, _waiting) = app.pending().await;

    let body = format!(r#"{{"event_id":"{}","action":"deny"}}"#, event.event_id);
    assert_eq!(
        app.post("/api/respond", &body, &[API, ("Cookie", &cookie)])
            .status,
        200
    );

    // The realistic case is the desktop answering first, and the phone needs to
    // be told its tap did nothing rather than shown a success.
    let second = app.post("/api/respond", &body, &[API, ("Cookie", &cookie)]);
    assert_eq!(second.status, 409);
}

#[tokio::test(flavor = "multi_thread")]
async fn approving_is_refused_when_the_machine_only_allows_denying() {
    let app = Harness::start().await;
    app.state
        .store
        .set_allow_approve(false)
        .await
        .expect("saves");
    let cookie = app.pair().await;
    let (event, _waiting) = app.pending().await;

    let approve = format!(r#"{{"event_id":"{}","action":"approve"}}"#, event.event_id);
    assert_eq!(
        app.post("/api/respond", &approve, &[API, ("Cookie", &cookie)])
            .status,
        403
    );

    // Denying still works — that is the whole point of the setting.
    let deny = format!(r#"{{"event_id":"{}","action":"deny"}}"#, event.event_id);
    assert_eq!(
        app.post("/api/respond", &deny, &[API, ("Cookie", &cookie)])
            .status,
        200
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_device_locks_it_out_at_once() {
    let app = Harness::start().await;
    let cookie = app.pair().await;

    assert_eq!(app.get("/api/state", &[API, ("Cookie", &cookie)]).status, 200);

    let device = app.state.store.devices().await.remove(0);
    app.state.store.revoke(&device.id).await.expect("revokes");

    // No restart, no cache to expire — the next request is already refused.
    assert_eq!(app.get("/api/state", &[API, ("Cookie", &cookie)]).status, 401);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pairing_code_cannot_be_used_twice() {
    let app = Harness::start().await;
    let issued = app.state.pairing.issue().await;

    let first = app.get(&format!("/pair?c={}", issued.code), &[]);
    assert_eq!(first.status, 303);

    // Someone else scanning the same QR a moment later gets nothing.
    let second = app.get(&format!("/pair?c={}", issued.code), &[]);
    assert_eq!(second.status, 403);
    assert!(second.body.contains("expired or was already used"));
    assert!(second.header("set-cookie").is_none());

    assert_eq!(app.state.store.devices().await.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn actions_that_only_make_sense_at_the_desk_are_refused() {
    let app = Harness::start().await;
    let cookie = app.pair().await;
    let (event, _waiting) = app.pending().await;

    // Raising a window on a machine you are not sitting at is not a thing a
    // phone should be able to ask for.
    let body = format!(r#"{{"event_id":"{}","action":"open_session"}}"#, event.event_id);
    assert_eq!(
        app.post("/api/respond", &body, &[API, ("Cookie", &cookie)])
            .status,
        400
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_the_server_closes_the_port() {
    let app = Harness::start().await;
    let addr = app.addr;
    assert_eq!(app.get("/", &[]).status, 200);

    drop(app);

    // Give the listener a moment to actually come down.
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    assert!(
        TcpStream::connect(addr).is_err(),
        "the port should be closed once the handle is dropped"
    );
}
