//! One-time pairing codes.
//!
//! The QR in the dashboard carries a code, not a device token. The phone spends
//! the code once, the server hands back a token in a cookie, and the code is
//! burned. That way the long-lived secret never appears in a URL — not in the
//! QR, not in browser history, not in a screenshot of the dashboard someone
//! left in a screen recording.
//!
//! Codes live in memory only. Losing them on restart is correct: an unspent
//! code is a door left open, and a restart should close it.

use chrono::{DateTime, Duration, Utc};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

/// How long a code stays spendable.
///
/// Long enough to find your phone and open the camera, short enough that a code
/// left on screen while you go to lunch has expired by the time you are back.
pub const TTL_SECONDS: i64 = 300;

#[derive(Debug, Clone)]
struct Outstanding {
    code: String,
    expires_at: DateTime<Utc>,
}

/// The single pairing code that may be outstanding at any moment.
///
/// One at a time on purpose: pressing "Pair a device" again should mean "that
/// last QR is no longer valid", which is what someone expects when they close
/// the panel and reopen it.
#[derive(Debug, Clone, Default)]
pub struct Pairing {
    current: Arc<Mutex<Option<Outstanding>>>,
}

/// A code that has been issued and not yet spent.
#[derive(Debug, Clone)]
pub struct Issued {
    pub code: String,
    pub expires_at: DateTime<Utc>,
}

impl Pairing {
    pub fn new() -> Self {
        Self::default()
    }

    /// Issue a fresh code, invalidating any previous one.
    pub async fn issue(&self) -> Issued {
        let issued = Outstanding {
            code: crate::store::new_secret(8),
            expires_at: Utc::now() + Duration::seconds(TTL_SECONDS),
        };

        *self.current.lock().await = Some(issued.clone());
        info!(expires_at = %issued.expires_at, "Issued a pairing code");

        Issued {
            code: issued.code,
            expires_at: issued.expires_at,
        }
    }

    /// The outstanding code, if there is one and it has not expired.
    pub async fn outstanding(&self) -> Option<Issued> {
        let guard = self.current.lock().await;
        let held = guard.as_ref()?;
        if held.expires_at <= Utc::now() {
            return None;
        }
        Some(Issued {
            code: held.code.clone(),
            expires_at: held.expires_at,
        })
    }

    /// Spend a code. True only the first time, and only before it expires.
    pub async fn redeem(&self, offered: &str) -> bool {
        let mut guard = self.current.lock().await;

        let Some(held) = guard.as_ref() else {
            return false;
        };
        if held.expires_at <= Utc::now() {
            *guard = None;
            return false;
        }
        if !secret_eq(&held.code, offered) {
            return false;
        }

        // Burn it before returning, so two phones racing on the same QR cannot
        // both pair.
        *guard = None;
        true
    }

    /// Withdraw the outstanding code without spending it.
    pub async fn cancel(&self) {
        *self.current.lock().await = None;
    }
}

/// Compare two secrets without leaking where they first differ.
///
/// A timing attack across a LAN against a 16-character code is not a realistic
/// threat, but the alternative costs nothing and this is the file where someone
/// will look to check.
pub fn secret_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_comparison_matches_equality() {
        assert!(secret_eq("abc123", "abc123"));
        assert!(!secret_eq("abc123", "abc124"));
        assert!(!secret_eq("abc123", "abc1234"));
        assert!(secret_eq("", ""));
    }

    #[tokio::test]
    async fn a_code_can_only_be_spent_once() {
        let pairing = Pairing::new();
        let issued = pairing.issue().await;

        assert!(pairing.redeem(&issued.code).await);
        assert!(!pairing.redeem(&issued.code).await);
    }

    #[tokio::test]
    async fn issuing_again_invalidates_the_previous_code() {
        let pairing = Pairing::new();
        let first = pairing.issue().await;
        let second = pairing.issue().await;

        assert!(!pairing.redeem(&first.code).await);
        assert!(pairing.redeem(&second.code).await);
    }

    #[tokio::test]
    async fn a_wrong_code_does_not_burn_the_real_one() {
        let pairing = Pairing::new();
        let issued = pairing.issue().await;

        assert!(!pairing.redeem("0000000000000000").await);
        assert!(pairing.redeem(&issued.code).await);
    }

    #[tokio::test]
    async fn nothing_is_outstanding_before_issuing_or_after_spending() {
        let pairing = Pairing::new();
        assert!(pairing.outstanding().await.is_none());

        let issued = pairing.issue().await;
        assert!(pairing.outstanding().await.is_some());

        pairing.redeem(&issued.code).await;
        assert!(pairing.outstanding().await.is_none());
    }

    #[tokio::test]
    async fn cancelling_withdraws_the_code() {
        let pairing = Pairing::new();
        let issued = pairing.issue().await;

        pairing.cancel().await;
        assert!(!pairing.redeem(&issued.code).await);
    }
}
