//! Trusted-proxy CIDR list and client IP resolution.
//!
//! Set `OXICLOUD_TRUST_PROXY_CIDR` to a comma-separated list of CIDR blocks
//! whose source IPs are trusted to set `X-Forwarded-For` / `X-Real-Ip`:
//!
//! ```text
//! OXICLOUD_TRUST_PROXY_CIDR=127.0.0.1/32,10.0.0.0/8,172.16.0.0/12,::1/128
//! ```
//!
//! When the TCP peer address falls inside one of those CIDRs the effective
//! client IP is the **rightmost `X-Forwarded-For` entry that is not itself a
//! trusted proxy** — the last address in the chain that no proxy we trust could
//! have appended. Walking from the right is what makes the value unforgeable: a
//! proxy appends the peer it saw (`X-Forwarded-For: <client-supplied...>,
//! <real client>`, nginx's default `$proxy_add_x_forwarded_for`), so every entry
//! to the left of the rightmost untrusted hop is attacker-supplied text. Reading
//! the **leftmost** entry instead — the previous behaviour — let any client pick
//! its own rate-limit bucket, and pick a victim's address to lock that victim
//! out.
//!
//! Entries are parsed and compared in normalised form, so `::ffff:203.0.113.9`
//! and `203.0.113.9` are one client, and the resolved value is rendered
//! canonically (`203.0.113.9`, `2001:db8::1`). An entry that is not a parseable
//! IP is skipped, not fatal: junk can only appear in the attacker-supplied
//! region, and a chain of junk still resolves to the peer.
//!
//! Resolution order (all of it gated on the peer being trusted — when it is not,
//! or when there is no peer, the headers are ignored entirely):
//! 1. `X-Forwarded-For`, right to left: the first entry that parses as an IP and
//!    is not inside a trusted CIDR.
//! 2. Otherwise `X-Real-Ip`, by the same rule (a single value; proxies overwrite
//!    it rather than appending).
//! 3. Otherwise the TCP peer itself.
//!
//! Step 3 is the "nothing identifies the client" case — an empty chain, a chain
//! of nothing but trusted hops (which includes the legacy
//! `OXICLOUD_TRUST_PROXY_HEADERS=1` trust-all mode, where every hop is inside
//! `0.0.0.0/0`), or a chain of junk. Those deployments key the limiter on the
//! proxy's address: one bucket for everyone. That is the honest answer for
//! "cannot tell clients apart", but it is a shared bucket — a CIDR list that is
//! narrower than the real proxy set, or a trust-all list, is a configuration
//! bug whose symptom is 429s shared across unrelated clients.
//!
//! IPv4-mapped IPv6 addresses (`::ffff:x.x.x.x`) are automatically
//! normalised to their IPv4 equivalent before CIDR lookup.

use axum::extract::ConnectInfo;
use axum::http::Request;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::OnceLock;

// ─── CIDR list ───────────────────────────────────────────────────────────────

static TRUSTED_CIDRS: OnceLock<Vec<(IpAddr, u8)>> = OnceLock::new();

fn trusted_cidrs() -> &'static [(IpAddr, u8)] {
    TRUSTED_CIDRS.get_or_init(|| {
        let mut cidrs: Vec<(IpAddr, u8)> = std::env::var("OXICLOUD_TRUST_PROXY_CIDR")
            .unwrap_or_default()
            .split(',')
            .filter_map(|s| parse_cidr(s.trim()))
            .collect();

        // Backward-compat: OXICLOUD_TRUST_PROXY_HEADERS=true trusts all source IPs.
        let legacy = std::env::var("OXICLOUD_TRUST_PROXY_HEADERS")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);
        if legacy {
            tracing::warn!(
                "OXICLOUD_TRUST_PROXY_HEADERS is deprecated — \
                 use OXICLOUD_TRUST_PROXY_CIDR to restrict trusted proxy source IPs. \
                 Falling back to trust all IPs (0.0.0.0/0 and ::/0)."
            );
            cidrs.push(("0.0.0.0".parse().unwrap(), 0));
            cidrs.push(("::".parse().unwrap(), 0));
        }

        cidrs
    })
}

