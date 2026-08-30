//! What the phone is allowed to see.
//!
//! Deliberately not the internal [`Session`] and [`AttentionEvent`] types. Those
//! carry things the phone has no use for and should not be handed — a process
//! id, for one — and freezing them into a network contract would mean every
//! future field on an internal struct silently becoming public API. So this is
//! a hand-written projection, and adding a field to it is a decision.

use agenttoast_core::event::{ActionType, AttentionEvent};
use agenttoast_core::session::Session;
use serde::Serialize;

/// The whole picture, as one poll returns it.
#[derive(Debug, Serialize)]
pub struct StateView {
    /// The name of the device doing the asking, so it can say "you are the
    /// iPhone" and someone can tell which entry in the dashboard to revoke.
    pub device: String,
    /// Whether this daemon accepts approvals at all. The page hides the
    /// affirmative buttons when it does not, rather than offering a button that
    /// would be refused.
    pub allow_approve: bool,
    /// Requests actually waiting on an answer, newest last.
    pub requests: Vec<RequestView>,
    /// Everything else that is running, for context.
    pub sessions: Vec<SessionView>,
}

/// One request, with the buttons that should appear under it.
#[derive(Debug, Serialize)]
pub struct RequestView {
    pub event_id: String,
    pub session_id: String,
    pub agent: String,
    pub title: String,
    pub detail: Option<String>,
    pub tool: Option<String>,
    pub folder: Option<String>,
    pub waiting_seconds: i64,
    pub actions: Vec<ActionView>,
}

/// A button. `kind` drives the styling; `action` is what to POST back.
#[derive(Debug, Serialize)]
pub struct ActionView {
    pub action: &'static str,
    pub label: String,
    pub kind: &'static str,
}

#[derive(Debug, Serialize)]
pub struct SessionView {
    pub session_id: String,
    pub agent: String,
    pub state: String,
    pub folder: Option<String>,
    pub waiting: bool,
}

/// Build the phone's view of the world.
pub fn build(device: &str, allow_approve: bool, sessions: Vec<Session>) -> StateView {
    let mut requests: Vec<RequestView> = sessions
        .iter()
        .filter_map(|s| s.attention_request.as_ref())
        .filter_map(|event| request_view(event, allow_approve))
        .collect();

    // Oldest first: the thing that has been blocking longest is the thing to
    // deal with first, and a stable order stops buttons moving under a thumb
    // as new requests arrive.
    requests.sort_by_key(|r| -r.waiting_seconds);

    let mut sessions: Vec<SessionView> = sessions
        .iter()
        .map(|s| SessionView {
            session_id: s.session_id.clone(),
            agent: s.agent_type.to_string(),
            state: s.state.to_string(),
            folder: s.working_directory.clone(),
            waiting: s.state.needs_attention(),
        })
        .collect();
    sessions.sort_by(|a, b| b.waiting.cmp(&a.waiting).then(a.agent.cmp(&b.agent)));

    StateView {
        device: device.to_string(),
        allow_approve,
        requests,
        sessions,
    }
}

/// Project one event, or `None` if there is nothing to answer.
///
/// A completion toast and a "the agent has a question" notice both carry only
/// `OpenSession`, which is meaningless on a phone — you cannot raise a window
/// you are not sitting in front of. Those are dropped rather than shown as a
/// card with no buttons.
fn request_view(event: &AttentionEvent, allow_approve: bool) -> Option<RequestView> {
    let actions: Vec<ActionView> = event
        .actions
        .iter()
        .filter_map(|a| {
            let (action, kind) = match a.action_type {
                ActionType::Approve => ("approve", "affirm"),
                ActionType::Confirm => ("confirm", "affirm"),
                ActionType::Deny => ("deny", "reject"),
                ActionType::Reject => ("reject", "reject"),
                // Nothing a phone can usefully do with these.
                ActionType::OpenSession | ActionType::SendText => return None,
            };
            if kind == "affirm" && !allow_approve {
                return None;
            }
            Some(ActionView {
                action,
                label: a.label.clone(),
                kind,
            })
        })
        .collect();

    if actions.is_empty() {
        return None;
    }

    Some(RequestView {
        event_id: event.event_id.to_string(),
        session_id: event.session_id.clone(),
        agent: event.agent.clone(),
        title: event.message.clone(),
        detail: event.context.clone(),
        tool: event.tool_name.clone(),
        folder: event.cwd.clone(),
        waiting_seconds: (chrono::Utc::now() - event.timestamp).num_seconds().max(0),
        actions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use agenttoast_core::session::{AgentType, Session};

    fn session_with(event: AttentionEvent) -> Session {
        let mut session = Session::new(&event.session_id, AgentType::ClaudeCode);
        session.set_attention(event);
        session
    }

    #[test]
    fn a_permission_request_offers_approve_and_deny() {
        let event = AttentionEvent::permission_request("s1", "claude", "Run tests", None);
        let view = build("Phone", true, vec![session_with(event)]);

        let actions: Vec<&str> = view.requests[0].actions.iter().map(|a| a.action).collect();
        assert_eq!(actions, vec!["approve", "deny"]);
    }

    #[test]
    fn approve_disappears_when_the_daemon_does_not_allow_it() {
        let event = AttentionEvent::permission_request("s1", "claude", "Run tests", None);
        let view = build("Phone", false, vec![session_with(event)]);

        let actions: Vec<&str> = view.requests[0].actions.iter().map(|a| a.action).collect();
        assert_eq!(actions, vec!["deny"]);
    }

    #[test]
    fn open_session_is_never_offered() {
        let event = AttentionEvent::confirmation_request("s1", "claude", "Overwrite?");
        let view = build("Phone", true, vec![session_with(event)]);

        let actions: Vec<&str> = view.requests[0].actions.iter().map(|a| a.action).collect();
        assert_eq!(actions, vec!["confirm", "reject"]);
    }

    /// A completion toast has only `OpenSession`, so there is nothing to
    /// answer and it should not become a card with no buttons.
    #[test]
    fn events_with_nothing_to_answer_are_dropped() {
        let event = AttentionEvent::completed("s1", "claude", "Finished");
        let view = build("Phone", true, vec![session_with(event)]);

        assert!(view.requests.is_empty());
        // The session is still listed, so the phone can see it exists.
        assert_eq!(view.sessions.len(), 1);
    }

    #[test]
    fn the_process_id_never_reaches_the_phone() {
        let mut event = AttentionEvent::permission_request("s1", "claude", "Run", None);
        event.process_id = Some(4242);
        let view = build("Phone", true, vec![session_with(event)]);

        let json = serde_json::to_string(&view).expect("serialises");
        assert!(!json.contains("4242"));
        assert!(!json.contains("process_id"));
    }

    #[test]
    fn the_longest_wait_comes_first() {
        let old = AttentionEvent::permission_request("s1", "claude", "Older", None);
        let mut newer = AttentionEvent::permission_request("s2", "claude", "Newer", None);
        newer.timestamp = chrono::Utc::now();
        let mut older = old;
        older.timestamp = chrono::Utc::now() - chrono::Duration::seconds(120);

        let view = build("Phone", true, vec![session_with(older), session_with(newer)]);
        assert_eq!(view.requests[0].title, "Older");
    }
}
