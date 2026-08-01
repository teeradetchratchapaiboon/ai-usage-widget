//! How much a quota reading can still be trusted.
//!
//! Quota percentages are not polled from an API — they are whatever the vendor's
//! own application last wrote to disk. Codex only records a `rate_limits` payload
//! when it actually talks to the service, so a machine that has not run Codex for
//! two days holds a two-day-old percentage that is still perfectly well-formed.
//! Rendering that number the same way as one written a minute ago is the bug this
//! module exists to prevent.
//!
//! Every threshold lives here so the policy can be retuned in one place.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// At or below this age a reading is treated as current.
///
/// The collector runs every 30 s, so anything materially older than a few
/// minutes means the source itself has not produced a new record.
pub const FRESH_MAX_AGE: Duration = Duration::minutes(15);

/// Above [`FRESH_MAX_AGE`] and at or below this, a reading is still usable but
/// must be shown with its age. Six hours is a little longer than Codex's own
/// five-hour window, so a quiet morning does not immediately read as broken.
pub const AGING_MAX_AGE: Duration = Duration::hours(6);

/// How far a quota reading is from being current.
///
/// Serialised in lowercase (`"fresh"`, `"aging"`, …) for the IPC boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Freshness {
    /// Written within [`FRESH_MAX_AGE`]. Safe to present as the current value.
    Fresh,
    /// Older than [`FRESH_MAX_AGE`] but within [`AGING_MAX_AGE`]. Usable, but
    /// the age has to be shown alongside it.
    Aging,
    /// Older than [`AGING_MAX_AGE`]. Present only as a last-known value.
    Stale,
    /// The window this reading describes has already rolled over, so the
    /// percentage is known to be wrong rather than merely old.
    Expired,
    /// No usable observation timestamp — typically metadata persisted before
    /// this field existed. The age is unknown, which is not the same as new.
    Unknown,
}

impl Freshness {
    /// Whether a threshold notification may be raised for this reading.
    ///
    /// Stale, expired and unknown readings are silent: a toast asserts
    /// something about right now, and none of them can support that claim.
    pub fn may_notify(&self) -> bool {
        matches!(self, Freshness::Fresh | Freshness::Aging)
    }

    /// Whether the message must carry the age of the data.
    pub fn requires_age_disclosure(&self) -> bool {
        matches!(self, Freshness::Aging)
    }

    /// The lowercase wire form, matching the serde representation.
    pub fn as_str(&self) -> &'static str {
        match self {
            Freshness::Fresh => "fresh",
            Freshness::Aging => "aging",
            Freshness::Stale => "stale",
            Freshness::Expired => "expired",
            Freshness::Unknown => "unknown",
        }
    }
}

/// One quota window's reading, with everything needed to judge it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QuotaTiming {
    /// When the source record that supplied the percentage was written.
    ///
    /// Never the collection time, the file mtime, or "now" — an invented
    /// timestamp would make every stale reading look fresh.
    pub observed_at: Option<DateTime<Utc>>,
    /// When this window rolls over, if the provider states or implies it.
    pub resets_at: Option<DateTime<Utc>>,
}

impl QuotaTiming {
    /// Classify this reading against `now`.
    ///
    /// `now` is a parameter rather than a call to [`Utc::now`] so tests can pin
    /// the clock instead of asserting against a moving target.
    ///
    /// Expiry is checked before age: once the window has rolled over, how
    /// recently the old percentage was written stops mattering — it describes
    /// a window that no longer exists.
    pub fn freshness(&self, now: DateTime<Utc>) -> Freshness {
        if let Some(resets_at) = self.resets_at {
            if resets_at <= now {
                return Freshness::Expired;
            }
        }

        let Some(age) = self.age(now) else {
            return Freshness::Unknown;
        };

        if age <= FRESH_MAX_AGE {
            Freshness::Fresh
        } else if age <= AGING_MAX_AGE {
            Freshness::Aging
        } else {
            Freshness::Stale
        }
    }

    /// Age of the reading at `now`.
    ///
    /// A timestamp in the future is treated as unusable rather than as a
    /// negative age: it means a clock changed under us, and no honest age can
    /// be reported from it.
    pub fn age(&self, now: DateTime<Utc>) -> Option<Duration> {
        let observed_at = self.observed_at?;
        let age = now - observed_at;
        (age >= Duration::zero()).then_some(age)
    }