fn parse_cidr(s: &str) -> Option<(IpAddr, u8)> {
    let (addr_s, prefix_s) = s.split_once('/')?;
    let addr: IpAddr = addr_s.parse().ok()?;
    let prefix: u8 = prefix_s.parse().ok()?;
    let max = if addr.is_ipv4() { 32 } else { 128 };
    if prefix > max {
        return None;
    }
    Some((addr, prefix))
}

// ─── CIDR matching ───────────────────────────────────────────────────────────

fn ipv4_in_cidr(base: Ipv4Addr, prefix: u8, ip: Ipv4Addr) -> bool {
    if prefix == 0 {
        return true;
    }
    let shift = 32 - u32::from(prefix);
    let mask = !0u32 << shift;
    (u32::from(base) & mask) == (u32::from(ip) & mask)
}

fn ipv6_in_cidr(base: Ipv6Addr, prefix: u8, ip: Ipv6Addr) -> bool {
    if prefix == 0 {
        return true;
    }
    let shift = 128 - u32::from(prefix);
    let mask = !0u128 << shift;
    (u128::from(base) & mask) == (u128::from(ip) & mask)
}

/// Normalise an IP to IPv4 if it is an IPv4-mapped IPv6 address.
fn normalise(ip: IpAddr) -> IpAddr {
    if let IpAddr::V6(v6) = ip
        && let Some(v4) = v6.to_ipv4_mapped()
    {
        return IpAddr::V4(v4);
    }
    ip
}

fn cidr_contains(base: IpAddr, prefix: u8, target: IpAddr) -> bool {
    let base = normalise(base);
    let target = normalise(target);
    match (base, target) {
        (IpAddr::V4(b), IpAddr::V4(t)) => ipv4_in_cidr(b, prefix, t),
        (IpAddr::V6(b), IpAddr::V6(t)) => ipv6_in_cidr(b, prefix, t),
        _ => false,
    }
}

/// Logs the loaded CIDR list at INFO level.  Call once at startup, after the
/// tracing subscriber is initialised, to make the effective configuration
/// visible in the logs.
pub fn log_config() {
    let cidrs = trusted_cidrs();
    if cidrs.is_empty() {
        tracing::info!(
            "trusted proxy CIDRs: none — X-Forwarded-For/X-Real-Ip headers will be ignored"
        );
    } else {
        let list: Vec<String> = cidrs
            .iter()
            .map(|(addr, prefix)| format!("{addr}/{prefix}"))
            .collect();
        tracing::info!("trusted proxy CIDRs: {}", list.join(", "));
    }
}

/// Returns `true` when `peer` falls inside one of the configured CIDR ranges.
pub fn is_trusted_proxy(peer: IpAddr) -> bool {
    let cidrs = trusted_cidrs();
    if cidrs.is_empty() {
        return false;
    }
    cidrs
        .iter()
        .any(|&(base, prefix)| cidr_contains(base, prefix, peer))
}

// ─── IP extraction ───────────────────────────────────────────────────────────

/// Resolve the effective client IP for a request.
///
/// * `include_port` — when `true` the returned string for direct (non-proxied)
///   connections includes the port, e.g. `"127.0.0.1:12345"`.  Pass `false`
///   for contexts that only need the bare IP (e.g. rate-limiting keys).
pub fn client_ip<B>(req: &Request<B>, include_port: bool) -> String {
    let peer: Option<SocketAddr> = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0);

    client_ip_from_parts(req.headers(), peer, include_port)
}

