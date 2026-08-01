use std::collections::HashSet;

use chrono::Utc;

/// Network guard that enforces the Zero-Token Guarantee by blocking
/// all outbound requests to AI inference endpoints and only allowing
/// explicitly allowlisted hosts.
pub struct NetworkGuard {
    allowed_hosts: HashSet<String>,
    blocked_domains: Vec<String>,
}

/// Audit log entry generated when a request is blocked.
#[derive(Debug, Clone)]
pub struct BlockedRequestEntry {
    pub timestamp: String,
    pub url: String,
    pub host: String,
    pub reason: String,
}

impl NetworkGuard {
    /// Create a new NetworkGuard with default allowed hosts and blocked patterns.
    pub fn new() -> Self {
        let mut allowed_hosts = HashSet::new();
        allowed_hosts.insert("api.github.com".to_string());
        allowed_hosts.insert("localhost".to_string());
        allowed_hosts.insert("127.0.0.1".to_string());

        let blocked_domains = vec![
            "openai.com".to_string(),
            "anthropic.com".to_string(),
            "chatgpt.com".to_string(),
            "claude.ai".to_string(),
        ];

        Self {
            allowed_hosts,
            blocked_domains,
        }
    }

    /// Check if a URL is allowed through the network guard.
    ///
    /// Returns true ONLY if the host is in `allowed_hosts` AND does not
    /// match any blocked pattern. Returns false for malformed URLs and
    /// any unknown domains (default deny).
    pub fn is_allowed(&self, url: &str) -> bool {
        let host = match extract_host(url) {
            Some(h) => h,
            None => return false,
        };

        // Check if host matches any blocked domain or subdomain
        if self.is_blocked(&host) {
            return false;
        }

        // Default deny: only allow explicitly listed hosts
        self.allowed_hosts.contains(&host)
    }

    /// Check if a host matches any blocked domain pattern (including subdomains).
    fn is_blocked(&self, host: &str) -> bool {
        for domain in &self.blocked_domains {
            // Exact match
            if host == domain.as_str() {
                return true;
            }
            // Subdomain match (e.g., "api.openai.com" ends with ".openai.com")
            if host.ends_with(&format!(".{}", domain)) {
                return true;
            }
        }
        false
    }

    /// Generate an audit log entry for a blocked request.
    pub fn audit_blocked_request(&self, url: &str) -> BlockedRequestEntry {
        let host = extract_host(url).unwrap_or_default();
        let reason = if host.is_empty() {
            "malformed URL".to_string()
        } else if self.is_blocked(&host) {
            format!("blocked inference endpoint: {}", host)
        } else {
            format!("host not in allowlist: {}", host)
        };

        BlockedRequestEntry {
            timestamp: Utc::now().to_rfc3339(),
            url: url.to_string(),
            host,
            reason,
        }
    }
}

