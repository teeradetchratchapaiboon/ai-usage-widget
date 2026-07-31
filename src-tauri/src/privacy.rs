//! Privacy hashing utilities for anonymizing sensitive identifiers before storage.
//!
//! All functions use SHA-256 and return 64-character lowercase hexadecimal strings.
//! These are one-way hashes — the original values cannot be recovered.
//!
//! Important: This module does NOT store any prompts, responses, or conversation text.

use sha2::{Digest, Sha256};

/// General-purpose SHA-256 hash function.
///
/// Returns a 64-character lowercase hexadecimal string.
/// Deterministic: same input always produces same output.
pub fn hash_string(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    format!("{:064x}", result)
}

/// Hash a file or project path using SHA-256.
///
/// Used to anonymize project paths before storage (Requirement 11.1).
pub fn hash_path(path: &str) -> String {
    hash_string(path)
}

/// Hash a session ID using SHA-256.
///
/// Used to anonymize session identifiers before storage (Requirement 11.2).
pub fn hash_session_id(session_id: &str) -> String {
    hash_string(session_id)
}

/// Hash an organization ID using SHA-256.
///
/// Used to anonymize organization identifiers before storage (Requirement 11.3).
pub fn hash_org_id(org_id: &str) -> String {
    hash_string(org_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_string_output_length_is_64() {
        let result = hash_string("hello world");
        assert_eq!(result.len(), 64);
    }

    #[test]
    fn hash_string_is_deterministic() {
        let a = hash_string("test input");
        let b = hash_string("test input");
        assert_eq!(a, b);
    }

    #[test]
    fn hash_string_differs_from_input() {
        let input = "my-secret-path";
        let result = hash_string(input);
        assert_ne!(result, input);
    }

    #[test]
    fn hash_string_different_inputs_produce_different_outputs() {
        let a = hash_string("input_a");
        let b = hash_string("input_b");
        assert_ne!(a, b);
    }

    #[test]
    fn hash_string_is_lowercase_hex() {
        let result = hash_string("anything");
        assert!(result.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn hash_path_output_length_is_64() {
        let result = hash_path("C:\\Users\\dev\\project");
        assert_eq!(result.len(), 64);
    }

    #[test]
    fn hash_path_is_deterministic() {
        let a = hash_path("/home/user/project");
        let b = hash_path("/home/user/project");
        assert_eq!(a, b);
    }

    #[test]
    fn hash_path_differs_from_input() {
        let input = "/home/user/project";
        let result = hash_path(input);
        assert_ne!(result, input);
    }

    #[test]
    fn hash_session_id_output_length_is_64() {
        let result = hash_session_id("sess_abc123");
        assert_eq!(result.len(), 64);
    }

    #[test]
    fn hash_session_id_is_deterministic() {
        let a = hash_session_id("session-xyz");
        let b = hash_session_id("session-xyz");
        assert_eq!(a, b);
    }

    #[test]
    fn hash_session_id_differs_from_input() {
        let input = "session-xyz";
        let result = hash_session_id(input);
        assert_ne!(result, input);
    }

    #[test]
    fn hash_org_id_output_length_is_64() {
        let result = hash_org_id("org_12345");
        assert_eq!(result.len(), 64);
    }

    #[test]
    fn hash_org_id_is_deterministic() {
        let a = hash_org_id("org-abc");
        let b = hash_org_id("org-abc");
        assert_eq!(a, b);
    }

    #[test]
    fn hash_org_id_differs_from_input() {
        let input = "org-abc";
        let result = hash_org_id(input);
        assert_ne!(result, input);
    }

    #[test]
    fn different_function_same_input_same_output() {
        // All functions are SHA-256 wrappers, so same input should produce same hash
        let input = "same-value";
        assert_eq!(hash_path(input), hash_session_id(input));
        assert_eq!(hash_session_id(input), hash_org_id(input));
        assert_eq!(hash_org_id(input), hash_string(input));
    }

    #[test]
    fn empty_string_produces_valid_hash() {
        let result = hash_string("");
        assert_eq!(result.len(), 64);
        // SHA-256 of empty string is a known value
        assert_eq!(
            result,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}

/// Property-based tests for privacy hashing utilities.
///
/// **Validates: Requirements 11.1, 11.2, 11.3**
#[cfg(test)]
mod prop_tests_privacy {
    use super::*;
    use proptest::prelude::*;
    use proptest::test_runner::Config;

    proptest! {
        #![proptest_config(Config::with_cases(50))]

        /// Property: Determinism - same input always produces the same hash.
        #[test]
        fn determinism(input in "\\PC{1,200}") {
            let hash1 = hash_string(&input);
            let hash2 = hash_string(&input);
            prop_assert_eq!(&hash1, &hash2, "hash_string must be deterministic");

            // Also verify the wrapper functions are deterministic
            let path1 = hash_path(&input);
            let path2 = hash_path(&input);
            prop_assert_eq!(&path1, &path2, "hash_path must be deterministic");

            let session1 = hash_session_id(&input);
            let session2 = hash_session_id(&input);
            prop_assert_eq!(&session1, &session2, "hash_session_id must be deterministic");

            let org1 = hash_org_id(&input);
            let org2 = hash_org_id(&input);
            prop_assert_eq!(&org1, &org2, "hash_org_id must be deterministic");
        }

        /// Property: Length - all hashes are exactly 64 characters (SHA-256 hex).
        #[test]
        fn length_is_64(input in "\\PC{0,500}") {
            prop_assert_eq!(hash_string(&input).len(), 64);
            prop_assert_eq!(hash_path(&input).len(), 64);
            prop_assert_eq!(hash_session_id(&input).len(), 64);
            prop_assert_eq!(hash_org_id(&input).len(), 64);
        }

        /// Property: Irreversibility (collision resistance) - different inputs produce different hashes.
        #[test]
        fn different_inputs_different_hashes(a in "\\PC{1,200}", b in "\\PC{1,200}") {
            prop_assume!(a != b);
            let hash_a = hash_string(&a);
            let hash_b = hash_string(&b);
            prop_assert_ne!(hash_a, hash_b, "Different inputs should produce different hashes");
        }

        /// Property: Non-empty output - any non-empty input produces a non-empty hash.
        #[test]
        fn non_empty_output(input in "\\PC{1,300}") {
            let result = hash_string(&input);
            prop_assert!(!result.is_empty(), "Hash output must not be empty");
            prop_assert!(result.chars().all(|c| c.is_ascii_hexdigit()),
                "Hash output must be valid hex");
        }

        /// Property: No raw data leakage - hash output does not contain any substring of the input (for inputs >= 4 chars).
        #[test]
        fn no_raw_data_leakage(input in "[a-zA-Z0-9_/\\\\\\-\\.]{4,100}") {
            let hash = hash_string(&input);
            // Check that no 4+ character substring of the input appears in the hash
            for window_size in 4..=input.len().min(64) {
                for start in 0..=(input.len() - window_size) {
                    let substring = &input[start..start + window_size];
                    prop_assert!(
                        !hash.contains(substring),
                        "Hash output must not contain substring '{}' from input",
                        substring
                    );
                }
            }
        }
    }
}