    /// Age in whole seconds, for the IPC boundary.
    pub fn age_seconds(&self, now: DateTime<Utc>) -> Option<i64> {
        self.age(now).map(|age| age.num_seconds())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed clock: every expectation below is exact, not "about".
    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-08-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn timing(observed_ago: Option<Duration>, resets_in: Option<Duration>) -> QuotaTiming {
        QuotaTiming {
            observed_at: observed_ago.map(|d| now() - d),
            resets_at: resets_in.map(|d| now() + d),
        }
    }

    #[test]
    fn test_a_just_written_reading_is_fresh() {
        let t = timing(Some(Duration::minutes(1)), Some(Duration::hours(3)));
        assert_eq!(t.freshness(now()), Freshness::Fresh);
        assert!(t.freshness(now()).may_notify());
    }

    #[test]
    fn test_the_fresh_boundary_is_inclusive() {
        let t = timing(Some(FRESH_MAX_AGE), Some(Duration::hours(3)));
        assert_eq!(t.freshness(now()), Freshness::Fresh);

        let t = timing(
            Some(FRESH_MAX_AGE + Duration::seconds(1)),
            Some(Duration::hours(3)),
        );
        assert_eq!(t.freshness(now()), Freshness::Aging);
    }

    #[test]
    fn test_the_aging_boundary_is_inclusive() {
        let t = timing(Some(AGING_MAX_AGE), Some(Duration::hours(30)));
        assert_eq!(t.freshness(now()), Freshness::Aging);

        let t = timing(
            Some(AGING_MAX_AGE + Duration::seconds(1)),
            Some(Duration::hours(30)),
        );
        assert_eq!(t.freshness(now()), Freshness::Stale);
    }

    #[test]
    fn test_a_two_day_old_reading_is_stale_and_silent() {
        // The reported case: Codex had not run for two days, so its last
        // percentage was still on disk and was being shown as current.
        let t = timing(Some(Duration::days(2)), Some(Duration::days(3)));

        assert_eq!(t.freshness(now()), Freshness::Stale);
        assert!(!t.freshness(now()).may_notify());
        assert_eq!(t.age_seconds(now()), Some(2 * 86_400));
    }

    #[test]
    fn test_a_window_that_already_rolled_over_is_expired() {
        // Even a reading written seconds ago describes a window that has ended
        let t = timing(Some(Duration::seconds(30)), Some(Duration::seconds(-1)));
        assert_eq!(t.freshness(now()), Freshness::Expired);
        assert!(!t.freshness(now()).may_notify());
    }

    #[test]
    fn test_expiry_at_exactly_the_reset_instant() {
        let t = timing(Some(Duration::minutes(1)), Some(Duration::zero()));
        assert_eq!(t.freshness(now()), Freshness::Expired);
    }

    #[test]
    fn test_metadata_without_an_observation_time_is_unknown_not_fresh() {
        // Config written before this field existed deserialises to None. The
        // whole point is that this must not read as "just updated".
        let t = timing(None, Some(Duration::hours(3)));

        assert_eq!(t.freshness(now()), Freshness::Unknown);
        assert!(!t.freshness(now()).may_notify());
        assert_eq!(t.age_seconds(now()), None);
    }

    #[test]
    fn test_nothing_known_at_all_is_unknown() {
        assert_eq!(QuotaTiming::default().freshness(now()), Freshness::Unknown);
    }

    #[test]
    fn test_a_timestamp_from_the_future_is_unusable_rather_than_negative() {
        // A clock change should not produce a negative age or a "fresh" verdict
        let t = timing(Some(Duration::hours(-2)), Some(Duration::hours(9)));
        assert_eq!(t.freshness(now()), Freshness::Unknown);
        assert_eq!(t.age_seconds(now()), None);
    }

    #[test]
    fn test_only_aging_has_to_disclose_its_age() {
        assert!(!Freshness::Fresh.requires_age_disclosure());
        assert!(Freshness::Aging.requires_age_disclosure());
        assert!(!Freshness::Stale.requires_age_disclosure());
    }

    #[test]
    fn test_wire_form_matches_serde() {
        for (value, expected) in [
            (Freshness::Fresh, "fresh"),
            (Freshness::Aging, "aging"),
            (Freshness::Stale, "stale"),
            (Freshness::Expired, "expired"),
            (Freshness::Unknown, "unknown"),
        ] {
            assert_eq!(value.as_str(), expected);
            assert_eq!(serde_json::to_string(&value).unwrap(), format!("\"{expected}\""));
        }
    }
}
