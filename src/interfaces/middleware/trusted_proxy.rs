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

#[cfg(test)] #[rustfmt::skip] mod tests;
