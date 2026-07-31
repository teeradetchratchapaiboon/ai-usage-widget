//! Property-based tests for version comparison logic used in the update checker.
//!
//! **Validates: Requirements 12.5**
//!
//! This integration test exercises the semver comparison logic independently
//! from the full Tauri application stack, avoiding WebView2/Windows DLL
//! dependencies that prevent the lib test binary from executing.

use proptest::prelude::*;

/// Compare two version strings and determine if an update is available.
/// Returns Some(latest) if latest > current, None otherwise.
///
/// This mirrors the logic in `commands::check_for_updates()`.
fn compare_versions(current: &str, latest: &str) -> Option<String> {
    let current_v = semver::Version::parse(current).ok()?;
    let latest_v = semver::Version::parse(latest).ok()?;
    if latest_v > current_v {
        Some(latest_v.to_string())
    } else {
        None
    }
}

/// Strategy to generate valid semver version components (0-99 range for practical testing)
fn version_component() -> impl Strategy<Value = u32> {
    0u32..100u32
}

/// Strategy to generate a valid semver version string "major.minor.patch"
fn semver_version() -> impl Strategy<Value = String> {
    (version_component(), version_component(), version_component())
        .prop_map(|(major, minor, patch)| format!("{}.{}.{}", major, minor, patch))
}

proptest! {
    #![proptest_config(proptest::test_runner::Config::with_cases(50))]

    /// Property 1: Reflexivity - Any version equals itself (not greater, not less).
    #[test]
    fn reflexivity(version in semver_version()) {
        // A version compared against itself should indicate no update available
        let result = compare_versions(&version, &version);
        prop_assert!(result.is_none(), "Version {} compared to itself should return None, got {:?}", version, result);
    }

    /// Property 2: Transitivity - If v1 > v2 and v2 > v3, then v1 > v3.
    #[test]
    fn transitivity(
        a in version_component(),
        base_minor in version_component(),
        base_patch in version_component(),
    ) {
        // Create three strictly ordered versions by using incrementing major components
        let base = a % 50;
        let v3 = format!("{}.{}.{}", base, base_minor, base_patch);
        let v2 = format!("{}.{}.{}", base + 1, base_minor, base_patch);
        let v1 = format!("{}.{}.{}", base + 2, base_minor, base_patch);

        // v1 > v2 (latest=v1, current=v2 -> update available)
        let r1 = compare_versions(&v2, &v1);
        prop_assert!(r1.is_some(), "Expected v1({}) > v2({})", v1, v2);

        // v2 > v3 (latest=v2, current=v3 -> update available)
        let r2 = compare_versions(&v3, &v2);
        prop_assert!(r2.is_some(), "Expected v2({}) > v3({})", v2, v3);

        // v1 > v3 (latest=v1, current=v3 -> update available) - transitivity
        let r3 = compare_versions(&v3, &v1);
        prop_assert!(r3.is_some(), "Expected v1({}) > v3({}) by transitivity", v1, v3);
    }

    /// Property 3: Patch version bump detected - 0.1.0 → 0.1.1 should indicate an update is available.
    #[test]
    fn patch_bump_detected(
        major in version_component(),
        minor in version_component(),
        patch in 0u32..99u32,
    ) {
        let current = format!("{}.{}.{}", major, minor, patch);
        let latest = format!("{}.{}.{}", major, minor, patch + 1);
        let result = compare_versions(&current, &latest);
        prop_assert!(result.is_some(), "Patch bump {} -> {} should indicate update", current, latest);
        prop_assert_eq!(result.unwrap(), latest);
    }

    /// Property 4: Minor version bump detected - 0.1.0 → 0.2.0 should indicate an update is available.
    #[test]
    fn minor_bump_detected(
        major in version_component(),
        minor in 0u32..99u32,
        patch in version_component(),
    ) {
        let current = format!("{}.{}.{}", major, minor, patch);
        let latest = format!("{}.{}.{}", major, minor + 1, 0);
        let result = compare_versions(&current, &latest);
        prop_assert!(result.is_some(), "Minor bump {} -> {} should indicate update", current, latest);
    }

    /// Property 5: Major version bump detected - 0.1.0 → 1.0.0 should indicate an update is available.
    #[test]
    fn major_bump_detected(
        major in 0u32..99u32,
        minor in version_component(),
        patch in version_component(),
    ) {
        let current = format!("{}.{}.{}", major, minor, patch);
        let latest = format!("{}.{}.{}", major + 1, 0, 0);
        let result = compare_versions(&current, &latest);
        prop_assert!(result.is_some(), "Major bump {} -> {} should indicate update", current, latest);
    }

    /// Property 6: Same version no update - When current == latest, no update should be indicated.
    #[test]
    fn same_version_no_update(version in semver_version()) {
        let result = compare_versions(&version, &version);
        prop_assert!(result.is_none(), "Same version {} should not indicate update", version);
    }

    /// Property 7: Pre-release ordering - Pre-release versions (e.g., 1.0.0-alpha < 1.0.0) follow semver rules.
    #[test]
    fn prerelease_ordering(
        major in 1u32..50u32,
        minor in version_component(),
        patch in version_component(),
    ) {
        let prerelease = format!("{}.{}.{}-alpha", major, minor, patch);
        let release = format!("{}.{}.{}", major, minor, patch);

        // Pre-release < release per semver spec
        let result = compare_versions(&prerelease, &release);
        prop_assert!(result.is_some(), "Pre-release {} should be less than release {}", prerelease, release);

        // Release is NOT less than pre-release
        let result_reverse = compare_versions(&release, &prerelease);
        prop_assert!(result_reverse.is_none(), "Release {} should not indicate update to pre-release {}", release, prerelease);
    }
}
