//! Policy pin for the `quinn` advisories that reach this workspace through `reqwest`.
//!
//! This is a **policy** test, not an exploit. It reads the committed `Cargo.lock` and refuses the affected
//! version ranges, so a later `cargo update` cannot silently reintroduce them. It asserts nothing about
//! reachability: `quinn-udp`'s receive-loop spin needs the non-`recvmmsg` fallback path (Apple platforms without
//! `fast-apple-datapath`, OpenBSD/NetBSD/Solaris/illumos, and from 0.6 DragonFly/Redox/GNU-Hurd), which the Linux
//! deployment does not take. The floor is still worth holding, because this workspace builds for other targets
//! and an audit everyone has learned to ignore is worse than no audit.
//!
//! Advisories pinned here:
//! - **RUSTSEC-2026-0538** / GHSA-6pp4-4cxf-xf88 (`quinn-udp`): fixed in `0.5.16` and `0.6.3`.
//! - **RUSTSEC-2026-0524** (`quinn-proto`): fixed in `0.11.19`.
//!
//! A dependency that leaves the tree makes this test fail deliberately — removing the pin is then a conscious act
//! with a review trail, rather than a silent loss of coverage.

use std::path::PathBuf;

fn parse_version(raw: &str) -> (u64, u64, u64) {
    let mut parts = raw.split('.').map(|p| {
        p.parse::<u64>()
            .unwrap_or_else(|_| panic!("unparsable version {raw}"))
    });
    (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    )
}

/// **Every** version of `krate` the lock resolves — not just the first. A lock can carry two generations of a
/// crate, and a first-match reader is exactly how an affected one hides behind a fixed one.
fn resolved_versions(lock: &toml::Value, krate: &str) -> Vec<(u64, u64, u64)> {
    lock.get("package")
        .and_then(|p| p.as_array())
        .map(|pkgs| {
            pkgs.iter()
                .filter(|p| p.get("name").and_then(|n| n.as_str()) == Some(krate))
                .filter_map(|p| p.get("version").and_then(|v| v.as_str()).map(parse_version))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// `Err` if any resolved generation of `krate` sits on `line` below `floor`.
fn policy_check(
    lock: &toml::Value,
    krate: &str,
    line: (u64, u64),
    floor: u64,
    advisory: &str,
) -> Result<(), String> {
    let found = resolved_versions(lock, krate);
    if found.is_empty() {
        return Err(format!(
            "{krate} is not in the lockfile — if the dependency really left the tree, remove this pin and say so \
             in the commit that removes it"
        ));
    }
    for (major, minor, patch) in found {
        if (major, minor) == line && patch < floor {
            return Err(format!(
                "{krate} {major}.{minor}.{patch} is inside the range affected by {advisory} (fixed in \
                 {}.{}.{floor}); update the lockfile — `cargo update -p {krate} --precise {}.{}.{floor}`",
                line.0, line.1, line.0, line.1
            ));
        }
    }
    Ok(())
}

fn lockfile() -> toml::Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.lock");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    // `str::parse::<toml::Value>` is not the document entry point in toml 1.x — it rejects the whole file with
    // "unexpected content, expected nothing". `toml::from_str` is.
    toml::from_str::<toml::Value>(&text)
        .unwrap_or_else(|e| panic!("cannot parse {} as TOML: {e}", path.display()))
}

#[test]
fn quinn_udp_is_outside_the_affected_advisory_range() {
    let lock = lockfile();
    let advisory = "RUSTSEC-2026-0538 / GHSA-6pp4-4cxf-xf88";
    policy_check(&lock, "quinn-udp", (0, 5), 16, advisory).unwrap();
    policy_check(&lock, "quinn-udp", (0, 6), 3, advisory).unwrap();
    // Version-free marker, printed only once both checks have passed, so it is absent on the unpatched tree.
    // CI requires it on GREEN/HEAD, which is how it tells "the pinned assertion passed" from "some test
    // matching the filter passed".
    println!("policy: quinn-udp clears {advisory}");
}

#[test]
fn quinn_proto_is_outside_the_affected_advisory_range() {
    let lock = lockfile();
    let advisory = "RUSTSEC-2026-0524";
    policy_check(&lock, "quinn-proto", (0, 11), 19, advisory).unwrap();
    println!("policy: quinn-proto clears {advisory}");
}

/// The pin must hold for **every** resolved generation. A lock carrying `quinn-udp 0.5.16` *and* an affected
/// `0.6.2` is precisely the shape a first-match reader waves through, so it is pinned here as a negative control
/// over a synthetic lock rather than left to be discovered in a future resolution.
#[test]
fn a_second_resolved_generation_cannot_slip_through() {
    let synthetic = r#"
[[package]]
name = "quinn-udp"
version = "0.5.16"

[[package]]
name = "quinn-udp"
version = "0.6.2"
"#;
    let lock: toml::Value = toml::from_str(synthetic).expect("the synthetic lock parses as TOML");

    let verdict = policy_check(&lock, "quinn-udp", (0, 6), 3, "RUSTSEC-2026-0538");
    assert!(
        verdict.is_err(),
        "an affected second generation must be refused, not read as the fixed first one"
    );
    assert!(
        verdict.unwrap_err().contains("0.6.2"),
        "the refusal must name the offending version"
    );

    // And the same lock must still satisfy the 0.5 line: the check is per-generation, not first-match.
    assert!(
        policy_check(&lock, "quinn-udp", (0, 5), 16, "RUSTSEC-2026-0538").is_ok(),
        "the fixed 0.5 generation must still pass"
    );
}
