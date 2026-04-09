//! Connection monitoring and failover for Plex server connections.
//!
//! Monitors network interface changes, debounces rapid transitions (500ms),
//! and evaluates connections with a three-tier failover strategy:
//! 1. Test current connection (3s timeout)
//! 2. Try cached connections sorted by priority (5s timeout each)
//! 3. Re-discover from plex.tv as last resort

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use url::Url;

use crate::models::PlexServer;
use crate::plex::client::PlexClient;

// --- Constants ---

/// Debounce window for network interface changes.
const DEBOUNCE_MS: u64 = 500;

/// Timeout for testing the current (active) connection.
const FAST_PATH_TIMEOUT: Duration = Duration::from_secs(3);

/// Timeout for testing cached alternative connections.
const CACHED_TIMEOUT: Duration = Duration::from_secs(5);

// --- Callback types ---

/// Callback fired when a new working connection is found.
/// Parameters: (server_url, access_token, is_local, is_http)
pub type ConnectionChangedCallback = Arc<dyn Fn(Url, String, bool, bool) + Send + Sync>;

/// Callback fired when all connections fail.
pub type ConnectionLostCallback = Arc<dyn Fn() + Send + Sync>;

// --- ConnectionMonitor ---

struct MonitorInner {
    cached_server: Option<PlexServer>,
    active_uri: Option<String>,
    auth_token: Option<String>,
    last_interfaces: HashSet<String>,
    allow_http: bool,
    is_evaluating: bool,
    debounce_handle: Option<tokio::task::AbortHandle>,
    on_connection_changed: Option<ConnectionChangedCallback>,
    on_connection_lost: Option<ConnectionLostCallback>,
}

/// Monitors network changes and manages Plex server connection failover.
///
/// Create with `Arc<ConnectionMonitor>` — the `handle_path_update` method
/// requires `&Arc<Self>` to spawn debounced evaluation tasks.
pub struct ConnectionMonitor {
    client: Arc<PlexClient>,
    inner: Mutex<MonitorInner>,
}

impl ConnectionMonitor {
    pub fn new(client: Arc<PlexClient>) -> Self {
        Self {
            client,
            inner: Mutex::new(MonitorInner {
                cached_server: None,
                active_uri: None,
                auth_token: None,
                last_interfaces: HashSet::new(),
                allow_http: true,
                is_evaluating: false,
                debounce_handle: None,
                on_connection_changed: None,
                on_connection_lost: None,
            }),
        }
    }

    // -- Configuration --

    pub fn set_on_connection_changed(&self, handler: ConnectionChangedCallback) {
        self.inner.lock().on_connection_changed = Some(handler);
    }

    pub fn set_on_connection_lost(&self, handler: ConnectionLostCallback) {
        self.inner.lock().on_connection_lost = Some(handler);
    }

    pub fn set_allow_http(&self, value: bool) {
        self.inner.lock().allow_http = value;
    }

    /// Start monitoring with the given server and active connection.
    pub fn start(&self, server: PlexServer, active_uri: String, auth_token: String) {
        let mut inner = self.inner.lock();
        inner.cached_server = Some(server);
        inner.active_uri = Some(active_uri);
        inner.auth_token = Some(auth_token);
    }

    /// Stop monitoring and cancel any pending debounce.
    pub fn stop(&self) {
        let mut inner = self.inner.lock();
        if let Some(handle) = inner.debounce_handle.take() {
            handle.abort();
        }
        inner.cached_server = None;
        inner.active_uri = None;
        inner.auth_token = None;
        inner.last_interfaces.clear();
    }

    /// Current active connection URI, if any.
    pub fn active_uri(&self) -> Option<String> {
        self.inner.lock().active_uri.clone()
    }

    /// Update the active URI (e.g. after a background failover).
    pub fn update_active_uri(&self, uri: String) {
        self.inner.lock().active_uri = Some(uri);
    }

