//! Standalone execution of the two `check_and_increment` shapes against the
//! LOCKFILE's moka (0.12.15), for the 64-thread barrier scenario asserted by
//! `concurrent_burst_admits_exactly_one_request` (patch 0001).
//!
//! No oxicloud build: only the primitive, with both bodies copied verbatim —
//! `Patched` from `src/interfaces/middleware/rate_limit.rs` (post-0001) and
//! `Original` from v0.10.0's file (pre-0001). Window 15 s to match the corpus
//! posture (`c0-burst.tsv`: max=1/15 s).
//!
//! Two modes (`argv[2]`), both run at opt-level 0 / 1 / 3:
//! * `burst` — the test's own scenario: 64 threads released from one barrier,
//!   10 trials per configuration. A stochastic detector; its overshoot rate
//!   depends on opt-level and on ambient load.
//! * `deterministic` — the legal interleaving of the pre-0001 read/write halves,
//!   executed literally with no threads: proves the shape is non-atomic without
//!   depending on scheduling, timing or optimisation level.
use moka::sync::Cache;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
struct Window {
    count: u32,
    started: Instant,
}

// ── post-0001: one key-locked upsert ────────────────────────────────────────
struct Patched {
    cache: Cache<String, Window>,
    max_requests: u32,
    window_secs: u64,
}

impl Patched {
    fn new(max_requests: u32, window_secs: u64, max_entries: u64) -> Self {
        let cache = Cache::builder()
            .time_to_live(Duration::from_secs(window_secs))
            .max_capacity(max_entries)
            .build();
        Self {
            cache,
            max_requests,
            window_secs,
        }
    }

    fn check_and_increment(&self, key: &str) -> Result<u32, ()> {
        let window = Duration::from_secs(self.window_secs);
        let entry = self
            .cache
            .entry_by_ref(key)
            .and_upsert_with(|current| match current {
                Some(entry) => {
                    let mut w = entry.into_value();
                    if w.started.elapsed() < window {
                        w.count = w.count.saturating_add(1);
                        w
                    } else {
                        Window {
                            count: 1,
                            started: Instant::now(),
                        }
                    }
                }
                None => Window {
                    count: 1,
                    started: Instant::now(),
                },
            });
        let count = entry.into_value().count;
        if count > self.max_requests {
            Err(())
        } else {
            Ok(count)
        }
    }
}

// ── pre-0001: read-then-write, nothing serialising it ───────────────────────
struct Original {
    cache: Cache<String, u32>,
    max_requests: u32,
    window_secs: u64,
}

impl Original {
    fn new(max_requests: u32, window_secs: u64, max_entries: u64) -> Self {
        let cache = Cache::builder()
            .time_to_live(Duration::from_secs(window_secs))
            .max_capacity(max_entries)
            .build();
        Self {
            cache,
            max_requests,
            window_secs,
        }
    }

    fn check_and_increment(&self, ip: &str) -> Result<u32, ()> {
        let count = self.cache.get(ip).unwrap_or(0) + 1;
        self.cache.insert(ip.to_string(), count);
        if count > self.max_requests {
            Err(())
        } else {
            Ok(count)
        }
    }

    /// The same function, with its read and its write exposed as separate steps
    /// so a caller can interleave them the way two threads may. Production code
    /// hides this split inside one call, which is exactly why the shape is not
    /// atomic — this makes the window literal instead of probabilistic.
    fn read(&self, ip: &str) -> u32 {
        self.cache.get(ip).unwrap_or(0) + 1
    }

    fn write_and_decide(&self, ip: &str, count: u32) -> Result<u32, ()> {
        self.cache.insert(ip.to_string(), count);
        if count > self.max_requests {
            Err(())
        } else {
            Ok(count)
        }
    }
}

fn burst<F>(f: F, n: usize) -> usize
where
    F: Fn(&str) -> Result<u32, ()> + Send + Sync + 'static,
{
    let barrier = Arc::new(Barrier::new(n));
    let f = Arc::new(f);
    let handles: Vec<_> = (0..n)
        .map(|_| {
            let barrier = barrier.clone();
            let f = f.clone();
            thread::spawn(move || {
                barrier.wait();
                f("burst").is_ok()
            })
        })
        .collect();
    handles
        .into_iter()
        .map(|h| h.join().expect("worker thread panicked"))
        .filter(|ok| *ok)
        .count()
}

fn run(shape: &str, budget: u32, trials: u32, n: usize) -> Vec<usize> {
    let mut served = Vec::new();
    for _ in 0..trials {
        let allowed = if shape == "patched" {
            let limiter = Patched::new(budget, 15, 1_000);
            burst(move |k| limiter.check_and_increment(k), n)
        } else {
            let limiter = Original::new(budget, 15, 1_000);
            burst(move |k| limiter.check_and_increment(k), n)
        };
        served.push(allowed);
    }
    served
}

fn deterministic() {
    let key = "burst";
    let budget = 1u32;

    // Two requests on one key, in the order the pre-0001 shape permits: B's read
    // happens before A's write. No threads, no timing, no optimisation
    // sensitivity — and both are served, on a budget of one.
    let original = Original::new(budget, 15, 1_000);
    let a = original.read(key);
    let b = original.read(key); // A has not written yet
    let a_ok = original.write_and_decide(key, a).is_ok();
    let b_ok = original.write_and_decide(key, b).is_ok();
    println!(
        "original  interleaved reads (A={a}, B={b}) -> served={} of budget {budget}: A_ok={a_ok} B_ok={b_ok}",
        u8::from(a_ok) + u8::from(b_ok)
    );

    // The upsert shape cannot be split this way: each request is one call, so the
    // second call sees the first one's write and the budget holds.
    let patched = Patched::new(budget, 15, 1_000);
    let first = patched.check_and_increment(key);
    let second = patched.check_and_increment(key);
    println!("patched   same two requests, one call each -> {first:?} then {second:?} (budget {budget})");
}

fn main() {
    // argv[1] labels the profile this binary was built under; argv[2] selects the
    // mode. The authoritative statement of the level is the run command in the
    // log (cargo's own OPT_LEVEL is not exposed to the crate).
    let label = std::env::args().nth(1).unwrap_or_else(|| "unlabelled".to_string());
    let mode = std::env::args().nth(2).unwrap_or_else(|| "burst".to_string());
    println!("label={label} mode={mode} opt_level_from_cargo={:?}", option_env!("OPT_LEVEL"));
    println!("moka =0.12.15, features [future, sync] (the repo's dependency spec)");

    if mode == "deterministic" {
        deterministic();
        return;
    }

    let n = 64usize;
    let trials = 10u32;
    println!("n={n} threads/barrier window=15s trials={trials}");
    for (shape, budget) in [
        ("patched", 1u32),
        ("patched", 8),
        ("original", 1),
        ("original", 8),
    ] {
        let served = run(shape, budget, trials, n);
        let mut sorted = served.clone();
        sorted.sort_unstable();
        let overshoots = served.iter().filter(|s| **s > budget as usize).count();
        println!(
            "{shape:9} budget={budget:2}  served per trial: {served:?}  min={} max={} expected={budget} overshoot_trials={overshoots}/{trials}",
            sorted[0],
            sorted[sorted.len() - 1],
        );
    }
}
