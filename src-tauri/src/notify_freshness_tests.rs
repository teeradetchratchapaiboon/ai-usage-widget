//! Freshness gating on quota notifications.
//!
//! A toast asserts something about right now. A reading that cannot support
//! that claim must stay silent — most importantly at startup, where a snapshot
//! restored from disk would otherwise fire immediately no matter how old it is.

use super::*;
use crate::freshness::Freshness;

/// A reading at `pct` with the given freshness and age.
fn reading(freshness: Freshness, used_pct: f64, age_secs: Option<i64>) -> QuotaReading {
    QuotaReading {
        window: QuotaWindow::Weekly,
        used_pct,
        resets_at: None,
        estimated: false,
        freshness,
        age_secs,
    }
}

#[test]
fn test_a_fresh_reading_notifies_normally() {
    let mut engine = NotificationEngine::new();
    let out = engine.check_threshold(
        "codex",
        "Codex Desktop",
        reading(Freshness::Fresh, 95.0, Some(60)),
    );

    assert_eq!(out.len(), 1);
    assert_eq!(out[0].level, NotificationLevel::Critical);
    // Fresh data needs no age caveat
    assert!(!out[0].message.contains("Data is"), "{}", out[0].message);
}

#[test]
fn test_an_aging_reading_must_state_its_age() {
    // Warning someone off a quota without saying the number is hours old
    // invites them to act on it as though it were current.
    let mut engine = NotificationEngine::new();
    let out = engine.check_threshold(
        "codex",
        "Codex Desktop",
        reading(Freshness::Aging, 95.0, Some(2 * 3600 + 30 * 60)),
    );

    assert_eq!(out.len(), 1);
    assert!(
        out[0].message.contains("Data is 2 hrs 30 min old."),
        "{}",
        out[0].message
    );
}

#[test]
fn test_an_aging_reading_with_no_usable_age_still_notifies() {
    // The gate is freshness, not the presence of a formatted age string.
    let mut engine = NotificationEngine::new();
    let out = engine.check_threshold("codex", "Codex", reading(Freshness::Aging, 95.0, None));

    assert_eq!(out.len(), 1);
    assert!(!out[0].message.contains("Data is"), "{}", out[0].message);
}

#[test]
fn test_a_stale_reading_never_notifies() {
    let mut engine = NotificationEngine::new();
    let out = engine.check_threshold(
        "codex",
        "Codex Desktop",
        reading(Freshness::Stale, 100.0, Some(2 * 86_400)),
    );

    assert!(out.is_empty(), "stale data must not raise a toast");
}

#[test]
fn test_an_expired_reading_never_notifies() {
    let mut engine = NotificationEngine::new();
    let out = engine.check_threshold(
        "codex",
        "Codex",
        reading(Freshness::Expired, 100.0, Some(30)),
    );

    assert!(out.is_empty());
}

#[test]
fn test_an_unknown_age_never_notifies() {
    // Metadata persisted before observation timestamps existed.
    let mut engine = NotificationEngine::new();
    let out = engine.check_threshold("codex", "Codex", reading(Freshness::Unknown, 100.0, None));

    assert!(out.is_empty());
}

#[test]
fn test_a_restored_stale_snapshot_stays_silent_at_startup() {
    // The reported shape of the bug: the app restarts, primes a two-day-old
    // 100% off disk, and — before this guard — toasted "critical" instantly.
    let mut engine = NotificationEngine::new();

    for _ in 0..5 {
        let out = engine.check_threshold(
            "codex",
            "Codex Desktop",
            reading(Freshness::Stale, 100.0, Some(2 * 86_400)),
        );
        assert!(out.is_empty());
    }
}

#[test]
fn test_the_gate_does_not_consume_the_cooldown() {
    // A silenced stale check must not spend the budget that the first fresh
    // reading is entitled to, or a recovering provider would go unannounced.
    let mut engine = NotificationEngine::new();

    assert!(engine
        .check_threshold(
            "codex",
            "Codex",
            reading(Freshness::Stale, 95.0, Some(99_999))
        )
        .is_empty());

    let out = engine.check_threshold("codex", "Codex", reading(Freshness::Fresh, 95.0, Some(10)));
    assert_eq!(
        out.len(),
        1,
        "the fresh reading must still be allowed to fire"
    );
}

#[test]
fn test_per_window_cooldowns_survive_the_freshness_gate() {
    let mut engine = NotificationEngine::new();

    let fast = QuotaReading {
        window: QuotaWindow::FastHours,
        ..reading(Freshness::Fresh, 92.0, Some(10))
    };
    let weekly = reading(Freshness::Fresh, 95.0, Some(10));

    assert_eq!(engine.check_threshold("codex", "Codex", fast).len(), 1);
    assert_eq!(
        engine.check_threshold("codex", "Codex", weekly).len(),
        1,
        "the weekly window is rate-limited independently"
    );
    assert!(
        engine.check_threshold("codex", "Codex", weekly).is_empty(),
        "the weekly window is still individually rate-limited"
    );
}

#[test]
fn test_freshness_does_not_override_the_threshold() {
    // Fresh is permission to notify, not a reason to.
    let mut engine = NotificationEngine::new();
    let out = engine.check_threshold("codex", "Codex", reading(Freshness::Fresh, 10.0, Some(10)));

    assert!(out.is_empty());
}