    /// Replace the cached server (e.g. after re-discovery brings fresh connections).
    pub fn update_server(&self, server: PlexServer) {
        self.inner.lock().cached_server = Some(server);
    }

    /// Whether the monitor is currently evaluating connections.
    pub fn is_evaluating(&self) -> bool {
        self.inner.lock().is_evaluating
    }

    // -- Network change handling --

    /// Called when network interfaces change (from platform-specific monitoring).
    ///
    /// Compares interfaces with the previous set and only triggers evaluation
    /// if the set changed. Debounces with a 500ms delay.
    pub fn handle_path_update(self: &Arc<Self>, interfaces: HashSet<String>) {
        let mut inner = self.inner.lock();

        // Skip spurious events (no actual change)
        if interfaces == inner.last_interfaces {
            return;
        }
        inner.last_interfaces = interfaces;

        // Cancel previous debounce
        if let Some(handle) = inner.debounce_handle.take() {
            handle.abort();
        }

        // Spawn debounced evaluation
        let monitor = Arc::clone(self);
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(DEBOUNCE_MS)).await;
            monitor.evaluate_connection().await;
        });
        inner.debounce_handle = Some(handle.abort_handle());
    }

    // -- Connection evaluation --

    /// Evaluate the connection using a three-tier failover strategy.
    ///
    /// 1. Fast path: test current URI (3s timeout)
    /// 2. Cached: test sorted connections (5s timeout each)
    /// 3. Re-discover from plex.tv
    pub async fn evaluate_connection(&self) {
        // Reentrancy guard
        {
            let mut inner = self.inner.lock();
            if inner.is_evaluating {
                return;
            }
            inner.is_evaluating = true;
        }

        // Ensure we clear the flag on exit
        let result = self.do_evaluate().await;

        self.inner.lock().is_evaluating = false;

        // Fire appropriate callback based on result
        match result {
            EvalResult::Unchanged => {}
            EvalResult::Changed { url, token, is_local, is_http } => {
                let cb = self.inner.lock().on_connection_changed.clone();
                if let Some(cb) = cb {
                    cb(url, token, is_local, is_http);
                }
            }
            EvalResult::Lost => {
                let cb = self.inner.lock().on_connection_lost.clone();
                if let Some(cb) = cb {
                    cb();
                }
            }
        }
    }

    async fn do_evaluate(&self) -> EvalResult {
        let (server, active_uri, auth_token, allow_http) = {
            let inner = self.inner.lock();
            match (&inner.cached_server, &inner.active_uri, &inner.auth_token) {
                (Some(s), Some(u), Some(t)) => {
                    (s.clone(), u.clone(), t.clone(), inner.allow_http)
                }
                _ => return EvalResult::Unchanged,
            }
        };

        // 1. Fast path: test current connection
        log::debug!("monitor: testing active URI: {}", active_uri);
        if matches_http_policy(&active_uri, allow_http)
            && self
                .client
                .test_connection(&active_uri, &server.access_token, Some(FAST_PATH_TIMEOUT))
                .await
        {
            return EvalResult::Unchanged;
        }
        log::debug!("monitor: active URI failed, trying {} cached connection(s)", server.connections.len());

        // 2. Try cached connections in priority order
        for conn in server.sorted_connections() {
            if conn.uri == active_uri {
                continue;
            }
            if !allow_http && conn.protocol != "https" {
                log::debug!("monitor: skipping HTTP connection (refuse_http): {}", conn.uri);
                continue;
            }

            log::debug!("monitor: testing cached: {} (local={}, relay={})", conn.uri, conn.local, conn.relay);
            if self
                .client
                .test_connection(&conn.uri, &server.access_token, Some(CACHED_TIMEOUT))
                .await
            {
                let url = match Url::parse(&conn.uri) {
                    Ok(u) => u,
                    Err(_) => continue,
                };
                let is_local = conn.local;
                let is_http = conn.protocol != "https";

                self.inner.lock().active_uri = Some(conn.uri.clone());
                crate::plex::auth::patch_stored_config(None, Some(&conn.uri));

                log::info!("monitor: switched to cached connection: {}", conn.uri);
                return EvalResult::Changed {
                    url,
                    token: server.access_token.clone(),
                    is_local,
                    is_http,
                };
            } else {
                log::debug!("monitor: cached connection failed: {}", conn.uri);
            }
        }

        // 3. Re-discover from plex.tv
        log::info!("monitor: all cached connections failed, re-discovering from plex.tv");
        match self.client.discover_servers(&auth_token).await {
            Ok(servers) => {
                if let Some(found) = servers
                    .iter()
                    .find(|s| s.machine_identifier == server.machine_identifier)
                {
                    log::debug!(
                        "monitor: re-discovered server with {} connection(s)",
                        found.connections.len(),
                    );
                    let (best, is_http) = self.client.find_best_connection(found, allow_http, true).await;
                    if let Some(conn) = best {
                        if let Ok(url) = Url::parse(&conn.uri) {
                            let is_local = conn.local;

                            // Update cached server with fresh data and new active URI
                            {
                                let mut inner = self.inner.lock();
                                inner.cached_server = Some(found.clone());
                                inner.active_uri = Some(conn.uri.clone());
                            }
                            // Persist fresh connections and active URI to disk
                            crate::plex::auth::patch_stored_config(
                                Some(&found.connections),
                                Some(&conn.uri),
                            );

                            log::info!("monitor: switched to re-discovered connection: {}", conn.uri);
                            return EvalResult::Changed {
                                url,
                                token: found.access_token.clone(),
                                is_local,
                                is_http,
                            };
                        }
                    } else {
                        log::warn!("monitor: re-discovered server but all {} connections failed", found.connections.len());
                    }
                } else {
                    log::warn!(
                        "monitor: server {} not found in {} re-discovered server(s)",
                        server.machine_identifier,
                        servers.len(),
                    );
                }
            }
            Err(e) => {
                log::warn!("monitor: plex.tv re-discovery failed: {}", e);
            }
        }

        // All tiers failed
        log::warn!("monitor: all connection tiers exhausted");
        EvalResult::Lost
    }
}

