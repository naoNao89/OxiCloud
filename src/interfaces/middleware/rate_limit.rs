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

#[cfg(test)] #[rustfmt::skip] mod tests;