impl Default for NetworkGuard {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract the host from a URL string.
/// Returns None for malformed URLs.
fn extract_host(url: &str) -> Option<String> {
    // Must have a scheme
    let after_scheme = if let Some(rest) = url.strip_prefix("https://") {
        rest
    } else { url.strip_prefix("http://")? };

    if after_scheme.is_empty() {
        return None;
    }

    // Extract host portion (before path, query, or port)
    let host_part = after_scheme.split('/').next().unwrap_or("");
    if host_part.is_empty() {
        return None;
    }

    // Strip query string if present (for URLs like "host?query" without a path)
    let host_part = if host_part.contains('?') {
        host_part.split('?').next().unwrap_or("")
    } else {
        host_part
    };

    if host_part.is_empty() {
        return None;
    }

    // Strip port if present
    let host = if host_part.contains(':') {
        host_part.split(':').next().unwrap_or("")
    } else {
        host_part
    };

    if host.is_empty() {
        return None;
    }

    Some(host.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_github_repos_allowed() {
        let guard = NetworkGuard::new();
        assert!(guard.is_allowed("https://api.github.com/repos/owner/repo/releases/latest"));
        assert!(guard.is_allowed("https://api.github.com/repos/user/project"));
    }

    #[test]
    fn test_localhost_allowed() {
        let guard = NetworkGuard::new();
        assert!(guard.is_allowed("http://localhost:3000/api/data"));
        assert!(guard.is_allowed("http://127.0.0.1:8080/status"));
    }

    #[test]
    fn test_openai_blocked() {
        let guard = NetworkGuard::new();
        assert!(!guard.is_allowed("https://api.openai.com/v1/chat/completions"));
        assert!(!guard.is_allowed("https://api.openai.com/v1/responses"));
        assert!(!guard.is_allowed("https://openai.com"));
    }

    #[test]
    fn test_anthropic_blocked() {
        let guard = NetworkGuard::new();
        assert!(!guard.is_allowed("https://api.anthropic.com/v1/messages"));
        assert!(!guard.is_allowed("https://anthropic.com"));
    }

    #[test]
    fn test_subdomains_blocked() {
        let guard = NetworkGuard::new();
        assert!(!guard.is_allowed("https://beta.openai.com/something"));
        assert!(!guard.is_allowed("https://console.anthropic.com/dashboard"));
        assert!(!guard.is_allowed("https://sub.api.openai.com/v1/models"));
        assert!(!guard.is_allowed("https://chatgpt.com/chat"));
        assert!(!guard.is_allowed("https://claude.ai/chat"));
    }

    #[test]
    fn test_malformed_urls_return_false() {
        let guard = NetworkGuard::new();
        assert!(!guard.is_allowed(""));
        assert!(!guard.is_allowed("not-a-url"));
        assert!(!guard.is_allowed("ftp://some-server.com/file"));
        assert!(!guard.is_allowed("://missing-scheme"));
        assert!(!guard.is_allowed("https://"));
    }

    #[test]
    fn test_unknown_domains_return_false() {
        let guard = NetworkGuard::new();
        assert!(!guard.is_allowed("https://example.com/page"));
        assert!(!guard.is_allowed("https://random-api.io/v1/data"));
        assert!(!guard.is_allowed("https://google.com"));
    }

    #[test]
    fn test_audit_log_blocked_inference() {
        let guard = NetworkGuard::new();
        let entry = guard.audit_blocked_request("https://api.openai.com/v1/chat/completions");
        assert_eq!(entry.host, "api.openai.com");
        assert!(entry.reason.contains("blocked inference endpoint"));
    }

    #[test]
    fn test_audit_log_not_allowlisted() {
        let guard = NetworkGuard::new();
        let entry = guard.audit_blocked_request("https://example.com/api");
        assert_eq!(entry.host, "example.com");
        assert!(entry.reason.contains("not in allowlist"));
    }

    #[test]
    fn test_audit_log_malformed() {
        let guard = NetworkGuard::new();
        let entry = guard.audit_blocked_request("not-a-url");
        assert_eq!(entry.host, "");
        assert!(entry.reason.contains("malformed URL"));
    }
}

// **Validates: Requirements 5.1, 5.2, 5.5**
///
/// Property 6: Network Guard Completeness
/// Verifies that the NetworkGuard correctly blocks inference endpoints,
/// allows only explicitly allowlisted hosts, and denies unknown hosts.
#[cfg(test)]
mod prop_tests_network_guard {
    use super::*;
    use proptest::prelude::*;
    use proptest::test_runner::Config;

    /// Strategy to generate random path segments (e.g., "/v1/chat/completions")
    fn path_strategy() -> impl Strategy<Value = String> {
        prop::collection::vec("[a-z0-9_\\-]{1,12}", 0..4)
            .prop_map(|segments| {
                if segments.is_empty() {
                    String::new()
                } else {
                    format!("/{}", segments.join("/"))
                }
            })
    }

    /// Strategy to generate random query strings (e.g., "?key=value&foo=bar")
    fn query_strategy() -> impl Strategy<Value = String> {
        prop::collection::vec(
            ("[a-z]{1,6}", "[a-z0-9]{1,8}"),
            0..3,
        )
        .prop_map(|pairs| {
            if pairs.is_empty() {
                String::new()
            } else {
                let qs: Vec<String> = pairs.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
                format!("?{}", qs.join("&"))
            }
        })
    }

    /// Strategy to generate random subdomain prefixes (e.g., "chat.", "beta.api.")
    fn subdomain_strategy() -> impl Strategy<Value = String> {
        prop::collection::vec("[a-z]{2,8}", 1..3)
            .prop_map(|parts| format!("{}.", parts.join(".")))
    }

    /// Strategy to generate random non-allowlisted domain names
    fn random_domain_strategy() -> impl Strategy<Value = String> {
        ("[a-z]{3,10}", prop::sample::select(vec!["com", "io", "net", "org", "dev"]))
            .prop_map(|(name, tld)| format!("{}.{}", name, tld))
            .prop_filter("must not be an allowlisted or blocked domain", |domain| {
                let blocked = ["openai.com", "anthropic.com", "chatgpt.com", "claude.ai"];
                let allowed = ["api.github.com", "localhost", "127.0.0.1"];
                !blocked.iter().any(|b| domain == *b || domain.ends_with(&format!(".{}", b)))
                    && !allowed.contains(&domain.as_str())
            })
    }

    // Property 1: All blocked patterns rejected - any URL containing blocked host patterns
    // (openai.com, anthropic.com, and their subdomains) must be rejected by is_allowed().
    proptest! {
        #![proptest_config(Config::with_cases(50))]

        #[test]
        fn blocked_hosts_always_rejected(
            path in path_strategy(),
            query in query_strategy(),
            host_idx in 0usize..4,
        ) {
            let guard = NetworkGuard::new();
            let blocked_hosts = [
                "openai.com",
                "anthropic.com",
                "chatgpt.com",
                "claude.ai",
            ];
            let host = blocked_hosts[host_idx];
            let url = format!("https://{}{}{}", host, path, query);
            prop_assert!(!guard.is_allowed(&url),
                "URL to blocked host should be rejected: {}", url);
        }
    }

    // Property 2: Allowlisted hosts pass - URLs to explicitly allowed hosts
    // (api.github.com) must pass is_allowed().
    proptest! {
        #![proptest_config(Config::with_cases(50))]

        #[test]
        fn allowlisted_hosts_always_pass(
            path in path_strategy(),
            query in query_strategy(),
        ) {
            let guard = NetworkGuard::new();
            let url = format!("https://api.github.com{}{}", path, query);
            prop_assert!(guard.is_allowed(&url),
                "URL to allowlisted host should be allowed: {}", url);
        }
    }

    // Property 3: Unknown hosts blocked - any URL to a host not in the allowlist
    // must be rejected.
    proptest! {
        #![proptest_config(Config::with_cases(50))]

        #[test]
        fn unknown_hosts_always_blocked(
            domain in random_domain_strategy(),
            path in path_strategy(),
        ) {
            let guard = NetworkGuard::new();
            let url = format!("https://{}{}", domain, path);
            prop_assert!(!guard.is_allowed(&url),
                "URL to unknown host should be blocked: {}", url);
        }
    }

    // Property 4: Path and query don't bypass - adding paths or query strings
    // to blocked hosts doesn't make them pass.
    proptest! {
        #![proptest_config(Config::with_cases(50))]

        #[test]
        fn paths_and_queries_dont_bypass_blocking(
            path in path_strategy(),
            query in query_strategy(),
            host_idx in 0usize..2,
        ) {
            let guard = NetworkGuard::new();
            let blocked_api_hosts = [
                "api.openai.com",
                "api.anthropic.com",
            ];
            let host = blocked_api_hosts[host_idx];
            let url = format!("https://{}{}{}", host, path, query);
            prop_assert!(!guard.is_allowed(&url),
                "Paths/queries should not bypass blocking: {}", url);
        }
    }

    // Property 5: Subdomain blocking - subdomains of blocked patterns
    // (e.g., chat.api.openai.com, messages.api.anthropic.com) are also blocked.
    proptest! {
        #![proptest_config(Config::with_cases(50))]

        #[test]
        fn subdomains_of_blocked_hosts_rejected(
            subdomain in subdomain_strategy(),
            path in path_strategy(),
            host_idx in 0usize..4,
        ) {
            let guard = NetworkGuard::new();
            let blocked_hosts = [
                "openai.com",
                "anthropic.com",
                "chatgpt.com",
                "claude.ai",
            ];
            let host = blocked_hosts[host_idx];
            let url = format!("https://{}{}{}", subdomain, host, path);
            prop_assert!(!guard.is_allowed(&url),
                "Subdomain of blocked host should be rejected: {}", url);
        }
    }
}
