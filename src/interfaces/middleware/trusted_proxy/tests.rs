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

        // 2b. Duplicate `X-Forwarded-For` LINES carve out nothing either: the
        //     header map holds two values and only the first is read, so the
        //     second line is invisible. Pinned as a known limitation — a client
        //     that prepends its own line ahead of a proxy that appends a second
        //     one keeps the value it sent. `$proxy_add_x_forwarded_for` (and
        //     Traefik's default) *replaces* the header, so the documented setup
        //     never produces two lines.
        let mut duplicated = headers(&[("x-forwarded-for", "1.1.1.1")]);
        duplicated.append(
            "x-forwarded-for".parse::<axum::http::HeaderName>().unwrap(),
            "2.2.2.2".parse::<axum::http::HeaderValue>().unwrap(),
        );
        assert_eq!(
            ip(&duplicated),
            "1.1.1.1",
            "only the first X-Forwarded-For line is read (known limitation)"
        );

        // 3. THE REGRESSION: a client-supplied prefix must not become the key.
        //    The leftmost rule returned "1.1.1.1" here, so any client picked its
        //    own rate-limit bucket — or spent a victim's by naming it.
        assert_eq!(
            ip(&headers(&[("x-forwarded-for", "1.1.1.1, 2.2.2.2")])),
            "2.2.2.2"
        );
        // 3b. Same rule with a second spoofed prefix and a second real client.
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
