//! Live-HTTP regression: `/api/setup`'s rate-limit budget is **its own**.
//!
//! The unit tests around this change cannot prove it. `rate_limit::tests::
//! setup_and_register_have_separate_budgets` builds the two limiter instances it
//! asserts about, so it passes on the tree where `main.rs` still hands the setup
//! route `register_limiter`; and a config-surface test only shows the knob is
//! missing. The property under test is a fact about the **wiring inside the real
//! binary**, so this test boots the real binary and watches the two routes'
//! budgets interact.
//!
//! The policy: `/api/setup` is the one-time first-run wizard and
//! `/api/auth/register` is open registration. Independent quotas means traffic
//! on one must not consume the other's — otherwise a burst of registrations (or
//! a bot probing `register`) locks the operator out of first-run setup, and a
//! mistyped setup attempt spends registration budget.
//!
//! What it does, against a fresh database and a free port:
//!   1. boots `env!("CARGO_BIN_EXE_oxicloud")` with `register` and `setup` both
//!      at `MAX = 2` attempts per hour (the setup pair is ignored by a build
//!      that does not know those variables, which is the defect);
//!   2. spends the setup route's budget on requests the handler rejects (a
//!      7-character password is a 400 — the limiter runs first, so a rejection
//!      spends budget exactly like a success would);
//!   3. calls `/api/auth/register` once and requires that it was **not** refused
//!      (`429`) by a budget the setup attempts consumed.
//!
//! Pre-fix (setup sharing `register_limiter`) step 3 answers 429 and this test
//! fails, naming the statuses it saw. Post-fix it answers 400 and the test
//! passes. The assertion in step 2 keeps the test from passing vacuously: a
//! server with no limiter at all would have nothing to consume.
//!
//! Hermeticity. Nothing here is hard-coded to one machine: the only input is
//! `OXICLOUD_TEST_DB_URL` (falling back to `OXICLOUD_DB_CONNECTION_STRING`), a
//! Postgres URL whose credentials/host/port are reused for a throwaway database
//! this test creates and drops. Without one of those variables the test prints
//! `SKIP` and returns — CI's plain `cargo test --workspace` does not stand up a
//! database, and a boot test that can only fail there would be noise. The child
//! gets a cleared environment (plus `PATH`) and only the variables below, so the
//! test's own environment cannot leak into the server's configuration.

use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Attempts per hour, for BOTH the register pair and (post-fix) the setup pair.
///
/// Small on purpose: the whole point is that a handful of requests exhausts the
/// setup budget, so the contrast with the register route is observable in
/// seconds. The numbers are passed to the child as configuration, so what is
/// asserted is the *split*, not these particular values.
const MAX: u32 = 2;

/// Requests sent at the setup route. More than `MAX`, so the burst both spends
/// the budget and shows the refusal (429) that proves a limiter is active.
const BURST: usize = 5;

/// A body the setup handler rejects on validation (password shorter than 8).
/// Rejections still spend budget — the limiter middleware runs before the
/// handler — which is what makes a mistyped setup attempt as expensive as a
/// valid one, the behaviour the fix is about.
const SETUP_BODY: &str = r#"{"username":"reconsetup","email":"reconsetup@example.com","password":"seven77"}"#;

/// A body the register handler rejects on validation (malformed address). The
/// assertion is about the *limiter*, not the handler: any status other than 429
/// means the request reached the route's own budget.
const REGISTER_BODY: &str =
    r#"{"username":"reconregister","email":"not-an-address","password":"eight888"}"#;

/// A server that must not outlive the test, even when an assertion panics.
struct ServerGuard(Child);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Drop the throwaway database and its storage directory when the test ends,
/// pass or panic. `Drop` may block, so it drives a minimal runtime of its own
/// for the two statements.
struct DatabaseGuard {
    admin_url: String,
    db_name: String,
    storage: PathBuf,
}

