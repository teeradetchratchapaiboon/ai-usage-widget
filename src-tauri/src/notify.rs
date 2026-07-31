use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Notification severity level based on quota threshold.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NotificationLevel {
    /// 75% threshold
    Warning,
    /// 90% threshold
    Critical,
}

/// A notification entry produced when a threshold is exceeded.
#[derive(Debug, Clone)]
pub struct NotificationEntry {
    pub level: NotificationLevel,
    pub provider_id: String,
    pub provider_name: String,
    pub current_pct: f64,
    pub threshold_pct: f64,
    pub message: String,
}

/// Engine that checks quota usage against thresholds and manages cooldowns.
///
/// Returns notification messages when thresholds are exceeded, subject to a
/// per-(provider, level) cooldown to avoid notification spam.
pub struct NotificationEngine {
    /// Tracks last notification time per (provider_id, level)
    cooldowns: HashMap<(String, NotificationLevel), Instant>,
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
    #[allow(clippy::collapsible_if)]
    pub fn check_threshold(
        &mut self,
        provider_id: &str,
        provider_name: &str,
        current_pct: f64,
    ) -> Vec<NotificationEntry> {
        let mut notifications = Vec::new();

        if current_pct >= self.critical_threshold {
            if self.should_notify(provider_id, &NotificationLevel::Critical) {
                let entry = NotificationEntry {
                    level: NotificationLevel::Critical,
                    provider_id: provider_id.to_string(),
                    provider_name: provider_name.to_string(),
                    current_pct,
                    threshold_pct: self.critical_threshold,
                    message: format!(
                        "{} quota usage is at {:.1}% (critical threshold: {:.0}%)",
                        provider_name, current_pct, self.critical_threshold
                    ),
                };
                self.record_cooldown(provider_id, NotificationLevel::Critical);
                notifications.push(entry);
            }
        } else if current_pct >= self.warning_threshold {
            if self.should_notify(provider_id, &NotificationLevel::Warning) {
                let entry = NotificationEntry {
                    level: NotificationLevel::Warning,
                    provider_id: provider_id.to_string(),
                    provider_name: provider_name.to_string(),
                    current_pct,
                    threshold_pct: self.warning_threshold,
                    message: format!(
                        "{} quota usage is at {:.1}% (warning threshold: {:.0}%)",
                        provider_name, current_pct, self.warning_threshold
                    ),
                };
                self.record_cooldown(provider_id, NotificationLevel::Warning);
                notifications.push(entry);
            }
        }

        notifications
    }

    /// Replace the thresholds, keeping existing cooldowns so a threshold
    /// change does not immediately re-notify providers that already warned.
    pub fn set_thresholds(&mut self, warning: f64, critical: f64) {
        self.warning_threshold = warning;
        self.critical_threshold = critical;
    }

    /// Reset cooldowns for a provider (called when usage resets).
    pub fn reset_provider(&mut self, provider_id: &str) {
        self.cooldowns
            .retain(|(id, _), _| id != provider_id);
    }

    /// Check if a notification should be sent (i.e., cooldown has expired).
    fn should_notify(&self, provider_id: &str, level: &NotificationLevel) -> bool {
        let key = (provider_id.to_string(), level.clone());
        match self.cooldowns.get(&key) {
            Some(last_sent) => last_sent.elapsed() >= self.cooldown_duration,
            None => true,
        }
    }

