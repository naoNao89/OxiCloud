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

    /// One `Arc<RateLimiter>` is one budget, whichever route the request takes —
    /// and a handler-level 400 spends it exactly like a success.
    ///
    /// Both halves are `main.rs` wiring facts: the auth routers share an instance
    /// wherever the policy is one budget (`login` also covers the OPAQUE login
    /// route), and `/api/setup` is handed the very same `register_limiter` the
    /// register route uses — so exhausting the budget on one locks the other out,
    /// which is what the assertions below pin (`setup_route()` +
    /// `from_fn_with_state(register_limiter.clone(), rate_limit_register)`).
    ///
    /// Not a RED test: both behaviours hold on v0.10.0 and after
    /// `0001-limiter-window.patch` alike. It guards the accounting through the
    /// window rewrite (a per-key window must not become a per-route one) and the
    /// 429 body + header shape.
    #[tokio::test]
    async fn two_routes_share_one_budget() {
        let limiter = Arc::new(RateLimiter::new(2, 60, 100));

        // Two routers, one limiter instance — the shape main.rs builds for
        // register + setup.
        let register_routes = Router::new().route("/auth/register", get(ok)).layer(
            axum::middleware::from_fn_with_state(limiter.clone(), rate_limit_register),
        );
        let setup_routes =
            Router::new()
                .route("/setup", get(ok))
                .layer(axum::middleware::from_fn_with_state(
                    limiter.clone(),
                    rate_limit_register,
                ));
        let app = register_routes.merge(setup_routes);

        let req = |uri: &str| Request::builder().uri(uri).body(Body::empty()).unwrap();

        let first = app.clone().oneshot(req("/auth/register")).await.unwrap();
        assert_eq!(first.status(), StatusCode::OK);

        // 2nd request overall, on the OTHER route: still inside the shared
        // budget, which only holds if both routes share one cache.
        let second = app.clone().oneshot(req("/setup")).await.unwrap();
        assert_eq!(second.status(), StatusCode::OK);

        // 3rd request overall: rejected, with the Retry-After header.
        let third = app.clone().oneshot(req("/auth/register")).await.unwrap();
        assert_eq!(third.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(third.headers().get("retry-after").unwrap(), "60");

        // The pin: exhausting the budget on /auth/register has already locked
        // out /setup.
        let fourth = app.clone().oneshot(req("/setup")).await.unwrap();
        assert_eq!(fourth.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(fourth.headers().get("retry-after").unwrap(), "60");

        let body = axum::body::to_bytes(fourth.into_body(), 4096)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "Too many requests");
        assert_eq!(json["retry_after_secs"], 60);

        // A handler-level rejection burns budget too: the limiter runs before
        // `next.run`, so a typo in the setup wizard — 400 from validation,
        // never reaching the database — spends the SAME register/setup budget a
        // real registration would. With `max_requests = 2`, two 400s are
        // served and the third request (on the other route) is already 429.
        let strict = Arc::new(RateLimiter::new(2, 60, 100));
        let rejecting_register = Router::new()
            .route("/auth/register", post(bad_request))
            .layer(axum::middleware::from_fn_with_state(
                strict.clone(),
                rate_limit_register,
            ));
        let rejecting_setup =
            Router::new()
                .route("/setup", get(ok))
                .layer(axum::middleware::from_fn_with_state(
                    strict.clone(),
                    rate_limit_register,
                ));
        let rejecting = rejecting_register.merge(rejecting_setup);

        let post_req = |uri: &str| {
            Request::builder()
                .method("POST")
                .uri(uri)
                .body(Body::empty())
                .unwrap()
        };
        for attempt in 1..=2 {
            let rejected = rejecting
                .clone()
                .oneshot(post_req("/auth/register"))
                .await
                .unwrap();
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
        let exhausted = rejecting.clone().oneshot(req("/setup")).await.unwrap();
        assert_eq!(
            exhausted.status(),
            StatusCode::TOO_MANY_REQUESTS,
            "the two 400s spent the shared budget, so /setup is locked out"
        );
    }