impl Drop for DatabaseGuard {
    fn drop(&mut self) {
        // This runs during unwinding too, and the test body is already inside a
        // Tokio runtime: building a second runtime on this thread panics
        // ("Cannot start a runtime from within a runtime"), and a panic in a
        // destructor aborts the process — which would swallow the assertion
        // that just failed. A plain OS thread has no ambient runtime, so the
        // cleanup is driven there and joined.
        let admin_url = self.admin_url.clone();
        let db_name = self.db_name.clone();
        let _ = std::thread::spawn(move || {
            let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            rt.block_on(async move {
                if let Ok(pool) = sqlx::PgPool::connect(&admin_url).await {
                    let _ = sqlx::query(&format!(
                        "DROP DATABASE IF EXISTS {db_name} WITH (FORCE)"
                    ))
                    .execute(&pool)
                    .await;
                }
            });
        })
        .join();
        let _ = fs::remove_dir_all(&self.storage);
    }
}

/// `url` with its database name replaced. Enough for the URL shapes this test
/// is given (`postgres://user:pass@host:port/db`), and it keeps the credentials
/// out of the test's own logic.
fn with_database(url: &str, db: &str) -> String {
    let (head, query) = match url.split_once('?') {
        Some((head, query)) => (head, format!("?{query}")),
        None => (url, String::new()),
    };
    let head = match head.rfind('/') {
        Some(cut) => &head[..cut],
        None => head,
    };
    format!("{head}/{db}{query}")
}

/// A port nothing is listening on: bind `:0`, take the number, release it. The
/// window between release and the child's bind is a race in principle; the
/// test reports a bind failure with the child's log rather than a bare timeout.
fn free_port() -> u16 {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .expect("bind an ephemeral port")
        .local_addr()
        .expect("local_addr")
        .port()
}

/// One HTTP/1.1 request, status only. Hand-rolled over `TcpStream` so the test
/// carries no client dependency and cannot be affected by one's redirect,
/// retry or connection-pool behaviour — it must see the status the server sent,
/// once, on a fresh connection.
fn request(port: u16, method: &str, path: &str, body: &str) -> Option<u16> {
    let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
    let mut stream = TcpStream::connect_timeout(&addr.into(), Duration::from_secs(5)).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok()?;
    let wire = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(wire.as_bytes()).ok()?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).ok()?;
    let head = String::from_utf8_lossy(&raw);
    let status_line = head.lines().next().unwrap_or_default();
    status_line.split_whitespace().nth(1)?.parse().ok()
}

/// The tail of the child's log, for failure messages.
fn log_tail(path: &Path) -> String {
    let body = fs::read_to_string(path).unwrap_or_else(|e| format!("<unreadable: {e}>"));
    let lines: Vec<&str> = body.lines().collect();
    lines[lines.len().saturating_sub(25)..].join("\n")
}

