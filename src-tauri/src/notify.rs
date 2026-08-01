use std::collections::HashMap;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

/// Notification severity level based on quota threshold.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NotificationLevel {
    /// 75% threshold
    Warning,
    /// 90% threshold
    Critical,
}

/// Which metered window a reading belongs to.
///
/// The windows run out independently and the wait to recover differs by days,
/// so a notification that names only the provider leaves the reader unable to
/// tell whether to take a break or stop for the week.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuotaWindow {
    FastHours,
    Weekly,
    Excess,
}

impl QuotaWindow {
    /// Human-readable name for the notification body.
    pub fn label(&self) -> &'static str {
        match self {
            QuotaWindow::FastHours => "5-hour",
            QuotaWindow::Weekly => "weekly",
            QuotaWindow::Excess => "excess",
        }
    }
}

/// One window's reading, as handed to the threshold check.
#[derive(Debug, Clone, Copy)]
pub struct QuotaReading {
    pub window: QuotaWindow,
    /// Percentage consumed.
    pub used_pct: f64,
    /// When this window rolls over, when the provider says.
    pub resets_at: Option<DateTime<Utc>>,
    /// True when `resets_at` was derived rather than published.
    pub estimated: bool,
}

impl QuotaReading {
    /// A reading with no reset information, for providers that publish none.
    pub fn new(window: QuotaWindow, used_pct: f64) -> Self {
        Self {
            window,
            used_pct,
            resets_at: None,
            estimated: false,
        }
    }

    /// The trailing "Resets in ..." sentence, when a reset time is known.
    ///
    /// A notification that says a quota is gone without saying for how long
    /// tells the reader the half of the story they cannot act on.
    fn reset_sentence(&self) -> String {
        let Some(wait) = self.resets_at.and_then(format_wait) else {
            return String::new();
        };
        if self.estimated {
            format!(" Resets in about {}.", wait)
        } else {
            format!(" Resets in {}.", wait)
        }
    }
}

/// Render the wait until `until` as at most two units: "4 days 2 hrs".
///
/// Rounds the smaller unit rather than truncating, matching the widget's own
/// countdown: both describe the same reset, and a toast reading "3 days 23
/// hrs" next to a widget reading "4 days" makes both look wrong.
///
/// Returns None for a time already past — a stale reset timestamp is not a
/// wait, and Codex keeps publishing one long after its window rolled over.
fn format_wait(until: DateTime<Utc>) -> Option<String> {
    let seconds = (until - Utc::now()).num_seconds();
    if seconds <= 0 {
        return None;
    }

    let mut days = seconds / 86_400;
    let mut hours;
    let mut minutes = 0;

    if days > 0 {
        hours = (seconds % 86_400 + 1_800) / 3_600;
        if hours == 24 {
            days += 1;
            hours = 0;
        }
    } else {
        hours = seconds / 3_600;
        minutes = (seconds % 3_600 + 30) / 60;
        if minutes == 60 {
            hours += 1;
            minutes = 0;
        }
    }

    Some(if days > 0 && hours > 0 {
        format!("{} days {} hrs", days, hours)
    } else if days > 0 {
        format!("{} days", days)
    } else if hours > 0 && minutes > 0 {
        format!("{} hrs {} min", hours, minutes)
    } else if hours > 0 {
        format!("{} hrs", hours)
    } else {
        format!("{} min", minutes.max(1))
    })
}

/// A notification entry produced when a threshold is exceeded.
#[derive(Debug, Clone)]
pub struct NotificationEntry {
    pub level: NotificationLevel,
    pub provider_id: String,
    pub provider_name: String,
    pub window: QuotaWindow,
    pub current_pct: f64,
    pub threshold_pct: f64,
    pub message: String,
}

/// Engine that checks quota usage against thresholds and manages cooldowns.
///
/// Returns notification messages when thresholds are exceeded, subject to a
/// per-(provider, level) cooldown to avoid notification spam.
pub struct NotificationEngine {
    /// Tracks last notification time per (provider_id, window, level).
    ///
    /// The window belongs in the key: a five-hour warning must not silence
    /// the weekly one, which is the more consequential of the two.
    cooldowns: HashMap<(String, QuotaWindow, NotificationLevel), Instant>,
    /// Warning threshold percentage (default: 75.0)
    warning_threshold: f64,
    /// Critical threshold percentage (default: 90.0)
    critical_threshold: f64,
    /// Cooldown duration (default: 1 hour)
    cooldown_duration: Duration,
}

