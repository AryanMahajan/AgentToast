//! Escalation logic — reminders for unresolved attention events.

use std::time::Duration;
use serde::{Deserialize, Serialize};

/// Configuration for escalation/reminder behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationConfig {
    pub enabled: bool,
    pub reminder_intervals: Vec<Duration>,
    pub max_reminders: Option<u32>,
    pub sound_on_reminder: bool,
}

impl Default for EscalationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            reminder_intervals: vec![
                Duration::from_secs(120),  // 2 minutes
                Duration::from_secs(300),  // 5 minutes
                Duration::from_secs(600),  // 10 minutes
            ],
            max_reminders: None,
            sound_on_reminder: true,
        }
    }
}

impl EscalationConfig {
    pub fn interval_for_reminder(&self, index: usize) -> Duration {
        if self.reminder_intervals.is_empty() {
            return Duration::from_secs(300); // fallback: 5 minutes
        }
        self.reminder_intervals
            .get(index)
            .copied()
            .unwrap_or_else(|| *self.reminder_intervals.last().unwrap())
    }

    pub fn should_remind(&self, reminders_sent: u32) -> bool {
        if !self.enabled {
            return false;
        }
        match self.max_reminders {
            Some(max) => reminders_sent < max,
            None => true,
        }
    }
}