/// The startup line that reports the four budgets the process built. It is the
/// cheapest way to see from outside which wiring the child actually has: a
/// pre-fix binary has no `setup:` field in it at all.
fn rate_limit_startup_line(log: &Path) -> String {
    fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .find(|line| line.contains("Rate limiting enabled"))
        .unwrap_or("<no 'Rate limiting enabled' line in the log>")
        .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn setup_attempts_do_not_consume_the_register_budget() {
    let Some(db_url) = std::env::var("OXICLOUD_TEST_DB_URL")
        .ok()
        .or_else(|| std::env::var("OXICLOUD_DB_CONNECTION_STRING").ok())
    else {
        eprintln!(
            "SKIP setup_attempts_do_not_consume_the_register_budget: set \
             OXICLOUD_TEST_DB_URL=postgres://user:pass@host:port/db — this test boots the real \
             server against a throwaway database and needs a Postgres to create it in."
        );
        return;
    };

    // ── 1. A fresh database ──────────────────────────────────────────────────
    let db_name = format!("oxicloud_setupbudget_{}", std::process::id());
    let admin_url = with_database(&db_url, "postgres");
    let app_url = with_database(&db_url, &db_name);
    let admin = sqlx::PgPool::connect(&admin_url)
        .await
        .unwrap_or_else(|e| {
            panic!("cannot reach Postgres at {admin_url} to create the test database: {e}")
        });
    // `WITH (FORCE)` (PG 13+) also kicks any session left behind by a previous
    // run of this test, so the run after a panicked run is not stuck.
    let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS {db_name} WITH (FORCE)"))
        .execute(&admin)
        .await;
    sqlx::query(&format!("CREATE DATABASE {db_name}"))
        .execute(&admin)
        .await
        .unwrap_or_else(|e| panic!("CREATE DATABASE {db_name} failed: {e}"));

    // ── 2. Boot the real binary ───────────────────────────────────────────────
    let storage = std::env::temp_dir().join(format!("oxicloud-sbbudget-{}", std::process::id()));
    let _ = fs::remove_dir_all(&storage);
    fs::create_dir_all(&storage).expect("create the storage directory");
    let log_path = storage.join("app.log");
    let log = fs::File::create(&log_path).expect("create the child's log");
    let log_err = log.try_clone().expect("clone the log handle");

    // Declared BEFORE the server guard so it drops AFTER it: the child is killed
    // first, then the database is dropped and the storage directory removed.
    let _db = DatabaseGuard {
        admin_url: admin_url.clone(),
        db_name: db_name.clone(),
        storage: storage.clone(),
    };

    let port = free_port();
    let static_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("frontend/static");
    let spawned = Command::new(env!("CARGO_BIN_EXE_oxicloud"))
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("OXICLOUD_DB_CONNECTION_STRING", &app_url)
        .env("OXICLOUD_SERVER_HOST", "127.0.0.1")
        .env("OXICLOUD_SERVER_PORT", port.to_string())
        .env("OXICLOUD_STORAGE_PATH", &storage)
        .env("OXICLOUD_STATIC_PATH", &static_path)
        .env(
            "OXICLOUD_JWT_SECRET",
            "recon-setup-budget-live-http-test-secret-32+chars",
        )
        .env("OXICLOUD_ENABLE_AUTH", "true")
        .env("OXICLOUD_RATE_LIMIT_REGISTER_MAX", MAX.to_string())
        .env("OXICLOUD_RATE_LIMIT_REGISTER_WINDOW_SECS", "3600")
        // Ignored by a binary that has no setup budget — which is the point of
        // the burst below. A build that reads them gets its own 2-per-hour
        // budget on /api/setup, so both arms send the same requests and differ
        // only in who owns the counter.
        .env("OXICLOUD_RATE_LIMIT_SETUP_MAX", MAX.to_string())
        .env("OXICLOUD_RATE_LIMIT_SETUP_WINDOW_SECS", "3600")
        .env("RUST_LOG", "info")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err))
        .spawn()
        .expect("spawn the oxicloud binary");
    // From here on the child is killed even if an assertion below panics.
    let mut child = ServerGuard(spawned);

    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        if let Some(status) = child.0.try_wait().expect("try_wait on the child") {
            panic!(
                "the server exited during boot ({status}); log tail:\n{}",
                log_tail(&log_path)
            );
        }
        if request(port, "GET", "/api/version", "") == Some(200) {
            break;
        }
        if Instant::now() >= deadline {
            panic!(
                "the server did not answer GET /api/version on 127.0.0.1:{port} within 180 s; \
                 log tail:\n{}",
                log_tail(&log_path)
            );
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    let startup = rate_limit_startup_line(&log_path);

    // ── 3. Spend the setup route's budget ────────────────────────────────────
    let setup_statuses: Vec<u16> = (0..BURST)
        .map(|_| {
            request(port, "POST", "/api/setup", SETUP_BODY)
                .expect("the setup route must answer while the server is up")
        })
        .collect();

    // ── 4. The register route must still have its own quota ──────────────────
    let register_status = request(port, "POST", "/api/auth/register", REGISTER_BODY)
        .expect("the register route must answer while the server is up");

    println!("startup line: {startup}");
    println!("setup attempts (bad password): {setup_statuses:?}");
    println!("register after the setup burst: {register_status}");

    // Guard against a vacuous pass: if the setup route never refused anything,
    // no budget was spent and step 4 proves nothing.
    assert!(
        setup_statuses.contains(&429),
        "the setup route answered {setup_statuses:?} — no 429, so nothing was exhausted and this \
         test would measure nothing; startup line: {startup}"
    );

    assert_ne!(
        register_status, 429,
        "setup attempts: {setup_statuses:?}; register after: {register_status} — the setup route \
         consumed the register budget. /api/setup (first-run wizard) and /api/auth/register must \
         have independent quotas, so a setup burst (or a mistyped setup attempt) cannot lock the \
         other route out. startup line: {startup}"
    );
}