impl NotificationEngine {
    /// Create a new NotificationEngine with default thresholds (75% warning, 90% critical)
    /// and a 1-hour cooldown.
    pub fn new() -> Self {
        Self {
            cooldowns: HashMap::new(),
            warning_threshold: 75.0,
            critical_threshold: 90.0,
            cooldown_duration: Duration::from_secs(3600),
        }
    }

    /// Create a new NotificationEngine with custom thresholds and a 1-hour cooldown.
    pub fn with_thresholds(warning: f64, critical: f64) -> Self {
        Self {
            cooldowns: HashMap::new(),
            warning_threshold: warning,
            critical_threshold: critical,
            cooldown_duration: Duration::from_secs(3600),
        }
    }

    /// Check if a quota value triggers a notification.
    ///
    /// Returns a vec of notifications (may be empty if under threshold or in cooldown).
    /// - If `current_pct` >= critical threshold, a Critical notification is returned (if not in cooldown).
    /// - If `current_pct` >= warning threshold but < critical, a Warning notification is returned (if not in cooldown).
    pub fn check_threshold(
        &mut self,
        provider_id: &str,
        provider_name: &str,
        reading: QuotaReading,
    ) -> Vec<NotificationEntry> {
        let current_pct = reading.used_pct;

        let (level, threshold) = if current_pct >= self.critical_threshold {
            (NotificationLevel::Critical, self.critical_threshold)
        } else if current_pct >= self.warning_threshold {
            (NotificationLevel::Warning, self.warning_threshold)
        } else {
            return Vec::new();
        };

        if !self.should_notify(provider_id, reading.window, &level) {
            return Vec::new();
        }

        let entry = NotificationEntry {
            level: level.clone(),
            provider_id: provider_id.to_string(),
            provider_name: provider_name.to_string(),
            window: reading.window,
            current_pct,
            threshold_pct: threshold,
            message: format!(
                "{} {} quota: {:.0}% left ({:.1}% used, {} at {:.0}%).{}",
                provider_name,
                reading.window.label(),
                (100.0 - current_pct).max(0.0),
                current_pct,
                match level {
                    NotificationLevel::Critical => "critical",
                    NotificationLevel::Warning => "warning",
                },
                threshold,
                reading.reset_sentence(),
            ),
        };

        self.record_cooldown(provider_id, reading.window, level);
        vec![entry]
    }

    /// Replace the thresholds, keeping existing cooldowns so a threshold
    /// change does not immediately re-notify providers that already warned.
    pub fn set_thresholds(&mut self, warning: f64, critical: f64) {
        self.warning_threshold = warning;
        self.critical_threshold = critical;
    }

    /// Reset cooldowns for a provider (called when usage resets).
    pub fn reset_provider(&mut self, provider_id: &str) {
        self.cooldowns.retain(|(id, _, _), _| id != provider_id);
    }

    /// Check if a notification should be sent (i.e., cooldown has expired).
    fn should_notify(
        &self,
        provider_id: &str,
        window: QuotaWindow,
        level: &NotificationLevel,
    ) -> bool {
        let key = (provider_id.to_string(), window, level.clone());
        match self.cooldowns.get(&key) {
            Some(last_sent) => last_sent.elapsed() >= self.cooldown_duration,
            None => true,
        }
    }

    /// Record that a notification was just sent.
    fn record_cooldown(&mut self, provider_id: &str, window: QuotaWindow, level: NotificationLevel) {
        self.cooldowns
            .insert((provider_id.to_string(), window, level), Instant::now());
    }
}