/// The address a trusted proxy reported for the client, or `None` when the
/// presented chain does not identify one.
///
/// * `X-Forwarded-For` first, right to left: the first entry that parses as an
///   IP and does not itself belong to a trusted proxy. Everything left of that
///   entry is attacker-supplied (see the module docs), so it is only ever
///   returned when nothing to its right qualified.
/// * Then `X-Real-Ip` — a single value, overwritten by a proxy rather than
///   appended — under the same rule: it is the client only if it is not itself
///   a trusted proxy.
///
/// `None` is the answer for every chain that identifies nothing — absent,
/// empty, all trusted hops, or nothing parseable — and each caller turns it
/// into the peer fallback.
fn forwarded_client_ip(headers: &axum::http::HeaderMap) -> Option<IpAddr> {
    if let Some(raw) = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
        && let Some(ip) = raw
            .rsplit(',')
            .filter_map(|entry| entry.trim().parse::<IpAddr>().ok())
            .map(normalise)
            .find(|ip| !is_trusted_proxy(*ip))
    {
        return Some(ip);
    }

    let xri: IpAddr = headers
        .get("x-real-ip")?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let xri = normalise(xri);
    (!is_trusted_proxy(xri)).then_some(xri)
}

/// A resolved client-IP source, rendered on demand instead of through an
/// intermediate `String`. [`std::fmt::Display`] writes it straight into the
/// caller's buffer (the tracing span's field storage), so the per-request span
/// factory no longer materializes an owned `String` on every request
/// (benches/ROUND13.md §H2). Bytes rendered are identical to
/// [`client_ip`]/[`client_ip_from_parts`] for all four cases.
pub enum ClientIpDisplay {
    /// Proxy-forwarded client address, canonicalised (an `IpAddr`, so the value
    /// is `Copy` and nothing is borrowed from the request).
    ForwardedIp(IpAddr),
    /// Direct TCP peer, rendered with the port.
    PeerWithPort(SocketAddr),
    /// Direct TCP peer, rendered as the bare IP.
    PeerIp(IpAddr),
    /// No connection info available.
    Unknown,
}

impl std::fmt::Display for ClientIpDisplay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientIpDisplay::ForwardedIp(ip) => write!(f, "{ip}"),
            ClientIpDisplay::PeerWithPort(addr) => write!(f, "{addr}"),
            ClientIpDisplay::PeerIp(ip) => write!(f, "{ip}"),
            ClientIpDisplay::Unknown => f.write_str("unknown"),
        }
    }
}

/// Allocation-free twin of [`client_ip_from_parts`]: resolves the client-IP
/// source without producing an owned `String`. The returned value owns
/// everything it renders (an `IpAddr`/`SocketAddr`, both `Copy`), and returns
/// exactly the bytes [`client_ip_from_parts`] would.
pub fn client_ip_display_from_parts(
    headers: &axum::http::HeaderMap,
    peer: Option<SocketAddr>,
    include_port: bool,
) -> ClientIpDisplay {
    let Some(peer_addr) = peer else {
        return ClientIpDisplay::Unknown;
    };
    if is_trusted_proxy(peer_addr.ip())
        && let Some(ip) = forwarded_client_ip(headers)
    {
        return ClientIpDisplay::ForwardedIp(ip);
    }
    if include_port {
        ClientIpDisplay::PeerWithPort(peer_addr)
    } else {
        ClientIpDisplay::PeerIp(peer_addr.ip())
    }
}

/// Zero-allocation twin of [`client_ip`] for the request-span factory.
pub fn client_ip_display<B>(req: &Request<B>, include_port: bool) -> ClientIpDisplay {
    let peer: Option<SocketAddr> = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0);
    client_ip_display_from_parts(req.headers(), peer, include_port)
}