enum EvalResult {
    Unchanged,
    Changed {
        url: Url,
        token: String,
        is_local: bool,
        is_http: bool,
    },
    Lost,
}

// --- Pure helpers ---

/// Check if a URI matches the HTTP policy.
/// If `allow_http` is true, all URIs pass. Otherwise, only HTTPS.
fn matches_http_policy(uri: &str, allow_http: bool) -> bool {
    if allow_http {
        return true;
    }
    uri.starts_with("https://")
}

/// Detect whether a network interface set change warrants re-evaluation.
pub fn interfaces_changed(current: &HashSet<String>, new: &HashSet<String>) -> bool {
    current != new
}

// --- Tests ---

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::PlexServerConnection;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    // -- Pure logic tests --

    #[test]
    fn test_http_policy_allows_all_when_enabled() {
        assert!(matches_http_policy("http://local:32400", true));
        assert!(matches_http_policy("https://remote:32400", true));
    }

    #[test]
    fn test_http_policy_rejects_http_when_disabled() {
        assert!(!matches_http_policy("http://local:32400", false));
        assert!(matches_http_policy("https://remote:32400", false));
    }

    #[test]
    fn test_interface_change_detection() {
        let set1: HashSet<String> = ["en0", "lo0"].iter().map(|s| s.to_string()).collect();
        let set2: HashSet<String> = ["en0", "lo0"].iter().map(|s| s.to_string()).collect();
        let set3: HashSet<String> = ["en0", "utun0"].iter().map(|s| s.to_string()).collect();

        assert!(!interfaces_changed(&set1, &set2)); // Same
        assert!(interfaces_changed(&set1, &set3)); // Different
        assert!(interfaces_changed(&set1, &HashSet::new())); // All removed
    }

    #[test]
    fn test_monitor_start_stop() {
        let client = Arc::new(PlexClient::new("test".into()));
        let monitor = ConnectionMonitor::new(client);

        assert!(monitor.active_uri().is_none());

        let server = make_test_server("server-1");
        monitor.start(server, "https://local:32400".into(), "token".into());
        assert_eq!(monitor.active_uri(), Some("https://local:32400".into()));

        monitor.stop();
        assert!(monitor.active_uri().is_none());
    }

    #[test]
    fn test_allow_http_default() {
        let client = Arc::new(PlexClient::new("test".into()));
        let monitor = ConnectionMonitor::new(client);
        // Default is allow HTTP
        assert!(matches_http_policy("http://test", monitor.inner.lock().allow_http));

        monitor.set_allow_http(false);
        assert!(!matches_http_policy("http://test", monitor.inner.lock().allow_http));
    }

    // -- Debounce tests --

    #[tokio::test]
    async fn test_debounce_skips_unchanged_interfaces() {
        let client = Arc::new(PlexClient::new("test".into()));
        let monitor = Arc::new(ConnectionMonitor::new(client));

        let server = make_test_server("server-1");
        monitor.start(server, "https://local:32400".into(), "token".into());

        let interfaces: HashSet<String> = ["en0"].iter().map(|s| s.to_string()).collect();

        // First update should be accepted
        monitor.handle_path_update(interfaces.clone());
        assert!(monitor.inner.lock().debounce_handle.is_some());

        // Cancel the debounce task to prevent it from running
        monitor.inner.lock().debounce_handle.take().unwrap().abort();

        // Same interfaces — should be skipped (no new debounce)
        monitor.handle_path_update(interfaces);
        assert!(monitor.inner.lock().debounce_handle.is_none());
    }

    #[tokio::test]
    async fn test_debounce_cancels_previous() {
        let client = Arc::new(PlexClient::new("test".into()));
        let monitor = Arc::new(ConnectionMonitor::new(client));

        let server = make_test_server("server-1");
        monitor.start(server, "https://local:32400".into(), "token".into());

        // Rapid interface changes — each cancels the previous debounce
        let set1: HashSet<String> = ["en0"].iter().map(|s| s.to_string()).collect();
        let set2: HashSet<String> = ["en0", "utun0"].iter().map(|s| s.to_string()).collect();
        let set3: HashSet<String> = ["en0", "utun0", "en1"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        monitor.handle_path_update(set1);
        monitor.handle_path_update(set2);
        monitor.handle_path_update(set3);

        // After rapid changes, should have exactly one pending debounce
        assert!(monitor.inner.lock().debounce_handle.is_some());

        // Interfaces should reflect the latest update
        assert_eq!(monitor.inner.lock().last_interfaces.len(), 3);

        // Clean up
        monitor.inner.lock().debounce_handle.take().unwrap().abort();
    }

    // -- Evaluation tests --

    #[tokio::test]
    async fn test_evaluate_reentrancy_guard() {
        let client = Arc::new(PlexClient::new("test".into()));
        let monitor = ConnectionMonitor::new(client);

        let server = make_test_server("server-1");
        monitor.start(server, "https://local:32400".into(), "token".into());

        // Manually set evaluating flag
        monitor.inner.lock().is_evaluating = true;

        // Should return immediately without doing anything
        monitor.evaluate_connection().await;

        // Flag should still be set (wasn't cleared because we entered the guard)
        assert!(monitor.inner.lock().is_evaluating);

        // Clean up
        monitor.inner.lock().is_evaluating = false;
    }

    #[tokio::test]
    async fn test_evaluate_without_server_is_noop() {
        let client = Arc::new(PlexClient::new("test".into()));
        let monitor = ConnectionMonitor::new(client);

        // No server configured — should be a no-op
        monitor.evaluate_connection().await;
        assert!(!monitor.is_evaluating());
    }

    #[tokio::test]
    async fn test_evaluate_fast_path_success() {
        let mock = wiremock::MockServer::start().await;

        // Mock /identity endpoint
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("identity"))
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&mock)
            .await;

        let client = Arc::new(PlexClient::new("test".into()));
        let monitor = ConnectionMonitor::new(client);

        let server = make_test_server_with_uri("s1", &mock.uri());
        let changed = Arc::new(AtomicBool::new(false));
        let lost = Arc::new(AtomicBool::new(false));

        let changed_flag = changed.clone();
        monitor.set_on_connection_changed(Arc::new(move |_, _, _, _| {
            changed_flag.store(true, Ordering::SeqCst);
        }));
        let lost_flag = lost.clone();
        monitor.set_on_connection_lost(Arc::new(move || {
            lost_flag.store(true, Ordering::SeqCst);
        }));

        monitor.start(server, mock.uri(), "token".into());
        monitor.evaluate_connection().await;

        // Fast path succeeded — no callbacks fired
        assert!(!changed.load(Ordering::SeqCst));
        assert!(!lost.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_evaluate_failover_to_cached() {
        let mock_dead = wiremock::MockServer::start().await;
        let mock_backup = wiremock::MockServer::start().await;

        // Dead server: no mock (connection refused or timeout)
        // Actually, wiremock returns 404 by default, which is not 200
        // So test_connection will return false

        // Backup server: respond with 200
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("identity"))
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&mock_backup)
            .await;

        let client = Arc::new(PlexClient::new("test".into()));
        let monitor = ConnectionMonitor::new(client);

        let server = PlexServer {
            machine_identifier: "s1".into(),
            name: "Test".into(),
            access_token: "token".into(),
            owned: true,
            connections: vec![
                PlexServerConnection {
                    uri: mock_dead.uri(),
                    local: true,
                    relay: false,
                    protocol: "https".into(),
                },
                PlexServerConnection {
                    uri: mock_backup.uri(),
                    local: false,
                    relay: false,
                    protocol: "https".into(),
                },
            ],
        };

        let changed_uri = Arc::new(Mutex::new(String::new()));
        let changed_flag = changed_uri.clone();
        monitor.set_on_connection_changed(Arc::new(move |url, _, _, _| {
            *changed_flag.lock() = url.to_string();
        }));

        monitor.start(server, mock_dead.uri(), "token".into());
        monitor.evaluate_connection().await;

        // Should have failed over to backup
        let new_uri = changed_uri.lock().clone();
        assert!(
            new_uri.contains(&mock_backup.address().port().to_string()),
            "Expected failover to backup server, got: {new_uri}"
        );
    }

    #[tokio::test]
    async fn test_evaluate_all_fail_signals_lost() {
        let mock_dead = wiremock::MockServer::start().await;
        // No mocks → all return 404 → test_connection fails

        let client = Arc::new(PlexClient::new("test".into()));
        let monitor = ConnectionMonitor::new(client);

        let server = PlexServer {
            machine_identifier: "s1".into(),
            name: "Test".into(),
            access_token: "token".into(),
            owned: true,
            connections: vec![PlexServerConnection {
                uri: mock_dead.uri(),
                local: true,
                relay: false,
                protocol: "https".into(),
            }],
        };

        let lost = Arc::new(AtomicBool::new(false));
        let lost_flag = lost.clone();
        monitor.set_on_connection_lost(Arc::new(move || {
            lost_flag.store(true, Ordering::SeqCst);
        }));

        monitor.start(server, mock_dead.uri(), "token".into());
        monitor.evaluate_connection().await;

        assert!(lost.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_evaluate_respects_http_policy() {
        let mock_http = wiremock::MockServer::start().await;

        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("identity"))
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&mock_http)
            .await;

        let client = Arc::new(PlexClient::new("test".into()));
        let monitor = ConnectionMonitor::new(client);
        monitor.set_allow_http(false); // Reject HTTP

        // Active URI is HTTP (wiremock uses http)
        let server = PlexServer {
            machine_identifier: "s1".into(),
            name: "Test".into(),
            access_token: "token".into(),
            owned: true,
            connections: vec![PlexServerConnection {
                uri: mock_http.uri(),
                local: true,
                relay: false,
                protocol: "http".into(),
            }],
        };

        let lost = Arc::new(AtomicBool::new(false));
        let lost_flag = lost.clone();
        monitor.set_on_connection_lost(Arc::new(move || {
            lost_flag.store(true, Ordering::SeqCst);
        }));

        monitor.start(server, mock_http.uri(), "token".into());
        monitor.evaluate_connection().await;

        // HTTP connection should be rejected, resulting in connection lost
        assert!(lost.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_evaluate_clears_flag_on_completion() {
        let client = Arc::new(PlexClient::new("test".into()));
        let monitor = ConnectionMonitor::new(client);

        let server = make_test_server("s1");
        monitor.start(server, "https://nonexistent:32400".into(), "token".into());

        assert!(!monitor.is_evaluating());
        monitor.evaluate_connection().await;
        assert!(!monitor.is_evaluating());
    }

    #[tokio::test]
    async fn test_evaluate_failover_priority_order() {
        // Create mock servers
        let mock_local = wiremock::MockServer::start().await;
        let mock_remote = wiremock::MockServer::start().await;
        let mock_relay = wiremock::MockServer::start().await;

        // Only relay responds (local and remote are "dead")
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("identity"))
            .respond_with(wiremock::ResponseTemplate::new(200))
            .mount(&mock_relay)
            .await;

        let client = Arc::new(PlexClient::new("test".into()));
        let monitor = ConnectionMonitor::new(client);

        let server = PlexServer {
            machine_identifier: "s1".into(),
            name: "Test".into(),
            access_token: "token".into(),
            owned: true,
            connections: vec![
                PlexServerConnection {
                    uri: mock_local.uri(),
                    local: true,
                    relay: false,
                    protocol: "https".into(),
                },
                PlexServerConnection {
                    uri: mock_remote.uri(),
                    local: false,
                    relay: false,
                    protocol: "https".into(),
                },
                PlexServerConnection {
                    uri: mock_relay.uri(),
                    local: false,
                    relay: true,
                    protocol: "https".into(),
                },
            ],
        };

        let changed_uri = Arc::new(Mutex::new(String::new()));
        let changed_flag = changed_uri.clone();
        monitor.set_on_connection_changed(Arc::new(move |url, _, _, _| {
            *changed_flag.lock() = url.to_string();
        }));

        // Start with a fake dead URI so fast path fails
        monitor.start(server, "https://dead:32400".into(), "token".into());
        monitor.evaluate_connection().await;

        // Should have found the relay (only one responding)
        let result = changed_uri.lock().clone();
        assert!(
            result.contains(&mock_relay.address().port().to_string()),
            "Expected relay connection, got: {result}"
        );
    }

    // -- Test helpers --

    fn make_test_server(id: &str) -> PlexServer {
        PlexServer {
            machine_identifier: id.into(),
            name: "Test Server".into(),
            access_token: "test-token".into(),
            owned: true,
            connections: vec![PlexServerConnection {
                uri: "https://test.local:32400".into(),
                local: true,
                relay: false,
                protocol: "https".into(),
            }],
        }
    }

    fn make_test_server_with_uri(id: &str, uri: &str) -> PlexServer {
        PlexServer {
            machine_identifier: id.into(),
            name: "Test Server".into(),
            access_token: "test-token".into(),
            owned: true,
            connections: vec![PlexServerConnection {
                uri: uri.into(),
                local: true,
                relay: false,
                protocol: "https".into(),
            }],
        }
    }
}