impl Default for NotificationEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plain five-hour reading, for tests about thresholds and cooldowns
    /// rather than about window labelling.
    fn reading(used_pct: f64) -> QuotaReading {
        QuotaReading::new(QuotaWindow::FastHours, used_pct)
    }

    #[test]
    fn test_no_notification_below_warning_threshold() {
        let mut engine = NotificationEngine::new();
        let result = engine.check_threshold("claude", "Claude Desktop", reading(50.0));
        assert!(result.is_empty());
    }

    #[test]
    fn test_warning_notification_at_75_percent() {
        let mut engine = NotificationEngine::new();
        let result = engine.check_threshold("claude", "Claude Desktop", reading(75.0));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].level, NotificationLevel::Warning);
        assert_eq!(result[0].provider_id, "claude");
        assert_eq!(result[0].provider_name, "Claude Desktop");
        assert_eq!(result[0].current_pct, 75.0);
        assert_eq!(result[0].threshold_pct, 75.0);
        assert!(result[0].message.contains("Claude Desktop"));
        assert!(result[0].message.contains("75.0%"));
    }

    #[test]
    fn test_warning_notification_between_75_and_90() {
        let mut engine = NotificationEngine::new();
        let result = engine.check_threshold("claude", "Claude Desktop", reading(82.5));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].level, NotificationLevel::Warning);
        assert_eq!(result[0].current_pct, 82.5);
    }

    #[test]
    fn test_critical_notification_at_90_percent() {
        let mut engine = NotificationEngine::new();
        let result = engine.check_threshold("claude", "Claude Desktop", reading(90.0));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].level, NotificationLevel::Critical);
        assert_eq!(result[0].current_pct, 90.0);
        assert_eq!(result[0].threshold_pct, 90.0);
        assert!(result[0].message.contains("Claude Desktop"));
        assert!(result[0].message.contains("90.0%"));
    }

    #[test]
    fn test_critical_notification_above_90_percent() {
        let mut engine = NotificationEngine::new();
        let result = engine.check_threshold("codex", "Codex Desktop", reading(95.3));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].level, NotificationLevel::Critical);
        assert_eq!(result[0].provider_id, "codex");
        assert_eq!(result[0].provider_name, "Codex Desktop");
        assert_eq!(result[0].current_pct, 95.3);
    }

    #[test]
    fn test_cooldown_prevents_duplicate_notification() {
        let mut engine = NotificationEngine::new();

        // First check triggers notification
        let result1 = engine.check_threshold("claude", "Claude Desktop", reading(80.0));
        assert_eq!(result1.len(), 1);

        // Second check within cooldown should not trigger
        let result2 = engine.check_threshold("claude", "Claude Desktop", reading(82.0));
        assert!(result2.is_empty());
    }

    #[test]
    fn test_cooldown_is_per_provider() {
        let mut engine = NotificationEngine::new();

        // Claude triggers warning
        let result1 = engine.check_threshold("claude", "Claude Desktop", reading(80.0));
        assert_eq!(result1.len(), 1);

        // Codex should still trigger (different provider)
        let result2 = engine.check_threshold("codex", "Codex Desktop", reading(78.0));
        assert_eq!(result2.len(), 1);
    }

    #[test]
    fn test_cooldown_is_per_level() {
        let mut engine = NotificationEngine::new();

        // Warning triggers
        let result1 = engine.check_threshold("claude", "Claude Desktop", reading(80.0));
        assert_eq!(result1.len(), 1);
        assert_eq!(result1[0].level, NotificationLevel::Warning);

        // Critical should still trigger (different level)
        let result2 = engine.check_threshold("claude", "Claude Desktop", reading(92.0));
        assert_eq!(result2.len(), 1);
        assert_eq!(result2[0].level, NotificationLevel::Critical);
    }

    #[test]
    fn test_reset_provider_clears_cooldowns() {
        let mut engine = NotificationEngine::new();

        // Trigger warning
        let result1 = engine.check_threshold("claude", "Claude Desktop", reading(80.0));
        assert_eq!(result1.len(), 1);

        // In cooldown - no notification
        let result2 = engine.check_threshold("claude", "Claude Desktop", reading(82.0));
        assert!(result2.is_empty());

        // Reset provider
        engine.reset_provider("claude");

        // Should trigger again after reset
        let result3 = engine.check_threshold("claude", "Claude Desktop", reading(83.0));
        assert_eq!(result3.len(), 1);
    }

    #[test]
    fn test_reset_provider_only_affects_target_provider() {
        let mut engine = NotificationEngine::new();

        // Both providers trigger
        engine.check_threshold("claude", "Claude Desktop", reading(80.0));
        engine.check_threshold("codex", "Codex Desktop", reading(78.0));

        // Reset only claude
        engine.reset_provider("claude");

        // Claude should trigger again
        let result1 = engine.check_threshold("claude", "Claude Desktop", reading(81.0));
        assert_eq!(result1.len(), 1);

        // Codex should still be in cooldown
        let result2 = engine.check_threshold("codex", "Codex Desktop", reading(79.0));
        assert!(result2.is_empty());
    }

    #[test]
    fn test_custom_thresholds() {
        let mut engine = NotificationEngine::with_thresholds(50.0, 80.0);

        // Below custom warning (50%)
        let result1 = engine.check_threshold("claude", "Claude Desktop", reading(40.0));
        assert!(result1.is_empty());

        // At custom warning (50%)
        let result2 = engine.check_threshold("claude", "Claude Desktop", reading(50.0));
        assert_eq!(result2.len(), 1);
        assert_eq!(result2[0].level, NotificationLevel::Warning);

        // At custom critical (80%) - different provider to avoid cooldown
        let result3 = engine.check_threshold("codex", "Codex Desktop", reading(80.0));
        assert_eq!(result3.len(), 1);
        assert_eq!(result3[0].level, NotificationLevel::Critical);
    }

    #[test]
    fn test_cooldown_expiry() {
        // Use a very short cooldown to test expiry
        let mut engine = NotificationEngine {
            cooldowns: HashMap::new(),
            warning_threshold: 75.0,
            critical_threshold: 90.0,
            cooldown_duration: Duration::from_millis(1),
        };

        // First check triggers
        let result1 = engine.check_threshold("claude", "Claude Desktop", reading(80.0));
        assert_eq!(result1.len(), 1);

        // Wait for cooldown to expire
        std::thread::sleep(Duration::from_millis(5));

        // Should trigger again after cooldown expires
        let result2 = engine.check_threshold("claude", "Claude Desktop", reading(81.0));
        assert_eq!(result2.len(), 1);
    }

    #[test]
    fn test_notification_message_contains_provider_and_percentage() {
        let mut engine = NotificationEngine::new();
        let result = engine.check_threshold("claude", "Claude Desktop", reading(77.5));
        assert_eq!(result.len(), 1);
        assert!(result[0].message.contains("Claude Desktop"));
        assert!(result[0].message.contains("77.5%"));
    }

    #[test]
    fn test_exactly_at_boundary_values() {
        let mut engine = NotificationEngine::new();

        // Exactly at 74.9% - no notification
        let result = engine.check_threshold("p1", "Provider 1", reading(74.9));
        assert!(result.is_empty());

        // Exactly at 75.0% - warning
        let result = engine.check_threshold("p1", "Provider 1", reading(75.0));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].level, NotificationLevel::Warning);

        // Exactly at 89.9% - warning (different provider to avoid cooldown)
        let result = engine.check_threshold("p2", "Provider 2", reading(89.9));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].level, NotificationLevel::Warning);

        // Exactly at 90.0% - critical
        let result = engine.check_threshold("p3", "Provider 3", reading(90.0));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].level, NotificationLevel::Critical);
    }


    #[test]
    fn test_wait_rounds_the_smaller_unit_like_the_widget_does() {
        // Truncating turned a full week into "6 days 23 hrs" in the toast
        // while the widget beside it said 7 days.
        let wait = format_wait(Utc::now() + chrono::Duration::days(7)).unwrap();
        assert_eq!(wait, "7 days");

        // One second short of five hours: the minutes round up to 60 and have
        // to carry, or this reads "4 hrs 60 min"
        let wait = format_wait(Utc::now() + chrono::Duration::seconds(5 * 3600 - 1)).unwrap();
        assert_eq!(wait, "5 hrs");

        // Genuinely mid-unit values keep both parts
        let wait = format_wait(Utc::now() + chrono::Duration::minutes(299)).unwrap();
        assert_eq!(wait, "4 hrs 59 min");
    }

    #[test]
    fn test_a_past_deadline_is_not_a_wait() {
        assert_eq!(format_wait(Utc::now() - chrono::Duration::hours(6)), None);
    }
    #[test]
    fn test_message_names_the_window_that_ran_out() {
        // "Codex Desktop 100%" cannot be acted on: a spent five-hour window
        // means take a break, a spent weekly one means stop for days.
        let mut engine = NotificationEngine::new();
        let entry = engine
            .check_threshold(
                "codex",
                "Codex Desktop",
                QuotaReading::new(QuotaWindow::Weekly, 100.0),
            )
            .remove(0);

        assert!(entry.message.contains("weekly"), "{}", entry.message);
        assert!(!entry.message.contains("5-hour"), "{}", entry.message);
        assert_eq!(entry.window, QuotaWindow::Weekly);
    }

    #[test]
    fn test_message_states_how_long_the_wait_is() {
        let mut engine = NotificationEngine::new();
        let entry = engine
            .check_threshold(
                "codex",
                "Codex Desktop",
                QuotaReading {
                    window: QuotaWindow::Weekly,
                    used_pct: 100.0,
                    resets_at: Some(Utc::now() + chrono::Duration::hours(50)),
                    estimated: false,
                },
            )
            .remove(0);

        assert!(entry.message.contains("Resets in 2 days 2 hrs"), "{}", entry.message);
        // Published, so it must not be hedged
        assert!(!entry.message.contains("about"), "{}", entry.message);
    }

    #[test]
    fn test_a_reconstructed_reset_is_hedged() {
        let mut engine = NotificationEngine::new();
        let entry = engine
            .check_threshold(
                "claude",
                "Claude Desktop",
                QuotaReading {
                    window: QuotaWindow::FastHours,
                    used_pct: 95.0,
                    resets_at: Some(Utc::now() + chrono::Duration::hours(3)),
                    estimated: true,
                },
            )
            .remove(0);

        assert!(entry.message.contains("Resets in about"), "{}", entry.message);
    }

    #[test]
    fn test_a_stale_reset_time_is_left_out_rather_than_stated() {
        // Codex never retracts its five-hour timestamp, so an idle app holds
        // one hours past. "Resets in 0 min" would be a false promise.
        let mut engine = NotificationEngine::new();
        let entry = engine
            .check_threshold(
                "codex",
                "Codex Desktop",
                QuotaReading {
                    window: QuotaWindow::FastHours,
                    used_pct: 100.0,
                    resets_at: Some(Utc::now() - chrono::Duration::hours(6)),
                    estimated: false,
                },
            )
            .remove(0);

        assert!(!entry.message.contains("Resets"), "{}", entry.message);
    }

    #[test]
    fn test_one_window_in_cooldown_does_not_silence_the_other() {
        // The whole point of splitting the windows: burning through the
        // five-hour quota must not hide the weekly limit running out an hour
        // later, which is the one that costs the rest of the week.
        let mut engine = NotificationEngine::new();

        let fast = engine.check_threshold(
            "codex",
            "Codex Desktop",
            QuotaReading::new(QuotaWindow::FastHours, 92.0),
        );
        assert_eq!(fast.len(), 1);

        let weekly = engine.check_threshold(
            "codex",
            "Codex Desktop",
            QuotaReading::new(QuotaWindow::Weekly, 95.0),
        );
        assert_eq!(weekly.len(), 1, "weekly must still be allowed to fire");
        assert_eq!(weekly[0].window, QuotaWindow::Weekly);

        // Each window is still individually rate-limited
        assert!(engine
            .check_threshold(
                "codex",
                "Codex Desktop",
                QuotaReading::new(QuotaWindow::Weekly, 96.0)
            )
            .is_empty());
    }

    #[test]
    fn test_reset_provider_clears_every_window() {
        let mut engine = NotificationEngine::new();
        engine.check_threshold("codex", "Codex", QuotaReading::new(QuotaWindow::FastHours, 92.0));
        engine.check_threshold("codex", "Codex", QuotaReading::new(QuotaWindow::Weekly, 92.0));

        engine.reset_provider("codex");

        assert_eq!(
            engine
                .check_threshold("codex", "Codex", QuotaReading::new(QuotaWindow::FastHours, 92.0))
                .len(),
            1
        );
        assert_eq!(
            engine
                .check_threshold("codex", "Codex", QuotaReading::new(QuotaWindow::Weekly, 92.0))
                .len(),
            1
        );
    }

}