    /// Record that a notification was just sent.
    fn record_cooldown(&mut self, provider_id: &str, level: NotificationLevel) {
        let key = (provider_id.to_string(), level);
        self.cooldowns.insert(key, Instant::now());
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

    #[test]
    fn test_no_notification_below_warning_threshold() {
        let mut engine = NotificationEngine::new();
        let result = engine.check_threshold("claude", "Claude Desktop", 50.0);
        assert!(result.is_empty());
    }

    #[test]
    fn test_warning_notification_at_75_percent() {
        let mut engine = NotificationEngine::new();
        let result = engine.check_threshold("claude", "Claude Desktop", 75.0);
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
        let result = engine.check_threshold("claude", "Claude Desktop", 82.5);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].level, NotificationLevel::Warning);
        assert_eq!(result[0].current_pct, 82.5);
    }

    #[test]
    fn test_critical_notification_at_90_percent() {
        let mut engine = NotificationEngine::new();
        let result = engine.check_threshold("claude", "Claude Desktop", 90.0);
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
        let result = engine.check_threshold("codex", "Codex Desktop", 95.3);
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
        let result1 = engine.check_threshold("claude", "Claude Desktop", 80.0);
        assert_eq!(result1.len(), 1);

        // Second check within cooldown should not trigger
        let result2 = engine.check_threshold("claude", "Claude Desktop", 82.0);
        assert!(result2.is_empty());
    }

    #[test]
    fn test_cooldown_is_per_provider() {
        let mut engine = NotificationEngine::new();

        // Claude triggers warning
        let result1 = engine.check_threshold("claude", "Claude Desktop", 80.0);
        assert_eq!(result1.len(), 1);

        // Codex should still trigger (different provider)
        let result2 = engine.check_threshold("codex", "Codex Desktop", 78.0);
        assert_eq!(result2.len(), 1);
    }

    #[test]
    fn test_cooldown_is_per_level() {
        let mut engine = NotificationEngine::new();

        // Warning triggers
        let result1 = engine.check_threshold("claude", "Claude Desktop", 80.0);
        assert_eq!(result1.len(), 1);
        assert_eq!(result1[0].level, NotificationLevel::Warning);

        // Critical should still trigger (different level)
        let result2 = engine.check_threshold("claude", "Claude Desktop", 92.0);
        assert_eq!(result2.len(), 1);
        assert_eq!(result2[0].level, NotificationLevel::Critical);
    }

    #[test]
    fn test_reset_provider_clears_cooldowns() {
        let mut engine = NotificationEngine::new();

        // Trigger warning
        let result1 = engine.check_threshold("claude", "Claude Desktop", 80.0);
        assert_eq!(result1.len(), 1);

        // In cooldown - no notification
        let result2 = engine.check_threshold("claude", "Claude Desktop", 82.0);
        assert!(result2.is_empty());

        // Reset provider
        engine.reset_provider("claude");

        // Should trigger again after reset
        let result3 = engine.check_threshold("claude", "Claude Desktop", 83.0);
        assert_eq!(result3.len(), 1);
    }

    #[test]
    fn test_reset_provider_only_affects_target_provider() {
        let mut engine = NotificationEngine::new();

        // Both providers trigger
        engine.check_threshold("claude", "Claude Desktop", 80.0);
        engine.check_threshold("codex", "Codex Desktop", 78.0);

        // Reset only claude
        engine.reset_provider("claude");

        // Claude should trigger again
        let result1 = engine.check_threshold("claude", "Claude Desktop", 81.0);
        assert_eq!(result1.len(), 1);

        // Codex should still be in cooldown
        let result2 = engine.check_threshold("codex", "Codex Desktop", 79.0);
        assert!(result2.is_empty());
    }

    #[test]
    fn test_custom_thresholds() {
        let mut engine = NotificationEngine::with_thresholds(50.0, 80.0);

        // Below custom warning (50%)
        let result1 = engine.check_threshold("claude", "Claude Desktop", 40.0);
        assert!(result1.is_empty());

        // At custom warning (50%)
        let result2 = engine.check_threshold("claude", "Claude Desktop", 50.0);
        assert_eq!(result2.len(), 1);
        assert_eq!(result2[0].level, NotificationLevel::Warning);

        // At custom critical (80%) - different provider to avoid cooldown
        let result3 = engine.check_threshold("codex", "Codex Desktop", 80.0);
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
        let result1 = engine.check_threshold("claude", "Claude Desktop", 80.0);
        assert_eq!(result1.len(), 1);

        // Wait for cooldown to expire
        std::thread::sleep(Duration::from_millis(5));

        // Should trigger again after cooldown expires
        let result2 = engine.check_threshold("claude", "Claude Desktop", 81.0);
        assert_eq!(result2.len(), 1);
    }

    #[test]
    fn test_notification_message_contains_provider_and_percentage() {
        let mut engine = NotificationEngine::new();
        let result = engine.check_threshold("claude", "Claude Desktop", 77.5);
        assert_eq!(result.len(), 1);
        assert!(result[0].message.contains("Claude Desktop"));
        assert!(result[0].message.contains("77.5%"));
    }

    #[test]
    fn test_exactly_at_boundary_values() {
        let mut engine = NotificationEngine::new();

        // Exactly at 74.9% - no notification
        let result = engine.check_threshold("p1", "Provider 1", 74.9);
        assert!(result.is_empty());

        // Exactly at 75.0% - warning
        let result = engine.check_threshold("p1", "Provider 1", 75.0);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].level, NotificationLevel::Warning);

        // Exactly at 89.9% - warning (different provider to avoid cooldown)
        let result = engine.check_threshold("p2", "Provider 2", 89.9);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].level, NotificationLevel::Warning);

        // Exactly at 90.0% - critical
        let result = engine.check_threshold("p3", "Provider 3", 90.0);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].level, NotificationLevel::Critical);
    }
}

/// Property-based tests for notification threshold accuracy.
///
/// **Validates: Requirements 10.1, 10.2, 10.3**
#[cfg(test)]
mod prop_tests_notification {
    use super::*;
    use proptest::prelude::*;
    use proptest::test_runner::Config;

    /// Strategy for generating valid percentage values (0.0..=100.0)
    fn pct_strategy() -> impl Strategy<Value = f64> {
        (0u32..=10000u32).prop_map(|v| v as f64 / 100.0)
    }

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
            let result = engine.check_threshold(&provider_id, "Test Provider", pct);

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
            let result = engine.check_threshold(&provider_id, "Test Provider", pct);

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
            let result = engine.check_threshold(&provider_id, "Test Provider", pct);

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
            let result1 = engine.check_threshold(&provider_id, "Test Provider", pct);
            prop_assert_eq!(result1.len(), 1, "First check should produce notification");

            // Second check immediately after (within cooldown) should NOT produce notification
            let result2 = engine.check_threshold(&provider_id, "Test Provider", pct);
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
            let result_a = engine.check_threshold(&provider_a, "Provider A", pct);
            prop_assert_eq!(result_a.len(), 1, "Provider A should trigger");

            // Provider B should independently trigger (not affected by A's cooldown)
            let result_b = engine.check_threshold(&provider_b, "Provider B", pct);
            prop_assert_eq!(result_b.len(), 1, "Provider B should trigger independently of A");

            // Provider A is now in cooldown
            let result_a2 = engine.check_threshold(&provider_a, "Provider A", pct);
            prop_assert!(result_a2.is_empty(), "Provider A should be in cooldown");

            // Provider B is also in cooldown independently
            let result_b2 = engine.check_threshold(&provider_b, "Provider B", pct);
            prop_assert!(result_b2.is_empty(), "Provider B should be in cooldown");
        }
    }
}