/// Same as [`client_ip`], but operates on already-extracted parts (headers
/// plus an optional TCP peer).  Handlers that don't take a full `Request<B>`,
/// e.g. those that consume the body via `Json<...>`, can still derive a stable
/// client identifier with this entry point.
pub fn client_ip_from_parts(
    headers: &axum::http::HeaderMap,
    peer: Option<SocketAddr>,
    include_port: bool,
) -> String {
    let Some(peer_addr) = peer else {
        return "unknown".to_string();
    };
    if is_trusted_proxy(peer_addr.ip())
        && let Some(ip) = forwarded_client_ip(headers)
    {
        return ip.to_string();
    }
    if include_port {
        peer_addr.to_string()
    } else {
        peer_addr.ip().to_string()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interfaces::middleware::rate_limit::RateLimiter;
    use std::sync::Arc;

    #[test]
    fn ipv4_cidr_host() {
        let base: Ipv4Addr = "192.168.1.1".parse().unwrap();
        assert!(ipv4_in_cidr(base, 32, "192.168.1.1".parse().unwrap()));
        assert!(!ipv4_in_cidr(base, 32, "192.168.1.2".parse().unwrap()));
    }

    #[test]
    fn ipv4_cidr_subnet() {
        let base: Ipv4Addr = "10.0.0.0".parse().unwrap();
        assert!(ipv4_in_cidr(base, 8, "10.255.255.255".parse().unwrap()));
        assert!(!ipv4_in_cidr(base, 8, "11.0.0.1".parse().unwrap()));
    }

    #[test]
    fn ipv4_cidr_any() {
        let base: Ipv4Addr = "0.0.0.0".parse().unwrap();
        assert!(ipv4_in_cidr(base, 0, "1.2.3.4".parse().unwrap()));
    }

    #[test]
    fn ipv6_cidr_loopback() {
        let base: Ipv6Addr = "::1".parse().unwrap();
        assert!(ipv6_in_cidr(base, 128, "::1".parse().unwrap()));
        assert!(!ipv6_in_cidr(base, 128, "::2".parse().unwrap()));
    }

    #[test]
    fn ipv4_mapped_matches_v4_cidr() {
        // ::ffff:127.0.0.1 should match 127.0.0.1/8
        let base = IpAddr::V4("127.0.0.0".parse().unwrap());
        let mapped = IpAddr::V6("::ffff:127.0.0.1".parse().unwrap());
        assert!(cidr_contains(base, 8, mapped));
    }

    #[test]
    fn parse_cidr_valid() {
        assert_eq!(
            parse_cidr("10.0.0.0/8"),
            Some((IpAddr::V4("10.0.0.0".parse().unwrap()), 8))
        );
        assert_eq!(
            parse_cidr("::1/128"),
            Some((IpAddr::V6("::1".parse().unwrap()), 128))
        );
    }

    #[test]
    fn parse_cidr_invalid_prefix() {
        assert!(parse_cidr("10.0.0.0/33").is_none());
        assert!(parse_cidr("::1/129").is_none());
        assert!(parse_cidr("notanip/8").is_none());
    }

    // ── Client-IP resolution under a trust list ──────────────────────────────
    //
    // `trusted_cidrs()` is a process-wide `OnceLock`, so an inline test that sets
    // `OXICLOUD_TRUST_PROXY_CIDR` fixes the CIDR list for the whole test binary —
    // and would race the rest of the suite. Instead the parent run re-execs the
    // test binary with the variable installed and asserts on the child's result;
    // only the child evaluates resolution, in a process whose list is known.
    // Nothing outside the child calls `client_ip*`/`is_trusted_proxy`, so the
    // parent never initialises the list at all.

    /// Set in the child so it knows it is the child.
    const CHILD_FLAG: &str = "OXICLOUD_TRUST_PROXY_TEST_CHILD";
    /// The child's trust list: loopback (proxy on the same host — the usual
    /// deployment) plus a second range so a chain can show two trusted hops.
    const CHILD_CIDRS: &str = "127.0.0.1/32,10.0.0.0/8";

    /// Re-execs this test binary with `CHILD_CIDRS` installed, running only
    /// `test_name`.
    fn assert_in_child(test_name: &str) {
        let exe = std::env::current_exe().expect("test binary path");
        let out = std::process::Command::new(exe)
            .arg(test_name)
            .env(CHILD_FLAG, "1")
            .env("OXICLOUD_TRUST_PROXY_CIDR", CHILD_CIDRS)
            .output()
            .expect("re-exec the test binary");
        let stdout = String::from_utf8_lossy(&out.stdout);
        // "1 passed" is load-bearing: a filter that matched nothing would exit 0
        // and the child's assertions would never run at all.
        assert!(
            out.status.success() && stdout.contains("1 passed"),
            "child run of {test_name} failed ({:?}); stdout:\n{stdout}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr),
        );
    }

    /// `true` when running as the child (with the trust list verified), `false`
    /// in the parent after it has run the child.
    fn in_child(test_name: &str) -> bool {
        if std::env::var_os(CHILD_FLAG).is_none() {
            assert_in_child(test_name);
            return false;
        }
        // Guard against a stale or global list: the assertions below only mean
        // anything if the peer is trusted and the spoofed client is not.
        assert!(
            is_trusted_proxy("127.0.0.1".parse().unwrap())
                && is_trusted_proxy("10.0.0.5".parse().unwrap())
                && !is_trusted_proxy("203.0.113.9".parse().unwrap()),
            "the child must have OXICLOUD_TRUST_PROXY_CIDR={CHILD_CIDRS} in effect"
        );
        true
    }

    fn headers(pairs: &[(&str, &str)]) -> axum::http::HeaderMap {
        let mut h = axum::http::HeaderMap::new();
        for (name, value) in pairs {
            h.insert(
                name.parse::<axum::http::HeaderName>().expect("header name"),
                value
                    .parse::<axum::http::HeaderValue>()
                    .expect("header value"),
            );
        }
        h
    }

    /// The resolution matrix, from a trusted peer.
    ///
    /// RED on v0.10.0: the multi-hop cases return the client-supplied leftmost
    /// entry (`6.6.6.6`), the mapped/IPv6 cases return the raw text rather than
    /// the normalised address, and an all-trusted chain returns a hop instead of
    /// the peer. GREEN after `0003-trusted-proxy-rightmost.patch`.
    #[test]
    fn client_ip_resolves_the_rightmost_untrusted_hop() {
        const NAME: &str = "client_ip_resolves_the_rightmost_untrusted_hop";
        if !in_child(NAME) {
            return;
        }

        // Inside 127.0.0.1/32.
        let peer: SocketAddr = "127.0.0.1:1234".parse().unwrap();
        // Inside nothing: a direct client, whose headers must be ignored.
        let direct: SocketAddr = "203.0.113.9:51000".parse().unwrap();
        let ip = |h: &axum::http::HeaderMap| client_ip_from_parts(h, Some(peer), false);
        let ip_port = |h: &axum::http::HeaderMap| client_ip_from_parts(h, Some(peer), true);

        // 1. Single hop: the trusted proxy appended the only entry there is.
        assert_eq!(
            ip(&headers(&[("x-forwarded-for", "203.0.113.9")])),
            "203.0.113.9"
        );

        // 2. Multi-hop, client first, one trusted middle hop.
        assert_eq!(
            ip(&headers(&[("x-forwarded-for", "203.0.113.9, 10.0.0.5")])),
            "203.0.113.9"
        );

        // 3. THE REGRESSION: a client-supplied prefix must not become the key.
        //    The leftmost rule returned "6.6.6.6" here, so any client picked its
        //    own rate-limit bucket — or spent a victim's by naming it.
        assert_eq!(
            ip(&headers(&[("x-forwarded-for", "6.6.6.6, 203.0.113.9")])),
            "203.0.113.9"
        );
        // 4. Several trusted hops to the right of the client, plus junk after
        //    them: still the rightmost untrusted address.
        assert_eq!(
            ip(&headers(&[(
                "x-forwarded-for",
                "6.6.6.6, 203.0.113.9, 10.0.0.5, 10.0.0.6, junk"
            )])),
            "203.0.113.9"
        );

        // 5. Whitespace and empty entries.
        assert_eq!(
            ip(&headers(&[("x-forwarded-for", "  203.0.113.9 , 10.0.0.5")])),
            "203.0.113.9"
        );
        assert_eq!(
            ip(&headers(&[("x-forwarded-for", ",203.0.113.9,")])),
            "203.0.113.9"
        );

        // 6. Malformed entries are skipped, not fatal.
        assert_eq!(
            ip(&headers(&[("x-forwarded-for", "junk, 203.0.113.9")])),
            "203.0.113.9"
        );
        // Known limitation, pinned deliberately: an entry carrying a port does
        // not parse, so a proxy configured with `$remote_addr:$remote_port`
        // leaves only attacker-supplied text to its left.
        assert_eq!(
            ip(&headers(&[("x-forwarded-for", "6.6.6.6, 203.0.113.9:443")])),
            "6.6.6.6"
        );

        // 7. IPv6 and IPv4-mapped entries: normalised for comparison and for
        //    rendering, so two spellings of one address are one client (one
        //    bucket) rather than two.
        assert_eq!(
            ip(&headers(&[("x-forwarded-for", "2001:0DB8::1, 10.0.0.5")])),
            "2001:db8::1"
        );
        assert_eq!(
            ip(&headers(&[(
                "x-forwarded-for",
                "::ffff:203.0.113.9, 10.0.0.5"
            )])),
            "203.0.113.9"
        );
        assert_eq!(
            ip(&headers(&[("x-forwarded-for", "::ffff:203.0.113.9")])),
            ip(&headers(&[("x-forwarded-for", "203.0.113.9")])),
            "a mapped client and its IPv4 spelling must resolve to one key"
        );
        // ...and a mapped TRUSTED hop is recognised as trusted, so it is skipped
        // like any other hop, leaving the client to its left.
        assert_eq!(
            ip(&headers(&[(
                "x-forwarded-for",
                "203.0.113.9, ::ffff:10.0.0.5"
            )])),
            "203.0.113.9"
        );

        // 8. Nothing identifies the client — every hop is trusted, or nothing
        //    parses — so the peer is the answer, with or without its port.
        assert_eq!(
            ip(&headers(&[("x-forwarded-for", "10.0.0.5, 10.0.0.6")])),
            "127.0.0.1"
        );
        assert_eq!(
            ip_port(&headers(&[("x-forwarded-for", "10.0.0.5")])),
            "127.0.0.1:1234"
        );
        assert_eq!(
            ip(&headers(&[("x-forwarded-for", "junk, more junk")])),
            "127.0.0.1"
        );

        // 9. X-Real-Ip fallback, under the same trust rule.
        assert_eq!(ip(&headers(&[("x-real-ip", "203.0.113.9")])), "203.0.113.9");
        assert_eq!(ip(&headers(&[("x-real-ip", "10.0.0.5")])), "127.0.0.1");
        assert_eq!(ip(&headers(&[("x-real-ip", "junk")])), "127.0.0.1");
        // X-Forwarded-For wins while it resolves; X-Real-Ip is the fallback.
        assert_eq!(
            ip(&headers(&[
                ("x-forwarded-for", "203.0.113.9"),
                ("x-real-ip", "6.6.6.6")
            ])),
            "203.0.113.9"
        );
        assert_eq!(
            ip(&headers(&[
                ("x-forwarded-for", "junk"),
                ("x-real-ip", "203.0.113.9")
            ])),
            "203.0.113.9"
        );

        // 10. Untrusted peer: every header is ignored, and a peerless call is
        //     still "unknown".
        assert_eq!(
            client_ip_from_parts(
                &headers(&[("x-forwarded-for", "6.6.6.6")]),
                Some(direct),
                false
            ),
            "203.0.113.9"
        );
        assert_eq!(
            client_ip_from_parts(&headers(&[("x-real-ip", "6.6.6.6")]), Some(direct), false),
            "203.0.113.9"
        );
        assert_eq!(
            client_ip_from_parts(&headers(&[("x-forwarded-for", "6.6.6.6")]), None, false),
            "unknown"
        );

        // 11. The Display twin renders the same bytes in every case above — the
        //     tracing span is built from it, so a divergence would log a
        //     different client than the limiter keys on.
        let multi_hop = headers(&[("x-forwarded-for", "6.6.6.6, 203.0.113.9, 10.0.0.5")]);
        assert_eq!(
            client_ip_display_from_parts(&multi_hop, Some(peer), false).to_string(),
            "203.0.113.9",
            "multi-hop chain with a trusted prefix: the resolved hop, not the \
             spoofed prefix"
        );
        let cases: Vec<(axum::http::HeaderMap, Option<SocketAddr>)> = vec![
            (multi_hop.clone(), Some(peer)),
            (headers(&[("x-forwarded-for", "203.0.113.9")]), Some(peer)),
            (
                headers(&[("x-forwarded-for", "  203.0.113.9 , 10.0.0.5")]),
                Some(peer),
            ),
            (
                headers(&[("x-forwarded-for", "junk, 203.0.113.9")]),
                Some(peer),
            ),
            (
                headers(&[("x-forwarded-for", "6.6.6.6, 203.0.113.9:443")]),
                Some(peer),
            ),
            (headers(&[("x-forwarded-for", "2001:0DB8::1")]), Some(peer)),
            (
                headers(&[("x-forwarded-for", "::ffff:203.0.113.9")]),
                Some(peer),
            ),
            (
                headers(&[("x-forwarded-for", "10.0.0.5, 10.0.0.6")]),
                Some(peer),
            ),
            (headers(&[("x-real-ip", "203.0.113.9")]), Some(peer)),
            (headers(&[("x-real-ip", "10.0.0.5")]), Some(peer)),
            (headers(&[("x-forwarded-for", "6.6.6.6")]), Some(direct)),
            (headers(&[("x-forwarded-for", "6.6.6.6")]), None),
            (axum::http::HeaderMap::new(), Some(peer)),
        ];
        for (hdrs, p) in &cases {
            for include_port in [false, true] {
                assert_eq!(
                    client_ip_display_from_parts(hdrs, *p, include_port).to_string(),
                    client_ip_from_parts(hdrs, *p, include_port),
                    "display twin diverged for {hdrs:?} / {p:?} / include_port={include_port}"
                );
            }
        }
    }

    /// The rate-limit consequence, which is why this rule matters: one client is
    /// one key, however it spells the chain.
    ///
    /// RED on v0.10.0: the three spoofed spellings below were three different
    /// keys, so one client rotated through three budgets (bypass) or spent a
    /// victim's bucket by naming the victim's address. GREEN after
    /// `0003-trusted-proxy-rightmost.patch` — the prefix no longer reaches the
    /// limiter.
    #[test]
    fn spoofed_prefix_cannot_choose_the_rate_limit_key() {
        const NAME: &str = "spoofed_prefix_cannot_choose_the_rate_limit_key";
        if !in_child(NAME) {
            return;
        }

        let peer: SocketAddr = "127.0.0.1:1234".parse().unwrap();
        let key = |chain: &str| {
            client_ip_from_parts(&headers(&[("x-forwarded-for", chain)]), Some(peer), false)
        };

        let a = key("6.6.6.6, 203.0.113.9");
        let b = key("7.7.7.7, 203.0.113.9");
        let c = key("9.9.9.9, 203.0.113.9");
        assert_eq!(a, "203.0.113.9", "the appended hop is the client");
        assert_eq!(a, b, "same client, different spoofed prefix → same key");
        assert_eq!(b, c, "…and again");

        // One key is one budget: the second request through any spelling is
        // rejected, so rotating the prefix buys nothing.
        let limiter = Arc::new(RateLimiter::new(1, 60, 100));
        assert!(limiter.check_and_increment(&a).is_ok());
        assert!(
            limiter.check_and_increment(&b).is_err(),
            "the spoofed prefix must not open a second budget"
        );

        // A different real client still gets its own bucket.
        assert!(
            limiter
                .check_and_increment(&key("6.6.6.6, 203.0.113.10"))
                .is_ok()
        );
    }
}