/// Property-based tests for notification threshold accuracy.
/// **Validates: Requirements 10.1, 10.2, 10.3**
#[cfg(test)]
mod prop_tests_notification {
    use super::*;
    use proptest::prelude::*;
    use proptest::test_runner::Config;

    /// A plain five-hour reading, for tests about thresholds and cooldowns
    /// rather than about window labelling.
    fn reading(used_pct: f64) -> QuotaReading {
        QuotaReading::new(QuotaWindow::FastHours, used_pct)
    }

    /// Strategy for generating valid percentage values (0.0..=100.0)
    /// Strategy for generating provider IDs
    fn provider_id_strategy() -> impl Strategy<Value = String> {
        prop::string::string_regex("[a-z][a-z0-9_]{1,10}")
            .unwrap()
    }

    proptest! {
        #![proptest_config(Config::with_cases(50))]

        /// Property 1: When quota usage >= 75% but < 90%, a warning notification is generated.
        /// Validates: Requirement 10.1
        #[test]
        fn prop_warning_threshold_triggers(
            pct in 75.0f64..90.0f64,
            provider_id in provider_id_strategy(),
        ) {
            let mut engine = NotificationEngine::new();
            let result = engine.check_threshold(&provider_id, "Test Provider", reading(pct));

            prop_assert_eq!(result.len(), 1, "Expected exactly 1 notification for {:.2}%", pct);
            prop_assert_eq!(&result[0].level, &NotificationLevel::Warning);
            prop_assert_eq!(&result[0].provider_id, &provider_id);
            prop_assert!((result[0].current_pct - pct).abs() < f64::EPSILON);
            prop_assert_eq!(result[0].threshold_pct, 75.0);
        }

        /// Property 2: When quota usage >= 90%, a critical notification is generated.
        /// Validates: Requirement 10.2
        #[test]
        fn prop_critical_threshold_triggers(
            pct in 90.0f64..=100.0f64,
            provider_id in provider_id_strategy(),
        ) {
            let mut engine = NotificationEngine::new();
            let result = engine.check_threshold(&provider_id, "Test Provider", reading(pct));

            prop_assert_eq!(result.len(), 1, "Expected exactly 1 notification for {:.2}%", pct);
            prop_assert_eq!(&result[0].level, &NotificationLevel::Critical);
            prop_assert_eq!(&result[0].provider_id, &provider_id);
            prop_assert!((result[0].current_pct - pct).abs() < f64::EPSILON);
            prop_assert_eq!(result[0].threshold_pct, 90.0);
        }

        /// Property 3: When quota usage < 75%, no notification is generated.
        #[test]
        fn prop_below_threshold_no_notification(
            pct in 0.0f64..75.0f64,
            provider_id in provider_id_strategy(),
        ) {
            let mut engine = NotificationEngine::new();
            let result = engine.check_threshold(&provider_id, "Test Provider", reading(pct));

            prop_assert!(result.is_empty(), "Expected no notification for {:.2}% but got {}", pct, result.len());
        }

        /// Property 4: Within cooldown period, the same threshold notification for the same
        /// provider is NOT repeated.
        /// Validates: Requirement 10.3
        #[test]
        fn prop_cooldown_prevents_repeats(
            pct in 75.0f64..=100.0f64,
            provider_id in provider_id_strategy(),
        ) {
            let mut engine = NotificationEngine::new();

            // First check should produce a notification
            let result1 = engine.check_threshold(&provider_id, "Test Provider", reading(pct));
            prop_assert_eq!(result1.len(), 1, "First check should produce notification");

            // Second check immediately after (within cooldown) should NOT produce notification
            let result2 = engine.check_threshold(&provider_id, "Test Provider", reading(pct));
            prop_assert!(result2.is_empty(), "Second check within cooldown should produce no notification");
        }

        /// Property 5: Cooldowns are per-provider. Triggering for provider A does not
        /// prevent provider B from triggering.
        #[test]
        fn prop_different_providers_independent(
            pct in 75.0f64..=100.0f64,
            provider_a in provider_id_strategy(),
            provider_b in provider_id_strategy(),
        ) {
            // Ensure distinct provider IDs
            prop_assume!(provider_a != provider_b);

            let mut engine = NotificationEngine::new();

            // Provider A triggers
            let result_a = engine.check_threshold(&provider_a, "Provider A", reading(pct));
            prop_assert_eq!(result_a.len(), 1, "Provider A should trigger");

            // Provider B should independently trigger (not affected by A's cooldown)
            let result_b = engine.check_threshold(&provider_b, "Provider B", reading(pct));
            prop_assert_eq!(result_b.len(), 1, "Provider B should trigger independently of A");

            // Provider A is now in cooldown
            let result_a2 = engine.check_threshold(&provider_a, "Provider A", reading(pct));
            prop_assert!(result_a2.is_empty(), "Provider A should be in cooldown");

            // Provider B is also in cooldown independently
            let result_b2 = engine.check_threshold(&provider_b, "Provider B", reading(pct));
            prop_assert!(result_b2.is_empty(), "Provider B should be in cooldown");
        }
    }
}
