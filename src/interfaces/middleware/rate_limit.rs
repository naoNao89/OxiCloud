//! IP-based rate limiting middleware for authentication endpoints.
//!
//! Uses `moka` caches (already a project dependency) to track request
//! counts per client IP.  Each protected endpoint group gets its own
//! [`RateLimiter`] instance with independently tuneable limits.
//!
//! Client IP resolution is delegated to [`super::trusted_proxy::client_ip`],
//! which honours `OXICLOUD_TRUST_PROXY_CIDR` for proxy-header forwarding.
//!
//! # Window semantics
//!
//! One convention, and it is a **fixed window**: the first request from a key
//! opens a window of `window_secs` and stores its start instant; every request
//! inside that window increments the count (rejected ones included — a request
//! that reaches the limiter spends budget whatever the handler answers); the
//! first request after the window closes opens a fresh window. The boundary
//! never moves on a rejection, so a client that keeps hammering is let back in
//! one window after its **first** request in the window, never one window after
//! its last attempt.
//!
//! When the limit is exceeded a `429 Too Many Requests` response is returned
//! with a `Retry-After` header carrying the time **left** in that key's current
//! window (rounded up, at least 1 s) — never the configured window, which
//! overstated the wait by however much of the window was already spent.

use axum::{
    http::{HeaderValue, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use moka::sync::Cache;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// One key's live window: how many requests it has seen, and when the window
/// opened.
///
/// The two move together — a request either joins the open window (count + 1,
/// start unchanged) or opens a new one (count 1, start now) — which is why they
/// are one value: moka hands the upsert closure the cached value as a whole, so
/// the read-decide-write step below is atomic for a key. Split across two cells
/// it would need two writes and could tear.
#[derive(Clone, Copy, Debug)]
struct Window {
    /// Requests seen in this window, rejected ones included.
    count: u32,
    /// When this window opened. `Instant`, not `SystemTime`: monotonic, so a
    /// wall-clock step (NTP, DST, `date -s`) cannot shorten or extend a window.
    started: Instant,
}

/// A fixed-window request counter keyed by IP address.
///
/// Each request increments its key's window; once the count exceeds
/// `max_requests` the request is rejected. A key's window ends `window_secs`
/// after the *first* request in it — see the module docs for the semantics and
/// why a rejection must not extend it.
#[derive(Clone)]
pub struct RateLimiter {
    /// Maps `key -> Window`, with a TTL used only to drop idle keys.
    cache: Cache<String, Window>,
    /// Maximum requests allowed within the window.
    max_requests: u32,
    /// Window duration in seconds.
    window_secs: u64,
}

impl RateLimiter {
    /// Create a new rate limiter.
    ///
    /// * `max_requests`, ceiling per key within the window
    /// * `window_secs` , fixed window duration (each window opens on its key's
    ///   first request)
    /// * `max_entries` , upper bound on tracked keys (evicts LRU when exceeded)
    pub fn new(max_requests: u32, window_secs: u64, max_entries: u64) -> Self {
        // The TTL is a garbage-collection hint, NOT the window boundary —
        // `Window::started` is. Every write refreshes it, and a write happens
        // only inside a live window, so an entry always outlives its own window
        // and an eviction can never cut a window short (the boundary reset is
        // reachable by construction). What is left is housekeeping: a key that
        // goes quiet is dropped one window after its last request, which is when
        // it would have reset anyway.
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

    /// Check whether the key is allowed. Returns `Ok(current_count)` or
    /// `Err(StatusCode::TOO_MANY_REQUESTS)`.
    #[allow(clippy::result_unit_err)]
    pub fn check_and_increment(&self, key: &str) -> Result<u32, ()> {
        let window = Duration::from_secs(self.window_secs);

        // One atomic upsert per key. `and_upsert_with` runs the closure under
        // moka's key-level lock and hands it the *current* value, so concurrent
        // calls for the SAME key are serialised (moka: "concurrent calls on the
        // same key are executed serially") and each one sees its predecessor's
        // write: N concurrent requests on one key hand out counts `1..=N`, and
        // exactly `min(N, max_requests)` of them come back `Ok`. Calls for
        // DIFFERENT keys take different locks and run in parallel — one client's
        // burst cannot eat another's budget, and the counters stay independent.
        //
        // The previous shape (`get().unwrap_or(0) + 1` then `insert`) was a
        // read-then-write pair with nothing serialising it: every thread that
        // read between a peer's `get` and its `insert` saw the same stale count,
        // so a burst against a budget of one could serve several requests. It
        // also re-inserted on rejection, which reset the TTL and slid the window
        // forward on the attacker's own retries.
        //
        // Cost: the upsert clones the key (`entry_by_ref` + `to_owned`) on every
        // call, exactly as the old `insert` did; the read path no longer borrows
        // the key without writing, because there is no longer a read path — the
        // increment and the accounting are one step.
        let entry = self
            .cache
            .entry_by_ref(key)
            .and_upsert_with(|current| match current {
                Some(entry) => {
                    let mut w = entry.into_value();
                    if w.started.elapsed() < window {
                        // Same window: the count grows, the boundary does not
                        // move — a rejected request must not extend the lockout.
                        w.count = w.count.saturating_add(1);
                        w
                    } else {
                        // The previous window has closed: open a new one now.
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

    /// Upper bound, in seconds, on how long any key can be told to wait: the
    /// configured window.
    ///
    /// This is the number the pre-`0001` code put in `Retry-After` for every
    /// key. It is still the correct *ceiling* — a window opens on its key's
    /// first request, so nothing a key can be asked to wait for exceeds it — and
    /// it stays for the callers that have no key at hand (and because
    /// `retry_after_for` is bounded by it). The value a client is actually told
    /// comes from [`Self::retry_after_for`].
    pub fn retry_after(&self) -> u64 {
        self.window_secs
    }

    /// Seconds the client should wait before a request from `key` may be
    /// served: the time left in that key's current window, rounded up to whole
    /// seconds and floored at 1.
    ///
    /// Rounded up on purpose: `Retry-After` is a lower bound the client is
    /// entitled to trust (RFC 9110 §10.2.3), so reporting 59 when 59.4 s remain
    /// would have a well-behaved client retry a tick early and collect another
    /// 429. The floor of 1 keeps `Retry-After: 0` (retry now, still rejected)
    /// off the wire — including for a key with no live window, where nothing is
    /// being held back and one second is the honest estimate. Never exceeds
    /// [`Self::retry_after`].
    pub fn retry_after_for(&self, key: &str) -> u64 {
        let window = Duration::from_secs(self.window_secs);
        let Some(entry) = self.cache.get(key) else {
            return 1;
        };
        let elapsed = entry.started.elapsed();
        if elapsed >= window {
            return 1;
        }
        let remaining = window - elapsed;
        // Whole seconds, rounded UP, never 0 here (remaining > 0): hand-rolled
        // because `Duration::div_ceil` is not stable std (E0599 on rustc 1.99.0).
        remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0)
    }
}

// ─── Axum middleware factories ──────────────────────────────────────────────

/// Extract the most-likely real client IP from headers / connection info.
///
/// Proxy headers (`X-Forwarded-For`, `X-Real-Ip`) are only trusted when the
/// TCP peer address falls within `OXICLOUD_TRUST_PROXY_CIDR`.  Without a
/// configured CIDR list an attacker could spoof headers to bypass rate limiting.
pub fn extract_client_ip<B>(req: &Request<B>) -> String {
    super::trusted_proxy::client_ip(req, false)
}

/// Build a rate-limit response with the standard `Retry-After` header.
///
/// Public so handlers that do their own (non-middleware) rate checks —
/// e.g. the email-invite branch of `POST /api/grants`, where the limit
/// only applies to one subject variant — can return the same shape.
pub fn too_many_requests(retry_after: u64) -> Response {
    let body = serde_json::json!({
        "error": "Too many requests",
        "retry_after_secs": retry_after,
    });
    let mut resp = (StatusCode::TOO_MANY_REQUESTS, axum::Json(body)).into_response();
    if let Ok(val) = HeaderValue::from_str(&retry_after.to_string()) {
        resp.headers_mut().insert("retry-after", val);
    }
    resp
}

/// Shared body of every per-endpoint limiter below.
///
/// The public wrappers differ only in the `endpoint` they name in the warning,
/// so the decision lives here once: each endpoint gets its own `RateLimiter`
/// (its own budget), but they all enforce it identically.
async fn enforce(
    limiter: &RateLimiter,
    endpoint: &'static str,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let ip = extract_client_ip(&req);
    match limiter.check_and_increment(&ip) {
        Ok(_) => next.run(req).await,
        Err(()) => {
            tracing::warn!(ip = %ip, endpoint, "Rate limit exceeded");
            // Per key, not the configured window: `Retry-After` tells THIS
            // client how much of ITS window is left.
            too_many_requests(limiter.retry_after_for(&ip))
        }
    }
}

/// Axum middleware: rate-limit login attempts.
///
/// Inject via:
/// ```ignore
/// .layer(axum::middleware::from_fn_with_state(limiter, rate_limit_login))
/// ```
pub async fn rate_limit_login(
    State(limiter): axum::extract::State<Arc<RateLimiter>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    enforce(&limiter, "login", req, next).await
}

/// Axum middleware: rate-limit registration attempts.
pub async fn rate_limit_register(
    State(limiter): axum::extract::State<Arc<RateLimiter>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    enforce(&limiter, "register", req, next).await
}

/// Axum middleware: rate-limit token refresh attempts.
pub async fn rate_limit_refresh(
    State(limiter): axum::extract::State<Arc<RateLimiter>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    enforce(&limiter, "refresh", req, next).await
}

/// Axum middleware: rate-limit share-password verification.
///
/// `POST /api/s/{token}/verify` is unauthenticated and runs Argon2id at the
/// configured memory cost per call, so it is both a password oracle and a
/// memory amplifier — the same two properties that earned `/api/auth/login` a
/// limiter. It is budgeted from `login_max_requests`/`login_window_secs` for
/// exactly that reason: a share password is a password, and an operator who
/// tightens the login budget means to tighten password guessing everywhere.
pub async fn rate_limit_share_verify(
    State(limiter): axum::extract::State<Arc<RateLimiter>>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    enforce(&limiter, "share_verify", req, next).await
}

use axum::extract::State;

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::{
        Router,
        routing::{get, post},
    };
    use std::thread;
    use tower::ServiceExt;

    /// Always-succeeds route: the limiter layer is the only thing that can turn
    /// a request into a 429.
    async fn ok() -> StatusCode {
        StatusCode::OK
    }

    /// Handler-level rejection (like a validation failure): the limiter must
    /// count it just the same.
    async fn bad_request() -> StatusCode {
        StatusCode::BAD_REQUEST
    }

    /// The seconds a 429 is told to wait, read off the response.
    fn retry_after_header(resp: &Response) -> u64 {
        resp.headers()
            .get("retry-after")
            .expect("a 429 carries Retry-After")
            .to_str()
            .expect("Retry-After is ASCII digits")
            .parse()
            .expect("Retry-After is a whole number of seconds")
    }

    /// A rejected request must NOT move the window forward.
    ///
    /// Regression this catches: the pre-`0001` shape wrote the counter back on
    /// every request, rejected ones included, and moka resets a key's TTL on
    /// every write — so the window followed the client's *last* attempt instead
    /// of its first, and a client that kept hammering a limited endpoint could
    /// hold its own lockout open indefinitely (t0 + window never arrived,
    /// because each attempt at t0 + ε moved the boundary to t0 + ε + window).
    ///
    /// Fixed window, 4 s, with one rejected attempt at t≈2 s to give the old
    /// shape something to slide on:
    /// * t≈0 s — request 1 allowed, request 2 rejected; the window opened at
    ///   t≈0 and closes at t≈4 s.
    /// * t≈2 s — request 3 rejected: still inside that window, which is the
    ///   proof it did not expire early. Under the old shape this write moved the
    ///   expiry to t≈6 s.
    /// * t≈5 s — request 4 must be ALLOWED: one window after the FIRST request,
    ///   not one window after the last rejected one. `thread::sleep` never
    ///   returns early, so "≥ 4 s elapsed" holds by construction; the 1 s of
    ///   slack under the old shape's t≈6 s expiry is what makes this
    ///   discriminate fixed from sliding.
    ///
    /// RED on v0.10.0 (request 4 is still rejected there), GREEN after
    /// `0001-limiter-window.patch`.
    #[test]
    fn rejected_requests_do_not_extend_the_window() {
        let limiter = RateLimiter::new(1, 4, 100);

        assert_eq!(limiter.check_and_increment("attacker"), Ok(1));
        assert_eq!(limiter.check_and_increment("attacker"), Err(()));

        thread::sleep(Duration::from_millis(2000));
        assert_eq!(
            limiter.check_and_increment("attacker"),
            Err(()),
            "t≈2 s is inside the window opened at t≈0"
        );

        thread::sleep(Duration::from_millis(3000));
        assert_eq!(
            limiter.check_and_increment("attacker"),
            Ok(1),
            "t≈5 s is one 4 s window past the FIRST request, so the client is \
             let back in; the pre-patch code kept it out until one window after \
             the rejected attempt at t≈2 s (t≈6 s)"
        );
    }

    /// `Retry-After` carries the time LEFT in the window, not the configured
    /// window: two rejections 3 s apart in a 6 s window must not both say 6.
    ///
    /// Regression this catches: the pre-`0001` `retry_after()` returned
    /// `window_secs` as a constant, so a client that had already spent seconds
    /// hammering was told to wait the whole window again — every 429 overstated
    /// the wait by however much of the window it had already burned, and an
    /// obedient client retried later than it had to.
    ///
    /// Measured through the real middleware because `Retry-After` on the 429 is
    /// the wire contract the SPA and third-party clients read, and because the
    /// pre-existing symbols (`RateLimiter::new`, `check_and_increment`,
    /// zero-arg `retry_after()`, the `rate_limit_*` layers) are all this test
    /// touches — it must compile unchanged against v0.10.0 for the RED run.
    ///
    /// Timing (6 s window, 3 s sleep): the second value is `ceil(≈3 s)`, so it
    /// lands in 1..=3 even if the sleep overshoots by 2 s. The assertion that
    /// discriminates is `second < first` — on v0.10.0 both are the constant 6.
    ///
    /// RED on v0.10.0, GREEN after `0001-limiter-window.patch`.
    #[tokio::test]
    async fn retry_after_header_shrinks_as_the_window_elapses() {
        let limiter = Arc::new(RateLimiter::new(1, 6, 100));
        let app =
            Router::new()
                .route("/limited", get(ok))
                .layer(axum::middleware::from_fn_with_state(
                    limiter.clone(),
                    rate_limit_login,
                ));
        let req = || {
            Request::builder()
                .uri("/limited")
                .body(Body::empty())
                .unwrap()
        };

        assert_eq!(
            app.clone().oneshot(req()).await.unwrap().status(),
            StatusCode::OK
        );

        let first = app.clone().oneshot(req()).await.unwrap();
        assert_eq!(first.status(), StatusCode::TOO_MANY_REQUESTS);
        let first_after = retry_after_header(&first);
        assert_eq!(
            first_after, 6,
            "t≈0: the whole 6 s window is still ahead (ceil of just under 6 s)"
        );
        assert_eq!(
            limiter.retry_after(),
            6,
            "the un-keyed value stays the configured window — the ceiling the \
             per-key header below can never exceed"
        );

        thread::sleep(Duration::from_millis(3000));

        let second = app.clone().oneshot(req()).await.unwrap();
        assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
        let second_after = retry_after_header(&second);
        assert!(
            second_after < first_after,
            "≈3 s of the 6 s window are gone, so the wait must shrink: \
             {second_after} s vs {first_after} s (equal values mean the header \
             is reporting the configured window again)"
        );
        assert!(
            (1..=3).contains(&second_after),
            "≈3 s remain, so ceil(remaining) is 3 (this bound tolerates 2 s of \
             scheduler overshoot on the sleep); got {second_after}"
        );

        let body = axum::body::to_bytes(second.into_body(), 4096)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "Too many requests");
        assert_eq!(
            json["retry_after_secs"], second_after,
            "the body field and the header are the same value in two places"
        );
    }

    /// GUARD — an invariant, NOT a RED/GREEN pair: 64 threads released from a
    /// barrier against a budget of 1 must serve exactly one request.
    ///
    /// What it protects: `check_and_increment` must stay one atomic
    /// read-decide-write per key. The pre-`0001` shape
    /// (`get().unwrap_or(0) + 1` then `insert`) had nothing serialising the
    /// pair, so a future refactor could reintroduce it — this test fails if it
    /// does, which is the whole point of keeping it.
    ///
    /// What it is NOT: proof that a defect was fixed. The interleavings the real
    /// server actually produces are identical on both builds — the build side
    /// drove a 64-request burst at register 1/15 s, 5 trials per build, and got
    /// exactly 1 served / 63 rejected every time
    /// (`docs/re/evidence/c0-burst.tsv` unpatched, `c1-burst.tsv` patched; the
    /// corpus cases are `concurrent-burst-rejects-8` / `-16` in
    /// `docs/re/evidence/A-cases.md`). That is why this test is not counted in a
    /// RED column here.
    ///
    /// What it does show, measured on a standalone instrument (both bodies
    /// verbatim, the lockfile's moka, 10 trials per configuration,
    /// `docs/re/patches/atomicity-guard-standalone-run.txt`): the pre-patch shape
    /// CAN serve more than its budget, but only sometimes, and only under a wide
    /// enough arrival spread. Two independent runs at each opt-level disagreed
    /// with each other — overshoot trials at budget 1 were 6/10, 3/10, 0/10 in one
    /// run and 1/10, 3/10, 6/10 in the other, with load differing — so this test's
    /// pass/fail is not reproducible from the code alone.
    ///
    /// The non-atomicity itself is not stochastic, though: interleaving the two
    /// halves of the old read/write pair by hand serves 2 on a budget of 1 at
    /// every opt-level (same instrument, `deterministic` mode), which is the
    /// property this patch fixes and which no arrival spread can change. So this
    /// test is a FLAKY detector of a real defect — which is precisely why it is
    /// classified as a guard here and kept out of the gate.
    #[test]
    fn concurrent_burst_admits_exactly_one_request() {
        const THREADS: usize = 64;

        // Corpus posture: budget 1 per 15 s, one key.
        let limiter = Arc::new(RateLimiter::new(1, 15, 128));
        let barrier = Arc::new(std::sync::Barrier::new(THREADS));

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let limiter = limiter.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    barrier.wait();
                    limiter.check_and_increment("burst").is_ok()
                })
            })
            .collect();

        let allowed = handles
            .into_iter()
            .map(|h| h.join().expect("worker thread panicked"))
            .filter(|ok| *ok)
            .count();
        assert_eq!(
            allowed, 1,
            "exactly one of {THREADS} concurrent requests fits a budget of 1"
        );

        // The counted key is spent, and a different key is untouched — the
        // serialisation is per key, not a global lock that ate every budget.
        // Both assertions also hold on v0.10.0: with no mutual exclusion the
        // pre-fix shape merely *may* have counted fewer, which is why the coarse
        // server-level interleavings measured on both builds (c0/c1-burst.tsv)
        // cannot tell the two implementations apart — see the doc comment above
        // for the unit-level harness run that can.
        assert_eq!(limiter.check_and_increment("burst"), Err(()));
        assert_eq!(limiter.check_and_increment("other"), Ok(1));
    }

    /// `/api/setup` and `/api/auth/register` have SEPARATE budgets: `main.rs`
    /// builds a dedicated `RateLimiter` for setup out of
    /// `OXICLOUD_RATE_LIMIT_SETUP_*`, so exhausting either route leaves the
    /// other's slot intact — asserted in both directions.
    ///
    /// This replaces `two_routes_share_one_budget`, which pinned the wiring this
    /// patch changes: `/api/setup` used to be handed the very same
    /// `register_limiter`, so one 5-per-hour budget covered both endpoints — a
    /// burst of registrations (or a bot probing `/api/auth/register`) locked the
    /// operator out of the first-run wizard, and a mistyped setup attempt spent
    /// registration budget.
    ///
    /// Not a RED test: it builds the two instances it asserts about, which is
    /// possible on v0.10.0 as well, so this is a guard rather than evidence of
    /// the fix (the evidence is the two-arm run of `/api/setup` vs
    /// `/api/auth/register` against a real server). What it guards: the
    /// accounting stays per INSTANCE (a process-wide cache or a "share the
    /// limiter to save memory" refactor fails here), and a handler-level 400
    /// still spends the budget of the limiter in front of it.
    ///
    /// Symbols: pre-existing ones only (`RateLimiter::new`, the `rate_limit_*`
    /// layers, the `retry-after` header) plus this module's own
    /// `retry_after_header`, so the module still compiles on v0.10.0.
    #[tokio::test]
    async fn setup_and_register_have_separate_budgets() {
        // Mirrors the wiring in `main.rs`: one limiter instance per route
        // group. Budget 2 keeps exhaustion cheap; the register route answers
        // 400 (a validation rejection must still count).
        fn app(register_limiter: Arc<RateLimiter>, setup_limiter: Arc<RateLimiter>) -> Router {
            Router::new()
                .route("/auth/register", post(bad_request))
                .layer(axum::middleware::from_fn_with_state(
                    register_limiter,
                    rate_limit_register,
                ))
                .merge(Router::new().route("/setup", get(ok)).layer(
                    axum::middleware::from_fn_with_state(setup_limiter, rate_limit_register),
                ))
        }

        let register_req = || {
            Request::builder()
                .method("POST")
                .uri("/auth/register")
                .body(Body::empty())
                .unwrap()
        };
        let setup_req = || {
            Request::builder()
                .uri("/setup")
                .body(Body::empty())
                .unwrap()
        };

        // ── Direction 1: spend register's budget; setup keeps its own slot. ──
        let d1 = app(
            Arc::new(RateLimiter::new(2, 60, 100)),
            Arc::new(RateLimiter::new(2, 60, 100)),
        );
        for attempt in 1..=2 {
            let rejected = d1.clone().oneshot(register_req()).await.unwrap();
            assert_eq!(
                rejected.status(),
                StatusCode::BAD_REQUEST,
                "validation rejection #{attempt} is still counted"
            );
            assert!(
                rejected.headers().get("retry-after").is_none(),
                "a 400 is not a 429 — no Retry-After"
            );
        }
        let spent = d1.clone().oneshot(register_req()).await.unwrap();
        assert_eq!(
            spent.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "two 400s spent register's budget"
        );
        let spent_after = retry_after_header(&spent);
        assert!(
            (1..=60).contains(&spent_after),
            "the 429 still carries a remaining-window Retry-After; got {spent_after}"
        );

        for n in 1..=2 {
            let own_slot = d1.clone().oneshot(setup_req()).await.unwrap();
            assert_eq!(
                own_slot.status(),
                StatusCode::OK,
                "/setup request {n}: its budget is untouched by register's"
            );
        }
        let setup_spent = d1.clone().oneshot(setup_req()).await.unwrap();
        assert_eq!(
            setup_spent.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "setup's own budget is spent after its own two requests"
        );

        // ── Direction 2, fresh instances: spend setup's first; register's slot
        //    is intact. ─────────────────────────────────────────────────────────
        let d2 = app(
            Arc::new(RateLimiter::new(2, 60, 100)),
            Arc::new(RateLimiter::new(2, 60, 100)),
        );
        for n in 1..=2 {
            assert_eq!(
                d2.clone().oneshot(setup_req()).await.unwrap().status(),
                StatusCode::OK,
                "setup request {n}"
            );
        }
        assert_eq!(
            d2.clone().oneshot(setup_req()).await.unwrap().status(),
            StatusCode::TOO_MANY_REQUESTS,
            "setup's own budget is spent"
        );
        for n in 1..=2 {
            let register_ok = d2.clone().oneshot(register_req()).await.unwrap();
            assert_eq!(
                register_ok.status(),
                StatusCode::BAD_REQUEST,
                "register request {n}: its budget is untouched by setup's"
            );
            assert!(register_ok.headers().get("retry-after").is_none());
        }
    }
}
