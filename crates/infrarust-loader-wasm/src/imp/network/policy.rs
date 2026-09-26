use std::fmt::Display;
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use infrarust_config::{HostPattern, NetworkRule, PortRange, WasmNetworkConfig};
use tokio::sync::Mutex;
use tokio::time::Instant;
use tracing::instrument::WithSubscriber;
use wasmtime_wasi::sockets::SocketAddrUse;

use super::resolver::Resolver;
use super::{DENIAL_LOG_BURST, HOSTNAME_REFRESH_INTERVAL};
use crate::consts::DENIED_CALL_LOG_INTERVAL;
use crate::rate_limit::SharedRateLimit;

pub(crate) type SocketCheckFuture = Pin<Box<dyn Future<Output = bool> + Send + Sync>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    MissingCapability,
    AllowList,
    HttpDisabled,
}

impl Refusal {
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::MissingCapability => "missing capability `network`",
            Self::AllowList => "network allow-list",
            Self::HttpDisabled => "http = false",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HttpRoute {
    Literal(SocketAddr),
    Named {
        name: String,
        port: u16,
        approved: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HttpResolveError {
    Denied,
    Lookup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    Allow,
    Deny(Refusal),
    Lookup,
}

enum Mode {
    Off(Refusal),
    On {
        rules: Vec<NetworkRule>,
        dns: bool,
        http: bool,
    },
}

struct HostnameSlot {
    name: String,
    ports: Vec<PortRange>,
    state: Mutex<Resolution>,
}

#[derive(Default)]
struct Resolution {
    ips: Vec<IpAddr>,
    attempted: Option<Instant>,
}

impl Resolution {
    fn stale(&self) -> bool {
        self.attempted
            .is_none_or(|at| at.elapsed() >= HOSTNAME_REFRESH_INTERVAL)
    }
}

impl HostnameSlot {
    fn covers(&self, port: u16) -> bool {
        self.ports.iter().any(|ports| ports.contains(port))
    }
}

pub(crate) struct NetworkPolicy {
    plugin_id: String,
    mode: Mode,
    hostnames: Vec<HostnameSlot>,
    resolver: Arc<dyn Resolver>,
    lookup_timeout: Duration,
    denials: SharedRateLimit,
    pub(super) tls: OnceLock<Result<Arc<rustls::ClientConfig>, String>>,
}

impl NetworkPolicy {
    pub(crate) fn disabled(plugin_id: String, resolver: Arc<dyn Resolver>) -> Self {
        Self::build(
            plugin_id,
            Mode::Off(Refusal::MissingCapability),
            resolver,
            Duration::ZERO,
        )
    }

    pub(crate) fn new(
        plugin_id: String,
        config: Option<&WasmNetworkConfig>,
        granted: bool,
        lookup_timeout: Duration,
        resolver: Arc<dyn Resolver>,
    ) -> Self {
        let mode = match config {
            _ if !granted => Mode::Off(Refusal::MissingCapability),
            Some(config) if !config.allow.is_empty() => Mode::On {
                rules: config.allow.clone(),
                dns: config.dns_enabled(),
                http: config.http,
            },
            _ => Mode::Off(Refusal::AllowList),
        };
        Self::build(plugin_id, mode, resolver, lookup_timeout)
    }

    fn build(
        plugin_id: String,
        mode: Mode,
        resolver: Arc<dyn Resolver>,
        lookup_timeout: Duration,
    ) -> Self {
        let mut hostnames: Vec<HostnameSlot> = Vec::new();
        if let Mode::On { rules, .. } = &mode {
            for rule in rules {
                let Some(name) = rule.hostname() else {
                    continue;
                };
                match hostnames.iter_mut().find(|slot| slot.name == name) {
                    Some(slot) => slot.ports.push(rule.ports()),
                    None => hostnames.push(HostnameSlot {
                        name: name.to_owned(),
                        ports: vec![rule.ports()],
                        state: Mutex::default(),
                    }),
                }
            }
        }
        Self {
            plugin_id,
            mode,
            hostnames,
            resolver,
            lookup_timeout,
            denials: SharedRateLimit::new(DENIED_CALL_LOG_INTERVAL, DENIAL_LOG_BURST),
            tls: OnceLock::new(),
        }
    }

    pub(crate) fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    pub(crate) fn is_active(&self) -> bool {
        matches!(self.mode, Mode::On { .. })
    }

    pub(crate) fn dns(&self) -> bool {
        matches!(self.mode, Mode::On { dns: true, .. })
    }

    pub(crate) fn has_hostnames(&self) -> bool {
        !self.hostnames.is_empty()
    }

    pub(crate) fn socket_check(
        self: &Arc<Self>,
    ) -> impl Fn(SocketAddr, SocketAddrUse) -> SocketCheckFuture + Send + Sync + 'static {
        let policy = Arc::clone(self);
        move |addr, usage| {
            let addr = canonical(addr);
            match policy.decide(addr, usage) {
                Decision::Allow => Box::pin(async { true }),
                Decision::Deny(refusal) => {
                    policy.report_denied(usage_label(usage), addr, refusal);
                    Box::pin(async { false })
                }
                Decision::Lookup => {
                    let policy = Arc::clone(&policy);
                    Box::pin(wasmtime_wasi::runtime::spawn(
                        async move { policy.finish_check(addr, usage).await }
                            .with_current_subscriber(),
                    ))
                }
            }
        }
    }

    async fn finish_check(&self, addr: SocketAddr, usage: SocketAddrUse) -> bool {
        let allowed = self.resolved_contains(addr).await;
        if !allowed {
            self.report_denied(usage_label(usage), addr, Refusal::AllowList);
        }
        allowed
    }

    fn decide(&self, addr: SocketAddr, usage: SocketAddrUse) -> Decision {
        let rules = match &self.mode {
            Mode::Off(refusal) => return Decision::Deny(*refusal),
            Mode::On { rules, .. } => rules,
        };
        match usage {
            SocketAddrUse::TcpBind | SocketAddrUse::UdpBind => {
                let ephemeral_udp = matches!(usage, SocketAddrUse::UdpBind) && addr.port() == 0;
                if ephemeral_udp || rules.iter().any(|rule| rule.permits_bind(addr)) {
                    Decision::Allow
                } else {
                    Decision::Deny(Refusal::AllowList)
                }
            }
            SocketAddrUse::TcpConnect
            | SocketAddrUse::UdpConnect
            | SocketAddrUse::UdpOutgoingDatagram => {
                if rules
                    .iter()
                    .any(|rule| rule.matches_ip(addr.ip(), addr.port()))
                {
                    Decision::Allow
                } else if self.hostnames.iter().any(|slot| slot.covers(addr.port())) {
                    Decision::Lookup
                } else {
                    Decision::Deny(Refusal::AllowList)
                }
            }
        }
    }

    pub(crate) async fn permits_connect(&self, addr: SocketAddr) -> bool {
        let addr = canonical(addr);
        match self.decide(addr, SocketAddrUse::TcpConnect) {
            Decision::Allow => true,
            Decision::Deny(_) => false,
            Decision::Lookup => self.resolved_contains(addr).await,
        }
    }

    async fn resolved_contains(&self, addr: SocketAddr) -> bool {
        let ip = addr.ip();
        let candidates: Vec<&HostnameSlot> = self
            .hostnames
            .iter()
            .filter(|slot| slot.covers(addr.port()))
            .collect();
        for slot in &candidates {
            if slot.state.lock().await.ips.contains(&ip) {
                return true;
            }
        }
        for slot in candidates {
            let mut state = slot.state.lock().await;
            if state.ips.contains(&ip) {
                return true;
            }
            if state.stale() {
                self.refresh(slot, &mut state).await;
                if state.ips.contains(&ip) {
                    return true;
                }
            }
        }
        false
    }

    async fn refresh(&self, slot: &HostnameSlot, state: &mut Resolution) {
        state.attempted = Some(Instant::now());
        match self.lookup(&slot.name).await {
            Some(ips) if !ips.is_empty() => state.ips = ips,
            _ => tracing::debug!(
                plugin = %self.plugin_id,
                host = %slot.name,
                "wasm network allow-list: resolving the hostname rule failed; keeping the previous addresses"
            ),
        }
    }

    async fn lookup(&self, name: &str) -> Option<Vec<IpAddr>> {
        match tokio::time::timeout(self.lookup_timeout, self.resolver.resolve(name)).await {
            Ok(Ok(ips)) => Some(ips.into_iter().map(|ip| ip.to_canonical()).collect()),
            Ok(Err(error)) => {
                tracing::debug!(plugin = %self.plugin_id, host = name, %error, "wasm network lookup failed");
                None
            }
            Err(_) => {
                tracing::debug!(plugin = %self.plugin_id, host = name, "wasm network lookup timed out");
                None
            }
        }
    }

    pub(crate) async fn warm(&self) {
        for slot in &self.hostnames {
            let mut state = slot.state.lock().await;
            if state.stale() {
                self.refresh(slot, &mut state).await;
            }
        }
    }

    pub(crate) fn warm_in_background(self: &Arc<Self>) {
        if !self.has_hostnames() {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let policy = Arc::clone(self);
        runtime.spawn(async move { policy.warm().await }.with_current_subscriber());
    }

    pub(crate) fn route_http(&self, host: &str, port: u16) -> Result<HttpRoute, Refusal> {
        let (rules, dns, http) = match &self.mode {
            Mode::Off(refusal) => return Err(*refusal),
            Mode::On { rules, dns, http } => (rules, *dns, *http),
        };
        if !http {
            return Err(Refusal::HttpDisabled);
        }
        let reachable_by_ip = rules.iter().any(|rule| {
            matches!(rule.host(), HostPattern::Ip(_) | HostPattern::Net(_))
                && rule.ports().contains(port)
        }) || self.hostnames.iter().any(|slot| slot.covers(port));
        if let Some(ip) = ip_literal(host) {
            return if reachable_by_ip {
                Ok(HttpRoute::Literal(SocketAddr::new(ip.to_canonical(), port)))
            } else {
                Err(Refusal::AllowList)
            };
        }
        let name = host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase();
        if rules.iter().any(|rule| rule.matches_name(&name, port)) {
            return Ok(HttpRoute::Named {
                name,
                port,
                approved: true,
            });
        }
        if dns && reachable_by_ip {
            return Ok(HttpRoute::Named {
                name,
                port,
                approved: false,
            });
        }
        Err(Refusal::AllowList)
    }

    pub(crate) async fn resolve_http(
        &self,
        route: HttpRoute,
    ) -> Result<Vec<SocketAddr>, HttpResolveError> {
        match route {
            HttpRoute::Literal(addr) => {
                if self.permits_connect(addr).await {
                    Ok(vec![addr])
                } else {
                    Err(HttpResolveError::Denied)
                }
            }
            HttpRoute::Named {
                name,
                port,
                approved,
            } => {
                let ips = self.lookup(&name).await.ok_or(HttpResolveError::Lookup)?;
                let mut targets = Vec::with_capacity(ips.len());
                for ip in ips {
                    let addr = SocketAddr::new(ip, port);
                    if approved || self.permits_connect(addr).await {
                        targets.push(addr);
                    }
                }
                match (targets.is_empty(), approved) {
                    (false, _) => Ok(targets),
                    (true, true) => Err(HttpResolveError::Lookup),
                    (true, false) => Err(HttpResolveError::Denied),
                }
            }
        }
    }

    pub(crate) fn report_denied(
        &self,
        kind: &'static str,
        destination: impl Display,
        refusal: Refusal,
    ) {
        let Some(suppressed) = self.denials.admit(std::time::Instant::now()) else {
            return;
        };
        let reason = refusal.reason();
        tracing::warn!(
            plugin = %self.plugin_id,
            kind,
            destination = %destination,
            reason,
            suppressed,
            "wasm plugin network access refused by the {reason}: {kind} to {destination}"
        );
    }
}

fn canonical(addr: SocketAddr) -> SocketAddr {
    SocketAddr::new(addr.ip().to_canonical(), addr.port())
}

fn ip_literal(host: &str) -> Option<IpAddr> {
    let bare = host
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(host);
    bare.parse().ok()
}

fn usage_label(usage: SocketAddrUse) -> &'static str {
    match usage {
        SocketAddrUse::TcpBind => "tcp-bind",
        SocketAddrUse::TcpConnect => "tcp-connect",
        SocketAddrUse::UdpBind => "udp-bind",
        SocketAddrUse::UdpConnect => "udp-connect",
        SocketAddrUse::UdpOutgoingDatagram => "udp-send",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;

    use super::super::resolver::ResolveFuture;
    use super::*;

    #[derive(Default)]
    struct FakeResolver {
        answers: StdMutex<HashMap<String, Vec<IpAddr>>>,
        calls: StdMutex<Vec<String>>,
    }

    impl FakeResolver {
        fn answer(&self, name: &str, ips: &[&str]) {
            self.answers.lock().unwrap().insert(
                name.to_owned(),
                ips.iter().map(|ip| ip.parse().unwrap()).collect(),
            );
        }

        fn calls(&self) -> usize {
            self.calls.lock().unwrap().len()
        }
    }

    impl Resolver for FakeResolver {
        fn resolve(&self, name: &str) -> ResolveFuture {
            self.calls.lock().unwrap().push(name.to_owned());
            let answer = self.answers.lock().unwrap().get(name).cloned();
            Box::pin(async move { answer.ok_or_else(|| std::io::Error::other("no such host")) })
        }
    }

    fn network(allow: &[&str], dns: Option<bool>, http: bool) -> WasmNetworkConfig {
        WasmNetworkConfig {
            allow: allow.iter().map(|rule| rule.parse().unwrap()).collect(),
            dns,
            http,
        }
    }

    fn policy_with(allow: &[&str], resolver: &Arc<FakeResolver>) -> Arc<NetworkPolicy> {
        Arc::new(NetworkPolicy::new(
            "p".to_owned(),
            Some(&network(allow, None, true)),
            true,
            Duration::from_secs(5),
            Arc::clone(resolver) as Arc<dyn Resolver>,
        ))
    }

    async fn check(policy: &Arc<NetworkPolicy>, addr: SocketAddr, usage: SocketAddrUse) -> bool {
        (policy.socket_check())(addr, usage).await
    }

    fn addr(text: &str) -> SocketAddr {
        text.parse().unwrap()
    }

    const CONNECT: SocketAddrUse = SocketAddrUse::TcpConnect;

    #[tokio::test]
    async fn ip_rules_allow_without_any_lookup() {
        let resolver = Arc::new(FakeResolver::default());
        let policy = policy_with(&["127.0.0.1:5432", "10.0.0.0/8:3306", "[::1]:*"], &resolver);
        assert!(check(&policy, addr("127.0.0.1:5432"), CONNECT).await);
        assert!(
            check(
                &policy,
                addr("10.20.30.40:3306"),
                SocketAddrUse::UdpOutgoingDatagram
            )
            .await
        );
        assert!(check(&policy, addr("[::1]:9"), SocketAddrUse::UdpConnect).await);
        assert!(!check(&policy, addr("127.0.0.1:5433"), CONNECT).await);
        assert!(!check(&policy, addr("11.0.0.1:3306"), CONNECT).await);
        assert_eq!(resolver.calls(), 0);
    }

    #[tokio::test]
    async fn ipv4_mapped_ipv6_destinations_are_normalised_before_matching() {
        let resolver = Arc::new(FakeResolver::default());
        let policy = policy_with(&["127.0.0.1:5432"], &resolver);
        assert!(check(&policy, addr("[::ffff:127.0.0.1]:5432"), CONNECT).await);
        assert!(!check(&policy, addr("[::ffff:127.0.0.2]:5432"), CONNECT).await);

        let v6_only = policy_with(&["[::/0]:*"], &resolver);
        assert!(!check(&v6_only, addr("[::ffff:127.0.0.1]:80"), CONNECT).await);
        assert!(check(&v6_only, addr("[2001:db8::1]:80"), CONNECT).await);
    }

    #[tokio::test]
    async fn a_hostname_rule_allows_the_addresses_it_resolves_to_on_its_ports() {
        let resolver = Arc::new(FakeResolver::default());
        resolver.answer("db.internal", &["10.9.9.9", "::ffff:10.9.9.8"]);
        let policy = policy_with(&["db.internal:5432"], &resolver);
        assert!(check(&policy, addr("10.9.9.9:5432"), CONNECT).await);
        assert!(check(&policy, addr("10.9.9.8:5432"), CONNECT).await);
        assert_eq!(resolver.calls(), 1, "the second check used the cache");
        assert!(!check(&policy, addr("10.9.9.9:5433"), CONNECT).await);
        assert_eq!(resolver.calls(), 1, "a port no rule covers never resolves");
    }

    #[tokio::test(start_paused = true)]
    async fn a_miss_re_resolves_a_rule_at_most_once_per_thirty_seconds() {
        let resolver = Arc::new(FakeResolver::default());
        resolver.answer("db.internal", &["10.0.0.1"]);
        let policy = policy_with(&["db.internal:5432"], &resolver);

        assert!(!check(&policy, addr("10.0.0.2:5432"), CONNECT).await);
        assert_eq!(resolver.calls(), 1);
        resolver.answer("db.internal", &["10.0.0.2"]);
        assert!(!check(&policy, addr("10.0.0.2:5432"), CONNECT).await);
        tokio::time::advance(HOSTNAME_REFRESH_INTERVAL - Duration::from_millis(1)).await;
        assert!(!check(&policy, addr("10.0.0.2:5432"), CONNECT).await);
        assert_eq!(
            resolver.calls(),
            1,
            "misses inside the window do not re-resolve"
        );

        tokio::time::advance(Duration::from_millis(1)).await;
        assert!(check(&policy, addr("10.0.0.2:5432"), CONNECT).await);
        assert_eq!(resolver.calls(), 2);
        assert!(
            !check(&policy, addr("10.0.0.1:5432"), CONNECT).await,
            "the new answer replaced the old one"
        );
        assert_eq!(resolver.calls(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_resolution_keeps_the_previous_addresses() {
        let resolver = Arc::new(FakeResolver::default());
        resolver.answer("db.internal", &["10.0.0.1"]);
        let policy = policy_with(&["db.internal:5432"], &resolver);
        policy.warm().await;
        resolver.answers.lock().unwrap().clear();
        tokio::time::advance(HOSTNAME_REFRESH_INTERVAL).await;
        assert!(!check(&policy, addr("10.0.0.2:5432"), CONNECT).await);
        assert_eq!(resolver.calls(), 2);
        assert!(check(&policy, addr("10.0.0.1:5432"), CONNECT).await);
    }

    #[tokio::test(start_paused = true)]
    async fn warming_resolves_each_name_once_per_window() {
        let resolver = Arc::new(FakeResolver::default());
        resolver.answer("db.internal", &["10.0.0.1"]);
        resolver.answer("cache.internal", &["10.0.0.3"]);
        let policy = policy_with(
            &[
                "db.internal:5432",
                "db.internal:6432",
                "cache.internal:6379",
                "*.example.org:443",
            ],
            &resolver,
        );
        assert!(policy.has_hostnames());
        policy.warm().await;
        assert_eq!(
            resolver.calls(),
            2,
            "one lookup per distinct name, none for wildcards"
        );
        policy.warm().await;
        assert_eq!(resolver.calls(), 2);
        assert!(check(&policy, addr("10.0.0.1:6432"), CONNECT).await);
        assert_eq!(resolver.calls(), 2);
        tokio::time::advance(HOSTNAME_REFRESH_INTERVAL).await;
        policy.warm().await;
        assert_eq!(resolver.calls(), 4);
    }

    #[tokio::test]
    async fn binds_are_refused_unless_an_exact_rule_names_the_address() {
        let resolver = Arc::new(FakeResolver::default());
        resolver.answer("localhost", &["127.0.0.1"]);
        let policy = policy_with(&["127.0.0.1:9000", "0.0.0.0/0:*", "localhost:*"], &resolver);
        assert!(check(&policy, addr("127.0.0.1:9000"), SocketAddrUse::TcpBind).await);
        assert!(!check(&policy, addr("0.0.0.0:9000"), SocketAddrUse::TcpBind).await);
        assert!(!check(&policy, addr("127.0.0.1:0"), SocketAddrUse::TcpBind).await);
        assert!(!check(&policy, addr("127.0.0.1:9001"), SocketAddrUse::TcpBind).await);
        assert!(check(&policy, addr("0.0.0.0:0"), SocketAddrUse::UdpBind).await);
        assert!(check(&policy, addr("[::]:0"), SocketAddrUse::UdpBind).await);
        assert!(!check(&policy, addr("0.0.0.0:5000"), SocketAddrUse::UdpBind).await);
        assert!(check(&policy, addr("127.0.0.1:9000"), SocketAddrUse::UdpBind).await);
        assert_eq!(resolver.calls(), 0, "binds never consult hostname rules");
    }

    #[tokio::test]
    async fn policies_that_are_off_refuse_everything_and_never_resolve() {
        let resolver = Arc::new(FakeResolver::default());
        resolver.answer("localhost", &["127.0.0.1"]);
        let config = network(&["127.0.0.1:*", "localhost:*"], Some(true), true);
        let dyn_resolver = Arc::clone(&resolver) as Arc<dyn Resolver>;
        let missing = Arc::new(NetworkPolicy::new(
            "p".to_owned(),
            Some(&config),
            false,
            Duration::from_secs(1),
            Arc::clone(&dyn_resolver),
        ));
        let empty = Arc::new(NetworkPolicy::new(
            "p".to_owned(),
            Some(&network(&[], Some(true), true)),
            true,
            Duration::from_secs(1),
            Arc::clone(&dyn_resolver),
        ));
        let absent = Arc::new(NetworkPolicy::new(
            "p".to_owned(),
            None,
            true,
            Duration::from_secs(1),
            Arc::clone(&dyn_resolver),
        ));
        let probe = Arc::new(NetworkPolicy::disabled("probe".to_owned(), dyn_resolver));
        for policy in [&missing, &empty, &absent, &probe] {
            assert!(!policy.is_active());
            assert!(!policy.dns());
            assert!(!policy.has_hostnames());
            for usage in [
                SocketAddrUse::TcpConnect,
                SocketAddrUse::TcpBind,
                SocketAddrUse::UdpBind,
                SocketAddrUse::UdpConnect,
                SocketAddrUse::UdpOutgoingDatagram,
            ] {
                assert!(!check(policy, addr("127.0.0.1:0"), usage).await);
                assert!(!check(policy, addr("127.0.0.1:80"), usage).await);
            }
            policy.warm().await;
        }
        assert_eq!(
            missing.route_http("127.0.0.1", 80),
            Err(Refusal::MissingCapability)
        );
        assert_eq!(
            probe.route_http("localhost", 80),
            Err(Refusal::MissingCapability)
        );
        assert_eq!(empty.route_http("127.0.0.1", 80), Err(Refusal::AllowList));
        assert_eq!(absent.route_http("127.0.0.1", 80), Err(Refusal::AllowList));
        assert_eq!(resolver.calls(), 0);
    }

    #[tokio::test]
    async fn dns_follows_the_config_and_the_capability() {
        let resolver = Arc::new(FakeResolver::default()) as Arc<dyn Resolver>;
        let make = |allow: &[&str], dns: Option<bool>| {
            NetworkPolicy::new(
                "p".to_owned(),
                Some(&network(allow, dns, true)),
                true,
                Duration::from_secs(1),
                Arc::clone(&resolver),
            )
        };
        assert!(make(&["localhost:80"], None).dns());
        assert!(!make(&["localhost:80"], Some(false)).dns());
        assert!(!make(&["127.0.0.1:80"], None).dns());
        assert!(make(&["127.0.0.1:80"], Some(true)).dns());
    }

    #[tokio::test]
    async fn http_routes_follow_the_rules() {
        let resolver = Arc::new(FakeResolver::default());
        let policy = NetworkPolicy::new(
            "p".to_owned(),
            Some(&network(
                &[
                    "127.0.0.1:8080",
                    "api.example.com:443",
                    "*.example.org:443",
                    "[::1]:80",
                ],
                Some(false),
                true,
            )),
            true,
            Duration::from_secs(1),
            Arc::clone(&resolver) as Arc<dyn Resolver>,
        );
        assert_eq!(
            policy.route_http("127.0.0.1", 8080),
            Ok(HttpRoute::Literal(addr("127.0.0.1:8080")))
        );
        assert_eq!(
            policy.route_http("[::1]", 80),
            Ok(HttpRoute::Literal(addr("[::1]:80")))
        );
        assert_eq!(
            policy.route_http("127.0.0.1", 9090),
            Err(Refusal::AllowList)
        );
        assert_eq!(
            policy.route_http("API.example.com.", 443),
            Ok(HttpRoute::Named {
                name: "api.example.com".to_owned(),
                port: 443,
                approved: true
            })
        );
        assert!(matches!(
            policy.route_http("cdn.example.org", 443),
            Ok(HttpRoute::Named { approved: true, .. })
        ));
        assert_eq!(
            policy.route_http("example.org", 443),
            Err(Refusal::AllowList)
        );
        assert_eq!(
            policy.route_http("api.example.com", 80),
            Err(Refusal::AllowList)
        );
        assert_eq!(
            policy.route_http("elsewhere.test", 8080),
            Err(Refusal::AllowList),
            "without dns an unlisted name is not even resolved"
        );

        let with_dns = NetworkPolicy::new(
            "p".to_owned(),
            Some(&network(&["10.0.0.0/8:80"], Some(true), true)),
            true,
            Duration::from_secs(1),
            Arc::clone(&resolver) as Arc<dyn Resolver>,
        );
        assert_eq!(
            with_dns.route_http("svc.local", 80),
            Ok(HttpRoute::Named {
                name: "svc.local".to_owned(),
                port: 80,
                approved: false
            })
        );
        assert_eq!(
            with_dns.route_http("svc.local", 443),
            Err(Refusal::AllowList)
        );

        let no_http = NetworkPolicy::new(
            "p".to_owned(),
            Some(&network(&["127.0.0.1:8080"], None, false)),
            true,
            Duration::from_secs(1),
            Arc::clone(&resolver) as Arc<dyn Resolver>,
        );
        assert_eq!(
            no_http.route_http("127.0.0.1", 8080),
            Err(Refusal::HttpDisabled)
        );
        assert_eq!(resolver.calls(), 0);
    }

    #[tokio::test]
    async fn an_unlisted_name_only_reaches_the_addresses_the_ip_rules_allow() {
        let resolver = Arc::new(FakeResolver::default());
        resolver.answer("svc.local", &["192.168.1.1", "10.0.0.5"]);
        resolver.answer("outside.local", &["192.168.1.1"]);
        resolver.answer("api.example.com", &["203.0.113.7"]);
        let policy = NetworkPolicy::new(
            "p".to_owned(),
            Some(&network(
                &["10.0.0.0/8:80", "api.example.com:443"],
                Some(true),
                true,
            )),
            true,
            Duration::from_secs(1),
            Arc::clone(&resolver) as Arc<dyn Resolver>,
        );
        let route = policy.route_http("svc.local", 80).unwrap();
        assert_eq!(
            policy.resolve_http(route).await,
            Ok(vec![addr("10.0.0.5:80")])
        );
        let route = policy.route_http("192.168.1.1", 80).unwrap();
        assert_eq!(
            policy.resolve_http(route).await,
            Err(HttpResolveError::Denied),
            "an IP literal on a covered port still has to match a rule"
        );
        let route = policy.route_http("10.1.2.3", 80).unwrap();
        assert_eq!(
            policy.resolve_http(route).await,
            Ok(vec![addr("10.1.2.3:80")])
        );
        let route = policy.route_http("outside.local", 80).unwrap();
        assert_eq!(
            policy.resolve_http(route).await,
            Err(HttpResolveError::Denied)
        );
        let route = policy.route_http("api.example.com", 443).unwrap();
        assert_eq!(
            policy.resolve_http(route).await,
            Ok(vec![addr("203.0.113.7:443")]),
            "a name the rules approve may resolve anywhere"
        );
        let route = policy.route_http("missing.local", 80).unwrap();
        assert_eq!(
            policy.resolve_http(route).await,
            Err(HttpResolveError::Lookup)
        );
    }
}
