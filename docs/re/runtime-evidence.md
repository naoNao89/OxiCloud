# Runtime evidence — OxiCloud 0.10.0 (behavioural RE)

This file is the measured record. Every number below was produced by the harness
in `docs/re/scripts/` against the released binary in an isolated sandbox; the
identity chain (tarball → binary → production host) is in
`docs/re/identity.txt`, and the machine-readable corpus is
`docs/re/evidence/A-*.jsonl` with the case tables in
`docs/re/evidence/A-cases.md` (primary set) and
`docs/re/evidence/A-loaded-cases.md` (loaded set).

**Evidence tiers used throughout**

* **Observed** — measured at the instrumented boundary in this sandbox:
  HTTP status/headers/body bytes, the app's own stdout (`RUST_LOG=debug`), the
  Postgres statement log (`log_statement=all`), and `psql` state probes.
* **Predicted** — read out of the source tree at checkout `389e161b`, which is
  *later* than the release commit `496d25b`; used only to say what the code
  shape suggests, never as evidence of the binary's behaviour.
* **Hypothesis** — what the case is designed to discriminate, stated before the
  numbers.

Nothing here is inferred from the source when a black-box measurement exists.

---

## How it was produced

Sandbox: `rustdev` (Debian 13 trixie, x86_64, 4 vCPU, 8 GiB), binary
`9d364fdd83f49d19dd086726b88830c2f95adc485001b16ad663dacfb665e080`
(= the production host's `/opt/oxicloud/releases/0.10.0/oxicloud`).

```
# migrate-template + full fixed-order corpus (17 records)
bash scripts/run-corpus.sh http://127.0.0.1:18086 /data/re-scratch/out A

# 4 extra postures, 9 experiment cases, one JSONL per posture
bash scripts/run-rate-limit-experiments.sh http://127.0.0.1:18086 /data/re-scratch/out A
```

Both scripts boot the binary themselves and, because the rate limiter is a
per-process in-memory cache, **restart the instance and reset the DB from a
migrated template before every case** — so case order cannot leak budget or DB
state, and a re-run produces the same records.

The committed scripts are byte-identical to the copies that ran in the sandbox
(`sha256sum` matched on both sides):

```
ded266c3dd08eeb2d2d858263197122e00378d70fab56c432daaa8eac103938b  lib-re.sh
4c3406e65cdda5b8ddd0f261ab3640560d5a534773d07f590a19f2048e4ddd20  run-corpus.sh
1dcd8fe81331e78c66000897bc587685584ca605af0f8f2c924cab906d240c91  run-rate-limit-experiments.sh
```

`lib-re.sh` moved from `271b7880…` (the exact revision that produced the
delivered JSONL) to `ded266c3…` by **comment lines only**: the header's
description of which label was contended, and the determinism counts, were
corrected after the files were delivered. Executable code is unchanged — verify
by stripping comments from both revisions:

```
# 271b7880 = the revision that produced the delivered evidence (kept in the
# sandbox as $RE_ROOT/scripts/lib-re.sh.ran-271b7880)
shasum -a 256 <(sed '/^[[:space:]]*#/d' lib-re.sh.ran-271b7880) \
              <(sed '/^[[:space:]]*#/d' docs/re/scripts/lib-re.sh)
# both: 4c26591992b415649ae6081e4970663a80acdd81a09b574a471dc0a9836455ae   (verified)
```

`docs/re/scripts/diff-labels.py` is the checker for the determinism claim below
(A.21); the two load-bearing raw Postgres windows are committed as
`docs/re/evidence/A-setup-pw7-step1.pg.log` and
`docs/re/evidence/A-rate-limit-6x-step6.pg.log` (sha256 equal to the
`pg_log_window.sha256` recorded in `A.jsonl` for those steps).

Two label sets are delivered, identical harness, different box load:

* **`A-*.jsonl`** — the primary set, produced by the exact committed revision
  above on a **quiet** box, in one run of `run-corpus.sh` followed by one run of
  `run-rate-limit-experiments.sh`. Every number in this report is from this set
  unless it is explicitly marked otherwise.
* **`A-loaded-*.jsonl`** — the same two scripts one revision earlier, taken
  while ~2 of the 4 vCPUs were being consumed by unrelated processes (see the
  background-load note in A.0). Kept because §A.2/A.3/A.4 quote the loaded
  variant beside the quiet one. Its per-case raw windows on the sandbox were
  re-used by the quiet run, so only its scalars are meaningful — which is all
  this report quotes from it. A direct comparison of the two sets is in A.21.

### Postures (each number below is labelled with the one that produced it)

All three limiter windows are quoted for every posture, verbatim from each
instance's own `startup.ratelimit.txt`:

| label | port | env | startup line (`Rate limiting enabled — …`) |
|---|---|---|---|
| `A` (corpus) | 18086 | *(release posture — no `OXICLOUD_RATE_LIMIT_*`)* | `login: 10/60 s, register: 5/3600 s, refresh: 20/60 s` + `trusted proxy CIDRs: none` |
| `A-rl3600` | 18087 | *(release posture)* | `login: 10/60 s, register: 5/3600 s, refresh: 20/60 s` + `trusted proxy CIDRs: none` |
| `A-xff` | 18090 | `OXICLOUD_TRUST_PROXY_CIDR=127.0.0.1/32` | `login: 10/60 s, register: 5/3600 s, refresh: 20/60 s` + `trusted proxy CIDRs: 127.0.0.1/32` |
| `A-max1win15` | 18089 | `REGISTER_MAX=1`, `REGISTER_WINDOW_SECS=15` | `login: 10/60 s, register: 1/15 s, refresh: 20/60 s` + `trusted proxy CIDRs: none` |
| `A-regwin15` | 18088 | `REGISTER_MAX=100`, `REGISTER_WINDOW_SECS=3600` | `login: 10/60 s, register: 100/3600 s, refresh: 20/60 s` + `trusted proxy CIDRs: none` |

(All five instances were booted with `RUST_LOG=debug` and the same DB user; the
login and refresh windows are not overridden anywhere in this work.)

The release-posture numbers in this section are **only** from `A` and
`A-rl3600`. The 15-second-window numbers are **only** from `A-max1win15`. They
are never mixed.

### What the JSONL gives the differential

Per step, in the order dispatched: `status`, response headers, `body_len`
(transferred), `body_decoded_len` (after undoing `content-encoding`),
`elapsed_ms`/`ttfb_ms` (curl's monotonic timer), `handler_ran` (request-id
scoped), `app_sql_statement_count` + `app_sql_statement_summaries` (the exact
ordered SQL templates logged inside that request's span), `resolved_client_ip`,
`limiter_ip`, plus `app_log_lines` as a *stable projection* (tracing span prefix
and timing fields removed) so two builds diff without noise.

**Trimming rule (so the committed artifact stays reviewable).** Every scalar
field, `state_before`/`state_after` and the per-request SQL summaries are inline.
The raw Postgres windows are *not* pasted: each step carries
`pg_log_window: {path, lines, sha256}` and the case carries
`pg_case_log_window`, with the full window left in the sandbox
(`/data/re-scratch/out/cases/<case_id>/…`, paths are relative to the corpus
directory). Pasted windows for 5 postures × N builds would be ~10 MB of JSONL in
a PR. `RE_FULL_LOGS=1` inlines them instead. `app_log_lines` is the single
exception kept inline: it is a stable projection, ~40 KB for a whole corpus, and
it carries the *reason* behind `handler_ran` (handler markers,
`setup.claim_released`, `Rate limit exceeded`).

The raw verbatim windows that this report quotes are from the sandbox paths
listed in `docs/re/identity.txt` §6.

---

## A — original released binary

### A.0.1 Verdict table

Eight questions, answered from measurement. Delivered label convention —
**`A-*`** is the pass taken on the **quiet** box (DB clock 14:47:22 → 14:49:26,
after the unrelated CPU load had been removed) and is the primary set: every
number below is from it. **`A-loaded-*`** is the pass that **started under that
load** (DB clock 14:40:36 → 14:42:41; the load was removed at ≈14:41, so only its
first half is contended) and is kept purely as a cross-check. Where a number is
an elapsed time or a race outcome the quiet value is quoted with the loaded value
beside it; every status, byte length, state counter and status multiset is
identical in both (see the determinism check in A.21).

| # | question | verdict | evidence |
|---|---|---|---|
| 1 | Does a 429 return before the setup handler runs? | **Yes.** No `Setup admin request received`, `handler_ran=false`, `app_sql_statement_count=0`, and the step's own PG window contains only background scheduler lines | A.1 (`rate-limit-6x` step 6) |
| 2 | Does a rejected request change the counter/deadline, and can you show the deadline moved? | **Yes — every request, accepted or rejected, re-arms the window.** Refused at +21.94 s although the original 15 s window had expired; refused at +27.23 s although the window re-armed at +9.65 s (expiry +24.65 s). The idle control expires normally (400 at +17.33 s) | A.2 (`ttl-refresh` + `ttl-refresh-control`, quiet label) |
| 3 | Is `Retry-After` a real remaining time or a constant? | **Constant** = the configured window. Still `3600` at +22.01 s into a 3600 s window (body `retry_after_secs:3600`); `15` in the 15 s posture | A.3 |
| 4 | Can two concurrent requests read the same counter before writing? | **Not observed, not refuted.** 8-way and 16-way simultaneous bursts on `max=1` served exactly one request each, on both the loaded and the quiet box; no lost update is proven | A.4 (`concurrent-burst-rejects-8/16`) |
| 5 | Which IP does the trusted-proxy path pick with several XFF hops? | **The leftmost hop, verbatim** (`6.6.6.6` from `6.6.6.6, 10.0.0.5`), only when the peer is inside the trusted CIDR; `XFF: 127.0.0.1` shares the bare-peer bucket | A.5 (`xff-honoured-trusted`, `xff-ignored-untrusted`) |
| 6 | Does a validation failure release the initialization claim? | **Yes — the row is deleted, so it is ABSENT, not `updated_by=NULL`/`Uuid::nil()`.** Measured sequence: `SELECT` → `INSERT … ON CONFLICT DO NOTHING` (`$1='00000000-…'`) → `DELETE`; post-case `si_rows=0` | A.6 (`setup-pw7`) |
| 7 | Do other middlewares (DPoP/CSRF) change the rejection order for `/api/setup`? | **No.** A request with no auth/DPoP/CSRF/cookies gets `201`; order is limiter → method routing → `Json` extraction → handler | A.7 |
| 8 | Which exact branch serializes the 400 bodies you observed? | **Two:** axum extractor rejections (`text/plain`) for 415/422/parse-400 with no handler run, and the app's compact JSON envelope (`68 + 2·len(message) + len(error_type)`) for domain validation, split into the bare password message and the `Error creating admin user: …` prefix family | A.8 |

### A.0 The environment the cases ran in

```
$ for l in A A-rl3600 A-xff A-max1win15 A-regwin15; do
    sed -e 's/^[0-9T:.Z-]* *//' /data/re-scratch/run-$l/startup.ratelimit.txt
    sed -e 's/^[0-9T:.Z-]* *//' /data/re-scratch/run-$l/startup.trustedproxy.txt
  done
 INFO oxicloud: Rate limiting enabled — login: 10/60 s, register: 5/3600 s, refresh: 20/60 s
 INFO oxicloud::interfaces::middleware::trusted_proxy: trusted proxy CIDRs: none — X-Forwarded-For/X-Real-Ip headers will be ignored
 INFO oxicloud: Rate limiting enabled — login: 10/60 s, register: 5/3600 s, refresh: 20/60 s
 INFO oxicloud::interfaces::middleware::trusted_proxy: trusted proxy CIDRs: none — X-Forwarded-For/X-Real-Ip headers will be ignored
 INFO oxicloud: Rate limiting enabled — login: 10/60 s, register: 5/3600 s, refresh: 20/60 s
 INFO oxicloud::interfaces::middleware::trusted_proxy: trusted proxy CIDRs: 127.0.0.1/32
 INFO oxicloud: Rate limiting enabled — login: 10/60 s, register: 1/15 s, refresh: 20/60 s
 INFO oxicloud::interfaces::middleware::trusted_proxy: trusted proxy CIDRs: none — X-Forwarded-For/X-Real-Ip headers will be ignored
 INFO oxicloud: Rate limiting enabled — login: 10/60 s, register: 100/3600 s, refresh: 20/60 s
 INFO oxicloud::interfaces::middleware::trusted_proxy: trusted proxy CIDRs: none — X-Forwarded-For/X-Real-Ip headers will be ignored
```

(in that order: `A`, `A-rl3600`, `A-xff`, `A-max1win15`, `A-regwin15`; each file
holds the most recent boot of its label, i.e. `A`'s is timestamped
`14:47:56` — inside the delivered quiet span.)

DB: PostgreSQL 18.2 (compose pin; production runs 17.11 — see identity.txt §3),
`log_statement=all`, `log_min_duration_statement=0`, `log_connections=on`,
112 migrations applied by the binary itself.

#### Background load — part of the environment, recorded

This sandbox is shared. While the runs labelled **`A-loaded-*`** were being taken
(DB clock 14:40:36 → 14:42:41), two unrelated `driver-libfuzzer` processes (from
another project's tree, `/data/home/fcitx5-lotus`) were continuously CPU-bound —
**149 594 CPU-seconds over 149 744 s of lifetime, times/etimes ≈ 0.999, ≈ 2 of
the 4 vCPUs** — plus a stray idle `coreutils tail --follow` (PID 16134, ≈ 54 h).
A `/proc/stat` jiffy delta taken between two of this harness's requests read
**busy 433 / idle 365 of 798 ≈ 54 % busy**; the same measurement after those
processes were killed (on the operator's instruction; their corpora were not
touched) read **busy 0 / idle 799 of 799**. The load was removed at ≈14:41, i.e.
part-way through that run: its corpus half (14:40:36 → 14:41:13) is definitely
contended, its later postures (14:41:14 → 14:42:41) may already have been
running on a quiet box. That ambiguity is why the quiet `A-*` set exists and is
the one quoted. (These two numbers are the main agent's measurement on this host, not
mine; both samples were taken while my harness sat between requests, so the delta
is background load and not this work.)

Consequence for reading the evidence: cases whose **assertion** is an elapsed
wall-clock value (`ttl-refresh`, `retry-after-constant`) or a **race outcome**
(the `concurrent-*` bursts) are reported from the quiet label **`A-*`**, with the
`A-loaded-*` numbers kept beside them. Cases asserting an ordering, status, byte
length or DB state are load-independent and are reported from `A-*`; the two
sets agree on all of them — verified mechanically, see A.21.

---

### A.1 Q1 — Does a 429 return before the setup handler runs?

**Hypothesis.** If the rate limiter is a router-level layer, a rejected request
never reaches `Json` extraction or the handler: no `Setup admin request
received` log line, and no SQL issued inside that request's span.

**Command** (case `rate-limit-6x`, release posture, from `run-corpus.sh`):

```
re_case_begin rate-limit-6x "..."
for i in 1 2 3 4 5 6; do re_case_step "post-$i" POST /api/setup application/json "$FX/pw7.json"; done
re_case_end
```

**Observed** (verbatim from `A.jsonl`, steps 1–6):

```
post-1 -> 400 len=167 handler=True  sql=5
post-2 -> 400 len=167 handler=True  sql=5
post-3 -> 400 len=167 handler=True  sql=5
post-4 -> 400 len=167 handler=True  sql=5
post-5 -> 400 len=167 handler=True  sql=5
post-6 -> 429 len=53  handler=False sql=0   retry_after=3600  elapsed_ms=2.696
```

The app's own log line for the rejected request (`app_log_lines`, verbatim
strip):

```
oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=127.0.0.1 endpoint="register"
http::api: client_error status=429 content_type="application/json"
```

and there is **no** `Setup admin request received` line, **no** `sqlx::query`
line for that request id, and `app_sql_statement_count = 0`.

**Predicted** (HEAD): `main.rs` layers `rate_limit_register` onto the whole
`setup_route()` router, and `enforce()` returns `too_many_requests(...)` without
calling `next.run(req)`.

**Answer: yes.** The 429 is produced by the register limiter middleware before
extraction and before the handler; the request causes zero DB round-trips. The
rejected request also names the limiter it hit: `endpoint="register"`.

*Caveat, measured:* the PG window for that step is **not** empty — it holds
`pg_log_line_count: 123` **log lines**, and none of them is attributable to the
request (committed verbatim as
`docs/re/evidence/A-rate-limit-6x-step6.pg.log`, `sha256`
`c8d136f068f3961f9537316ce92df1c84d8239872ec1af2a63ff5cc07a8e5fb2`).
They are one background `storage.tree_etag_dirty` sweep (a `WITH drained AS (
DELETE FROM storage.tree_etag_dirty …)` statement plus its `DETAIL` lines)
belonging to the app's scheduler. The field is therefore named
`pg_log_line_count` (lines, not statements) and the raw array is kept on disk
(`pg_log_window.path`). The per-request oracle is
`app_sql_statement_count` / `app_sql_statement_summaries`, both 0 here. Any
reader who treats the PG window as "what this request did" would conclude the
opposite of what was measured.

---

### A.2 Q2 — Does a rejected request change the counter/deadline (and can you show the deadline moved)?

**Hypothesis.** If the counter is written back on every request including
rejections (read → `count+1` → `insert`, where `insert` re-arms the entry TTL),
then a burst of rejections keeps pushing the expiry out, and an "expired by
now" request is still refused.

**Command** (case `ttl-refresh`, posture `A-max1win15`: `register 1/15 s`):

```
re_case_step t0-req1 POST /api/setup ...      # 400
re_case_step t0-req2 POST /api/setup ...      # 429
sleep 9;  re_case_step t9-req3  ...           # 429
sleep 12; re_case_step t21-req4 ...           # 429  (t > the ORIGINAL 15 s deadline)
sleep 5;  re_case_step t26-req5 ...           # 429
```

**Observed** (monotonic timestamps from `wall_epoch_us` deltas, body bytes from
the wire):

```
t0-req1  t=+ 0.000s -> 400 len=167 sql=5 handler=True
t0-req2  t=+ 0.325s -> 429 len=51  sql=0 handler=False  retry_after=15
t9-req3  t=+ 9.650s -> 429 len=51  sql=0 handler=False  retry_after=15
t21-req4  t=+21.944s -> 429 len=51 sql=0 handler=False  retry_after=15
t26-req5  t=+27.230s -> 429 len=51 sql=0 handler=False  retry_after=15
```
(quiet label `A`; the loaded label `A-loaded` gave 0.330 / 9.681 / 22.052 /
27.353 with the same statuses — the verdict does not depend on the load, which
is why the loaded set is kept as a cross-check rather than re-litigated.)

**Control** (case `ttl-refresh-control`, same posture, fresh process, no
activity inside the window):

```
c0-req1  t=+ 0.000s -> 400 len=167 sql=5 handler=True
c17-req2 t=+17.332s -> 400 len=167 sql=5 handler=True     # idle > 15 s => allowed again
```
(loaded label: 17.340 s, same statuses)

**Observed, restated.** The request at +21.94 s is refused although the original
15 s window from t=0 had expired at +15 s, and the request at +27.23 s is refused
although the window re-armed at +9.65 s would have expired at +24.65 s — because
the rejection at +21.94 s re-armed it again. The idle control proves the same
limiter config *does* expire when nothing touches it.

**Predicted** (HEAD): `RateLimiter::check_and_increment` does
`let count = cache.get(ip).unwrap_or(0) + 1; cache.insert(ip, count);` on every
call, and the source comment states insertion resets the TTL.

**Answer: yes — every request, accepted or rejected, increments the counter and
re-arms the deadline.** A client that keeps retrying during a lockout extends
its own lockout indefinitely. *Prediction that failed:* I expected the +27.2 s
request to be allowed (window re-armed at +9.65 s ⇒ expiry +24.65 s). It was
refused, which is only consistent with the +21.94 s rejection also re-arming the
deadline. The finding is stronger than the prediction; the control shows the
window does expire when idle, so this is TTL-refresh, not a permanent latch.

---

### A.3 Q3 — Is `Retry-After` a real remaining time or a constant?

**Hypothesis.** If `Retry-After` is the configured window rather than the
residual time, then 20 s into a 3600 s window it still reads 3600.

**Command** (case `retry-after-constant`, posture `A-rl3600`, release
5/3600 s): five POSTs (all served), a sixth (429), `sleep 20`, a seventh.

**Observed:**

```
post-1..post-5  -> 400 len=167   (limiter slots 1..5)
post-6          -> 429 len=53  retry_after=3600      at t=+1.67 s
post-after-20s  -> 429 len=53  retry_after=3600      at t=+22.01 s  (20.34 s later)
```

Both 429 bodies are byte-identical:

```
{"error":"Too many requests","retry_after_secs":3600}
```

**Predicted** (HEAD): `too_many_requests(limiter.retry_after())` and
`retry_after()` returns `self.window_secs`.

**Answer: constant, not a countdown.** `Retry-After`/`retry_after_secs` is the
configured window (3600 here, 15 in the `A-max1win15` posture) and does not
shrink with elapsed time. Consequence for clients: after a partial wait they
must compute the residual themselves or re-probe — and per A.2, re-probing
re-arms the window.

---

### A.4 Q4 — Can two concurrent requests read the same counter before writing?

**Hypothesis.** `get()` then `insert()` is a read-then-write pair. Under a
simultaneous burst, two requests could both read `count = 0` on a `max=1`
limiter and both be served. A count of served > 1 would prove at least one such
pair; a count of exactly 1 proves nothing about atomicity.

**Command** (cases `concurrent-burst-rejects-8` and
`concurrent-burst-rejects-16`, posture `A-max1win15` = `register 1/15 s`, fresh
process/DB before each):

```
re_case_burst 8  cbr   POST /api/setup application/json "$FX/pw7.json"
re_case_burst 16 cbr16 POST /api/setup application/json "$FX/pw7.json"
```

`re_case_burst` starts N curl processes that all block on one barrier file and
are released by a single `: > $go`, so the requests are genuinely simultaneous.

**Observed:**

```
quiet label `A`      concurrent-burst-rejects-8 :  429,429,429,429,429,429,400,429   served=1 (step 7)  refused=7
quiet label `A`      concurrent-burst-rejects-16:  400 then 429 × 15                 served=1 (step 1)  refused=15
loaded label `A-loaded` 8-way :  served=1 (step 7)
loaded label `A-loaded` 16-way:  served=1 (step 12)
```

(Each `400` has `sql=5, handler_ran=True` — it really reached the handler; each
`429` has `sql=0`.)

**Predicted** (HEAD): the source comment on `check_and_increment` says
"Read-then-write is not atomic, but it never was — under a concurrent burst both
shapes can undercount the same way, which only makes the limiter marginally
lenient, never wrongly strict."

**Answer: not observed — and not refuted.** In four independent samples (8- and
16-way bursts, on the loaded and on the quiet box) exactly one request was
served each time, which is what an atomic check would also produce. The read-then-write shape means the
race *exists in the code shape* (Predicted), but **no lost update is proven by
this measurement**, and I am not claiming one. Proving it would need either a
much wider burst, a deterministic interleaving instrument, or instrumentation
inside the cache — none of which this harness does.

---

### A.5 Q5 — Which IP does the trusted-proxy path pick with several XFF hops?

**Hypothesis.** With the TCP peer inside `OXICLOUD_TRUST_PROXY_CIDR`, the
resolved client IP is the **leftmost** `X-Forwarded-For` hop, used verbatim as
the bucket key; the value may be unparseable and still keys a bucket.

**Command** (case `xff-honoured-trusted`, posture `A-xff`, peer = 127.0.0.1,
trusted 127.0.0.1/32):

```
re_case_step xff-6-1 POST /api/setup ... -H 'X-Forwarded-For: 6.6.6.6, 10.0.0.5'
re_case_step xff-7-1 POST /api/setup ... -H 'X-Forwarded-For: 7.7.7.7, 10.0.0.5'
re_case_step xff-6-2 POST /api/setup ... -H 'X-Forwarded-For: 6.6.6.6, 10.0.0.5'
re_case_step xff-7-2 POST /api/setup ... -H 'X-Forwarded-For: 7.7.7.7, 10.0.0.5'
re_case_step peer-1   POST /api/setup ...                      # no XFF
re_case_step xff-peer POST /api/setup ... -H 'X-Forwarded-For: 127.0.0.1'
re_case_step peer-2..peer-5 POST /api/setup ...                # no XFF
re_case_step xff-6-3  POST ... -H 'X-Forwarded-For: 6.6.6.6, 10.0.0.5'
re_case_step xff-7-3  POST ... -H 'X-Forwarded-For: 7.7.7.7, 10.0.0.5'
re_case_step victim-1 POST ... -H 'X-Forwarded-For: 10.0.0.9'
```

**Observed** (`resolved_client_ip` = what the request span resolved,
`limiter_ip` = the key the limiter logged on a 429):

```
step        status  resolved_client_ip  limiter_ip
xff-6-1     400     6.6.6.6             -
xff-7-1     400     7.7.7.7             -
xff-6-2     400     6.6.6.6             -
xff-7-2     400     7.7.7.7             -
peer-1      400     127.0.0.1           -
xff-peer    400     127.0.0.1           -
peer-2      400     127.0.0.1           -
peer-3      400     127.0.0.1           -
peer-4      400     127.0.0.1           -
peer-5      429     127.0.0.1           127.0.0.1      <- bucket exhausted
xff-6-3     400     6.6.6.6             -              <- still alive, count 3
xff-7-3     400     7.7.7.7             -              <- still alive, count 3
victim-1    400     10.0.0.9            -              <- its own third bucket
```

Which bucket was exhausted is therefore not inferred from statuses alone: the
`127.0.0.1` bucket absorbed exactly 5 requests (peer-1, the XFF=127.0.0.1
request, peer-2..4); the 6th request to it (peer-5) was refused **while** the
`6.6.6.6` and `7.7.7.7` buckets continued to be served. Interleaving the two
spoofed values never exhausted either bucket, so they are distinct keys.

**Counter-case** (case `xff-ignored-untrusted`, posture `A`, trusted list
empty, different XFF on every request):

```
xff-1 (XFF 6.6.6.6) 400 / xff-2 (7.7.7.7) 400 / xff-3 (8.8.8.8) 400
xff-4 (9.9.9.9)     400 / xff-5 (none)    400 / xff-6 (none) 429
```

All six landed in the single `127.0.0.1` bucket: the header is ignored when the
peer is not trusted.

**Predicted** (HEAD): `client_ip_display_from_parts` takes
`xff.split(',').next().map(str::trim)`, i.e. the leftmost hop, and only when
`is_trusted_proxy(peer.ip())`.

**Answer:** the **leftmost** `X-Forwarded-For` hop, taken verbatim (never
validated as an IP: `10.0.0.9` and `6.6.6.6` are simply strings here), and only
when the TCP peer is inside the trusted CIDR. A single-hop value equal to the
peer's own address (`X-Forwarded-For: 127.0.0.1`) maps onto the **same** bucket
as a request with no header — so a trusted-proxy deployment lets a client pick
any bucket it likes, including one that is already exhausted.

---

### A.6 Q6 — Does a validation failure release the initialization claim?

**Hypothesis.** The handler claims initialization with an
`INSERT … ON CONFLICT DO NOTHING` carrying `Uuid::nil()`, then creates the
admin. If creation fails, the claim must be deleted, otherwise every retry
would see "already initialized".

**Command** (case `setup-pw7`, release posture):

```
re_case_step post-pw7 POST /api/setup application/json "$FX/pw7.json"
```

**Observed — HTTP:** `400`, `application/json`, 167 bytes:

```
{"status":"400 Bad Request","error":"Password must be at least 8 characters long","message":"Password must be at least 8 characters long","error_type":"Invalid Input"}
```

**Observed — app log (request-scoped, verbatim):**

```
oxicloud::interfaces::api::handlers::auth_handler: Setup admin request received for user: admin
oxicloud::interfaces::api::handlers::auth_handler: Setup admin creation failed: Invalid Input: Password must be at least 8 characters long
audit: 🔄 setup admin failed; released initialization claim so retries can proceed event="setup.claim_released" reason=Invalid Input: Password must be at least 8 characters long
```

**Observed — the database saw exactly this sequence** (`case.pg.log`, PID in
brackets, timestamps from the database's own clock):

```
14:47:22.595 [34434] LOG:  execute sqlx_s_2: SELECT value FROM auth.admin_settings WHERE key = $1
14:47:22.595 [34434] DETAIL: Parameters: $1 = 'system_initialized'
14:47:22.595 [34430] LOG:  parse sqlx_s_3: INSERT INTO auth.admin_settings (key, value, category, is_secret, updated_by, updated_at)
                              VALUES ('system_initialized', 'true', 'system', false, $1, NOW())
                              ON CONFLICT (key) DO NOTHING
14:47:22.595 [34430] DETAIL: Parameters: $1 = '00000000-0000-0000-0000-000000000000'
14:47:22.599 [34433] LOG:  execute sqlx_s_8: DELETE FROM auth.admin_settings WHERE key = $1
14:47:22.599 [34433] DETAIL: Parameters: $1 = 'system_initialized'
```
(raw window `cases/setup-pw7/01-post-pw7.pg.log`, committed verbatim as
`docs/re/evidence/A-setup-pw7-step1.pg.log`; `sha256`
`7a28da896efabf55863902f37545f5a2d553a92df2ee8d9c28badc81612ae34f`, the same value
`A.jsonl` records in that step's `pg_log_window`.)

and the state probes before/after the case:

```
state_before  users=0 owners=0 si_rows=0 si_value="" si_updated_by=""
state_after   users=0 owners=0 si_rows=0 si_value="" si_updated_by=""
```

**Predicted** (HEAD): `try_claim_initialization` = `INSERT … ON CONFLICT DO
NOTHING` with `Uuid::nil()`; on `setup_create_admin` error the handler calls
`release_initialization_claim()`, which is `DELETE FROM auth.admin_settings
WHERE key = $1`.

**Answer: yes, and the "nil vs absent" question is decided: ABSENT.** The claim
row exists only inside the request (14:47:22.595 → 14:47:22.599 — 4 ms) and is
physically deleted, not left behind with `updated_by = NULL` or `= Uuid::nil()`.
Post-case state has **zero** `system_initialized` rows. This is why the operator
can retry after a validation failure.

Note the two writes come from **different pooled connections** (PIDs 34430 and
34433) — the claim and its release are not in one transaction, so a crash between
them would leave the placeholder row (`updated_by = 00000000-…`) behind. I did
not try to induce that.

---

### A.7 Q7 — Do other middlewares (DPoP/CSRF) change the rejection order for `/api/setup`?

**Hypothesis.** `/api/setup` is a public one-shot route; DPoP and CSRF layers
are attached only to the protected route groups, so they never run here.

**Observed.** Every `201` in the corpus was obtained by a request carrying **no
`Authorization`, no DPoP proof, no CSRF token and no cookies** (plain
`curl -X POST -H 'Content-Type: application/json'`), e.g. case
`terminal-201-then-403`:

```
valid-1 -> 201 len=159
{"id":"fb19bedd-23f1-4247-96f9-08a4fab8e50b","username":"admin","email":"admin@example.test","role":"owner","image":null,"is_external":false,"is_online":false}
```

No case in the corpus produced a DPoP, CSRF, 401 or 403-with-`DpopRequired`
response. The only 403 observed carries `error_type:"SystemAlreadyInitialized"`.

**Predicted** (HEAD): `main.rs` builds `setup_router` from `setup_route()` with
only `rate_limit_register` attached and merges it into the public API; the
`auth_middleware` / `require_dpop_layer` / `csrf_middleware` layers are on
`auth_protected_routes`, `protected_api`, `app_password_routes` and the device
protected routes.

**Answer: no — DPoP and CSRF do not participate in `/api/setup`. The measured
rejection order is: register rate limiter → method routing → `Json` extraction
(content-type, then parse) → handler (already-initialized pre-check → claim →
create → release/finalize).**

---

### A.8 Q8 — Which exact branch serializes the 400 bodies you observed?

**Hypothesis.** Two different serializers produce the 400s: the handler's
`AppError` JSON envelope for domain failures, and axum's own extractor rejection
(a plain-text body) for 415/422.

**Observed.** All seven setup-path rejections, with `Content-Type`, body bytes
and the SQL count for that request:

| case | status | content-type | bytes | sql | body (verbatim) |
|---|---|---|---|---|---|
| `setup-content-type-text` | 415 | `text/plain; charset=utf-8` | 54 | 0 | `Expected request with `Content-Type: application/json`` |
| `setup-content-type-absent` | 415 | `text/plain; charset=utf-8` | 54 | 0 | *(same)* |
| `setup-json-syntax-error` | 400 | `text/plain; charset=utf-8` | 89 | 0 | `Failed to parse the request body as JSON: EOF while parsing an object at line 1 column 72` |
| `setup-json-missing-field` | 422 | `text/plain; charset=utf-8` | 99 | 0 | `Failed to deserialize the JSON body into the target type: missing field `email` at line 1 column 42` |
| `setup-json-wrong-type` | 422 | `text/plain; charset=utf-8` | 134 | 0 | `Failed to deserialize the JSON body into the target type: username: invalid type: integer `123`, expected a string at line 1 column 15` |
| `setup-pw7` | 400 | `application/json` | 167 | 5 | `{"status":"400 Bad Request","error":"Password must be at least 8 characters long",…}` |
| `setup-pw8-bademail` | 400 | `application/json` | 219 | 5 | `{"status":"400 Bad Request","error":"Error creating admin user: Validation error: Invalid email: missing @",…}` |

**Predicted** (HEAD), the two JSON sites:

* `src/interfaces/errors.rs` — the length oracle test in that file pins the
  compact envelope `{"status","error","message","error_type"}` with a **68-byte**
  fixed skeleton, and names `"Password must be at least 8 characters long"` as
  one of the real message/type pairs.
* `src/application/services/auth_application_service.rs:1391` —
  `format!("Error creating admin user: {}", e)` wraps every `User::new(...)`
  validation failure, which is why the **email/username** branches (and only
  those) carry the `Error creating admin user: ` prefix.

**Answer:** the four `400/415/422` bodies from `Content-Type`/JSON-parse
problems are axum extractor rejections — plain text, `text/plain;
charset=utf-8`, handler never runs (`sql=0`). The two `400`s from domain
validation are the app's own compact JSON envelope
(`status`/`error`/`message`/`error_type`), and they split into two message
families: the password-length check emits the bare message, while
`User::new(...)` validation failures are prefixed `Error creating admin user: `.

**The measured lengths are not approximations — they are exactly the
analytical oracle.** The compact envelope is
`{"status","error","message","error_type"}` with `error == message`, so
`len(body) = skeleton(status) + 2·len(message) + len(error_type)` where the
skeleton (fixed keys, punctuation and the status literal, with empty values) is
**68 bytes** for `"400 Bad Request"` and **66** for `"403 Forbidden"`. Every
observed length reproduces exactly:

| case | skeleton | message | `error_type` | computed | measured |
|---|---|---|---|---|---|
| `setup-pw7` | 68 | 43 | `Invalid Input` (13) | **167** | 167 |
| `setup-pw8-bademail` | 68 | 69 | `Invalid Input` (13) | **219** | 219 |
| `setup-email-nodot` | 68 | 85 | `Invalid Input` (13) | **251** | 251 |
| `setup-email-forbidden` | 68 | 89 | `Invalid Input` (13) | **259** | 259 |
| `setup-username-badchars` | 68 | 118 | `Invalid Input` (13) | **317** | 317 |
| `terminal-201-then-403` (2nd call) | 66 | 67 | `SystemAlreadyInitialized` (24) | **224** | 224 |

and the limiter body is `{"error":"Too many requests","retry_after_secs":W}` =
53 bytes for `W=3600` (measured 53) and 51 for `W=15` (measured 51). Two
independent channels agree on the byte counts: the recorded `body_len` (curl's
`size_download`, off the socket) and `wc -c` of the saved body file. That the
oracle lands on the number the harness measured is the strongest available sign
that the harness measures the right bytes.

---

### A.9 Case 1 — `setup-pw7` (7-character password)

Command / observed / predicted: see **A.6** (this is the same case; it is the
claim-release probe and the smallest 400 body).

```
status 400  bytes 167  content-type application/json  sql 5  handler_ran true
state_after users=0 owners=0 si_rows=0        # init stays false, no claim left
```

---

### A.10 Case 2 — `setup-pw8-bademail` — **the 219-byte candidate**

**Hypothesis.** The operator's observed 219-byte body is the raw JSON length of
the missing-`@` email branch, not a compressed size.

**Command:**

```
re_case_step post-bademail POST /api/setup application/json "$FX/pw8-bademail.json"
# body: {"username":"admin","email":"admin","password":"12345678"}
```

**Observed:**

```
status 400   content-type application/json   content-encoding (none)   bytes 219   sql 5
```

Body, verbatim and complete (219 bytes, `wc -c` agrees):

```
{"status":"400 Bad Request","error":"Error creating admin user: Validation error: Invalid email: missing @","message":"Error creating admin user: Validation error: Invalid email: missing @","error_type":"Invalid Input"}
```

**Observed — compression cannot be involved.** The compression bracket (A.22)
sends `Accept-Encoding: gzip` on this exact body: the response is still
**uncompressed**, 219 bytes transferred, `Content-Encoding` absent. Bodies at
251 bytes are also uncompressed with gzip offered; the first compressed response
in the corpus is at 259 bytes (→151 transferred). The threshold therefore sits
between 251 and 259 bytes; 219 is far below it.

**Predicted** (HEAD): `SizeAbove::new(256)` is part of the global compression
predicate in `main.rs`, and the predicate tests the **decoded body length** — so
a body whose *raw* (decoded) length is ≤ 256 bytes is never compressed; a body
larger than that is compressed, and its **transferred** size can then sit well
below 256 (259 → 151 in the bracket). The measured boundary therefore has to be
read on the decoded side: 251 B is the largest decoded body that arrives
uncompressed, 259 B is the first decoded body that arrives compressed. A ≤256
*transferred* size is not evidence of anything — it can be a compressed larger body.

**Answer (labelled CANDIDATE, per the ticket).** The 219-byte body the operator
saw matches, byte-for-byte in length, the **raw, uncompressed** JSON of the
"Invalid email: missing @" branch, and that branch cannot be compressed at 219
bytes under this binary's compression policy. So the 219 is a *raw* length. I
did not observe the operator's original response, so I cannot confirm it was
this branch — but every other 400 branch has a different length (167 / 251 /
259 / 317), so it is the only fitting candidate.

---

### A.11 Case 3 — `setup-email-nodot`

```
body: {"username":"admin","email":"admin@localhost","password":"12345678"}
status 400  bytes 251  content-type application/json  sql 5  handler_ran true
{"status":"400 Bad Request","error":"Error creating admin user: Validation error: Invalid email: domain must contain a dot",…}
```

State after: `users=0 owners=0 si_rows=0` (claim released, same as A.6).

---

### A.12 Case 4 — `setup-email-forbidden` (raw body > 256 bytes)

Command: the same request twice, once plain and once with
`-H 'Accept-Encoding: gzip'`.

```
step                  status  transferred  decoded  content-encoding
post-forbidden         400     259          259      (none)
post-forbidden-gzip    400     151          259      gzip
```

```
{"status":"400 Bad Request","error":"Error creating admin user: Validation error: Invalid email: contains forbidden characters",…}
```

**Compressed-vs-raw is explicit here:** raw 259 → transferred 151 with
`Content-Encoding: gzip`; the decoded copy (`.decoded.body`) is 259 bytes and
byte-identical to the uncompressed step's body.

---

### A.13 Case 5 — `setup-username-badchars` (raw body > 256 bytes)

```
step                  status  transferred  decoded  content-encoding
post-badchars          400     317          317      (none)
post-badchars-gzip     400     167          317      gzip
```

```
{"status":"400 Bad Request","error":"Error creating admin user: Invalid username: Username may only contain letters, digits, hyphens, underscores, and dots",…}
```

---

### A.14 Cases 6–8 — JSON body shape (`json-missing-field`, `json-wrong-type`, `json-syntax-error`)

| case | body sent | status | content-type | bytes | handler | sql |
|---|---|---|---|---|---|---|
| `setup-json-missing-field` | `{"username":"admin","password":"12345678"}` | 422 | `text/plain; charset=utf-8` | 99 | false | 0 |
| `setup-json-wrong-type` | `{"username":123,…}` | 422 | `text/plain; charset=utf-8` | 134 | false | 0 |
| `setup-json-syntax-error` | truncated JSON | 400 | `text/plain; charset=utf-8` | 89 | false | 0 |

Bodies verbatim in A.8. `handler_ran=false` and `sql=0` for all three: the
extractor rejects before the handler, and no database work happens.

---

### A.15 Cases 9–10 — `Content-Type` handling

| case | request | status | content-type | bytes |
|---|---|---|---|---|
| `setup-content-type-text` | valid JSON body, `Content-Type: text/plain` | 415 | `text/plain; charset=utf-8` | 54 |
| `setup-content-type-absent` | valid JSON body, `Content-Type` header removed (`-H 'Content-Type:'`) | 415 | `text/plain; charset=utf-8` | 54 |

Both bodies: `Expected request with `Content-Type: application/json``.
`handler_ran=false`, `sql=0`, state unchanged. Note the absent-header case is
genuinely absent — curl's default `application/x-www-form-urlencoded` is
explicitly stripped, otherwise curl would have sent one and the case would
measure something else.

---

### A.16 Case 11 — `GET /api/setup` and whether it consumed budget

**Hypothesis.** If the limiter is layered on the path's router (not on the POST
route), a `GET` — which the route does not serve and which therefore ends in
`405` — still consumes a register slot.

**Command:** one `GET /api/setup`, then five `POST /api/setup` (pw7 body), all
on one fresh 5/3600 instance.

**Observed:**

```
get-setup 405 len=0    sql=0 handler=False
post-1    400 len=167  sql=5 handler=True
post-2    400 len=167  sql=5 handler=True
post-3    400 len=167  sql=5 handler=True
post-4    400 len=167  sql=5 handler=True
post-5    429 len=53   sql=0 handler=False   retry_after=3600
```

**Answer: yes, the GET consumed one of the five slots.** Five requests passed
the limiter and the sixth was refused; if the GET had not counted, all five
POSTs would have been served. Consistent with the `405` being produced after the
middleware: the limiter runs before method routing, which is strictly earlier
than extraction.

Note the `405` response has **zero-length body** and no `Content-Type`.

---

### A.17 Case 12 — `rate-limit-6x`

See **A.1**; the essential numbers:

```
5 × 400 (167 bytes, sql=5, handler ran)  then  1 × 429 (53 bytes, retry_after=3600, sql=0, handler did not run)
```

---

### A.18 Case 13 — `shared-budget` (both directions)

**Hypothesis.** `/api/auth/register` and `/api/setup` draw on the *same*
`register` limiter, so exhausting either closes the other.

**Command:** five `POST /api/auth/register`, then one `POST /api/setup`; then a
fresh instance/DB and the reverse (five `/api/setup`, then one
`/api/auth/register`).

**Observed:**

```
register-1 201 len=140   (user created)
register-2 409 len=129   {"status":"409 Conflict","error":"Email is already registered",…}
register-3 409 len=129
register-4 409 len=129
register-5 409 len=129
setup-after-register 429 len=53 retry_after=3600 sql=0 handler=False

-- fresh instance, reverse direction --
setup-1..setup-5 400 len=167
register-after-setup 429 len=53 retry_after=3600 sql=0 handler=False
```

**Answer: one shared budget, both directions.** Note the limiter counts
requests that *fail* for other reasons — the four `409`s (email taken) still
consumed slots, because the limiter decides before the handler. The app log
confirms the register path was the one executing:

```
audit: 🛂 user registered event="auth.register" reason="created" …
audit: 🛂 register collision: email 're-reg@example.test' is already registered event="auth.register" reason="email_taken"
```

---

### A.19 Case 14 — `xff-ignored-untrusted`

See A.5 (counter-case). Six requests carrying five different `X-Forwarded-For`
values, trusted list empty:

```
400,400,400,400,400,429     # all in the single 127.0.0.1 bucket
```

The header is ignored when the TCP peer is not a trusted proxy.

---

### A.20 Case 16 — `terminal-201-then-403`

**Command:** valid setup twice, then `GET /api/auth/status`.

**Observed:**

```
valid-1 201 len=159  sql=22 handler=True
        {"id":"fb19bedd-23f1-4247-96f9-08a4fab8e50b","username":"admin","email":"admin@example.test","role":"owner","image":null,"is_external":false,"is_online":false}
valid-2 403 len=224  sql=1  handler=True
        {"status":"403 Forbidden","error":"System is already initialized. Use the admin panel to manage users.","message":"System is already initialized. Use the admin panel to manage users.","error_type":"SystemAlreadyInitialized"}
status  200 len=86   sql=2  handler=False
        {"initialized":true,"admin_count":1,"registration_allowed":true,"faces_enabled":false}

state_after  users=1 owners=1 si_rows=1 si_value="true" si_updated_by=fb19bedd
```

App log for the second call:

```
oxicloud::interfaces::api::handlers::auth_handler: Setup admin rejected: system already initialized (user: admin)
```

**Answer: 201 exactly once, then 403 forever; exactly one user, exactly one
row with role `owner`.** And the cross-check that closes A.6: the claim row's
`updated_by` is the created admin's UUID (`fb19bedd…`, the same id in the 201
body) — the placeholder `Uuid::nil()` was replaced by
`mark_system_initialized(real_user_id)`, not left as nil.

The second call issues **one** statement (the `is_system_initialized` SELECT) and
never attempts the claim: cost of a repeat call is one indexed SELECT.

---

### A.21 Case 17 — `concurrent-valid-2` and `concurrent-valid-8`

**Command:** `re_case_burst 2` / `re_case_burst 8` valid setup POSTs on a fresh
DB, release posture (5/3600).

**Observed** (`A`, 8-way; the 3 `429`s are the documented confound — with
`max=5`, requests 6..8 can be refused before ever reaching the claim):

```
429, 429, 201(159, sql=22), 403(224, sql=1), 403, 403, 403, 429
state_after users=1 owners=1 si_rows=1
```
(5 requests were served — 1 × `201` + 4 × `403` — and 3 were refused by the
release-posture limiter; max = 5.)

**Observed** (2-way): `403, 201` → one owner, one user.

**Unconfounded re-run** (posture `A-regwin15`, `register max=100`, so the
limiter cannot refuse anything):

```
N=8  (delivered run, statuses in step order):
     403,403,403,403,403,201,403,403     sql=1,1,1,1,1,22,1,1
N=16 (delivered run):
     403,403,403,403,403,403,201,403,403,403,403,403,403,403,403,403
     sql=1,1,1,1,1,1,22,1,1,1,1,1,1,1,1,1
state_after  users=1 owners=1 si_rows=1 si_value="true"
```
(the loaded run put the single `201` at position 6 and position 16 respectively —
see the determinism check below.)

**Answer: exactly one `201` and exactly one owner row in every sample (2, 8,
16 simultaneous).** The *winner* is not stable across runs — across the runs in
this session the same 8-way corpus case put the `201` on step 8, step 2, step 3
and step 1, and the delivered run's vector is
`201,429,403,403,429,403,429,403` (1 × `201`, 4 × `403`, 3 × `429`). — the
invariant is "exactly one", not "which one". The single `201` in each burst
issues 22 statements (user + drive + root folder + role grants + calendar…); the
`403` losers issue 1 statement, occasionally 2 (when the loser also attempted
the claim and saw 0 rows affected).

#### Determinism check — the delivered set against the loaded set

Reproducible from the repo alone:

```
$ python3 docs/re/scripts/diff-labels.py docs/re/evidence
A = A (A-max1win15.jsonl, A-regwin15.jsonl, A-rl3600.jsonl, A-xff.jsonl, A.jsonl)
B = A-loaded (A-loaded-max1win15.jsonl, A-loaded-regwin15.jsonl, A-loaded-rl3600.jsonl, A-loaded-xff.jsonl, A-loaded.jsonl)
same-outcomes concurrent-burst-rejects-16      statuses [400, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429]
same-outcomes concurrent-burst-rejects-8       statuses [400, 429, 429, 429, 429, 429, 429, 429]
same-outcomes concurrent-valid-16-unconfounded statuses [201, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403]
same-outcomes concurrent-valid-2               statuses [201, 403]
same-outcomes concurrent-valid-8               statuses [201, 403, 403, 403, 403, 429, 429, 429]
same-outcomes concurrent-valid-8-unconfounded  statuses [201, 403, 403, 403, 403, 403, 403, 403]
cases identical (field by field)      : 20
cases with same status multiset only  : 6
cases whose status multiset differs   : 0
```
`[ … the tool then prints, for each of the six `same-outcomes` cases, the
differing per-step fields — 275 lines in total … ]`

(exit status 0.) The six `same-outcomes` lines are the whole difference: **20 of
the 26 cases are identical field by field** between the two runs, the other six
are the cases with *simultaneous* requests, and every one of them keeps the same
status multiset. What varies is exactly:

| class | why it varies | how a cross-build diff must handle it |
|---|---|---|
| step **order** of a simultaneous burst | which sibling wins the claim / which one the limiter refuses is a race | compare the status multiset, not the index |
| per-step `app_log_lines` inside a burst | sibling requests' log windows overlap, so a sibling's marker can land in this step's window | ignore for bursts; reliable on sequential cases (A.5 uses it there) |
| the losing step's `app_sql_statement_count` (1 vs 2) | a claim-race loser either sees `is_system_initialized` already true (1 query) or attempts the claim and sees 0 rows (2 queries) | treat 1 and 2 as the same outcome on a 403 loser |
| `state_*.si_updated_by` | the admin UUID is minted per run | exclude |
| `response.x_request_id`, `wall_epoch_us`, `elapsed_ms`, `ttfb_ms`, `db_now*`, `pg_log_window.sha256/.lines` | fresh per request / clock / background churn | exclude |
| `Available disk space (N bytes)` inside `app_log_lines` | N is the sandbox's free disk at that moment | normalise to `(N bytes)` |
| UUIDs minted by the run inside `app_log_lines` (user, personal drive, calendar, address book) | fresh per run | normalise to `<uuid>` |

Everything else — `status`, `content_type`, `content_encoding`, `content_length`,
`retry_after`, `body_len`, `body_file_bytes`, `body_decoded_len`,
`body_decoded_source`, `handler_ran`, `app_sql_statement_summaries`,
`resolved_client_ip`, `request`, and `state_*.{users,owners,si_rows,si_value}` —
is stable across runs and is what the differential must be built on.
`diff-labels.py` applies the last two normalisations itself (they are the only
noise removals it makes); the same exclusion list is in the `lib-re.sh` header so
the next agent does not chase them.



(exit status 0; the tool prints the differing per-step fields for the six
`same-outcomes` cases, which is where the exclusion list below comes from.)

**20 of the 26 cases are identical field by field between the two runs.** The six
that are not are all cases with *simultaneous* requests, and every one of them
keeps the same status multiset. What differs is exactly:

| class | why it varies | how a cross-build diff must handle it |
|---|---|---|
| step **order** of a simultaneous burst | which sibling wins the claim / which one the limiter refuses is a race | compare the status multiset, not the index |
| per-step `app_log_lines` inside a burst | the sibling requests' windows overlap, so a marker line from a sibling can land in this step's window | ignore for bursts; it is reliable on sequential cases (A.5 uses it there) |
| the losing step's `app_sql_statement_count` (1 vs 2) | a claim-race loser either sees `is_system_initialized` already true (1 query) or attempts the claim and sees 0 rows (2 queries) | compare as ∈ {1,2} on 403 losers |
| `state_*.si_updated_by` | the admin's UUID is minted per run | exclude |
| `response.x_request_id`, `wall_epoch_us`, `elapsed_ms`, `ttfb_ms`, `db_now*`, `pg_log_window.sha256/.lines` | fresh per request / clock / background churn | exclude |
| `Available disk space (N bytes)` inside an `app_log_lines` entry | N is the sandbox's free disk at that moment | normalise to `(N bytes)` |
| UUIDs minted by the run inside `app_log_lines` (user, personal drive, calendar, address book) | fresh per run | normalise to `<uuid>` |

Everything else — `status`, `content_type`, `content_encoding`, `content_length`,
`retry_after`, `body_len`, `body_file_bytes`, `body_decoded_len`,
`body_decoded_source`, `handler_ran`, `app_sql_statement_summaries`,
`resolved_client_ip`, `request`, and `state_*.{users,owners,si_rows,si_value}` —
is byte-stable across runs, and is what the differential must be built on.
`diff-labels.py` applies the last two normalisations itself (they are the only
"noise" removals it makes) and prints the classes above; the same exclusion list
is stated in the `lib-re.sh` header so the next agent does not chase them.

*Caveat on `limiter_ip` for burst steps:* a burst step's log window overlaps its
siblings' (the requests really are simultaneous), so on a *served* burst step
`limiter_ip` can be a neighbour's `Rate limit exceeded` line rather than this
step's. It is reliable on single-request cases, which is where A.5 uses it; the
bucket evidence in A.5 comes from a sequential case for that reason.

---

### A.22 Experiments — case 15 (`xff-honoured-trusted`), 18, 18b, 19, 20 and the compression bracket

Covered above: A.5 (case 15), A.4 (case 18 + 18b), A.2 (case 19), A.3 (case 20).

#### Compression bracket (posture `A-regwin15`, `register 100/3600 s`)

Command: seven POSTs, every one sent with `-H 'Accept-Encoding: gzip'`, chosen so
each 400 branch appears once:

```
step            raw (decoded)  transferred  content-encoding
ct-415               54            54       (none)
syntax-400           89            89       (none)
pw7-400             167           167       (none)
bademail-400        219           219       (none)     <-- the candidate branch
nodot-400           251           251       (none)
forbidden-400       259           151       gzip
badchars-400        317           167       gzip
```

**Measured bracket: not compressed at 251 bytes, compressed at 259 bytes.**
Predicted from HEAD: `SizeAbove::new(256)`. This is the measurement that
retires the "219 was a compressed size" theory for A.10.

---

### A.23 What I could not measure (and why)

1. **The operator's actual 219-byte response.** I never contacted the production
   instance with a POST (it is an irreversible one-shot initialisation oracle:
   one valid payload creates the first admin permanently). The 219-byte branch is
   therefore a **candidate by exact length**, not an identified response.
2. **A proven lost update in the rate limiter (Q4).** Two bursts (8 and 16
   simultaneous, `max=1`) served exactly one request each. Proving the race needs
   a wider burst, a deterministic interleaving instrument, or instrumentation
   inside the moka cache; none was used, so the answer stays "not observed".
3. **`strace`/`gdb` corroboration of the 429 path.** Not needed: the app's own
   request-scoped `sqlx` lines plus the DB statement log already show zero DB
   I/O, and a syscall trace could not attribute work to a request id anyway.
   Skipped deliberately (and it would have distorted the `elapsed_ms` numbers).
4. **Production's runtime config.** `/etc/oxicloud/oxicloud.env` is
   `root:oxicloud 0640` and unreadable from the deploying account; I did not
   escalate. The limiter/trust numbers used here are the *shipped defaults* as
   read back from the binary's own startup line, not production's file.
5. **Cross-major PG behaviour.** The sandbox runs PostgreSQL 18.2 (the repo's
   compose pin) while production runs 17.11. Every measured statement is
   plain; no version-specific feature was exercised, but a difference in
   planner/notice output between those majors is not excluded by these runs.
6. **Whether a crash between claim-write and claim-release leaves the
   placeholder row.** The sequence is on two different pooled connections
   (PIDs 34430 / 34433), so the window exists in principle; inducing it is out of
   scope here and is stated as an unmeasured risk, not a finding.

---

# B — Builds, provenance and the causal gate (C0, C1L, C1P, C1, B)

Read with `docs/re/differential-results.md` §1 (the provenance table this section
fills in) and `docs/re/experiment-matrix.md` §7 (the gate). **The quiet `A` pass
delivered in `docs/re/evidence/A*.jsonl` is the baseline for every number here**;
`A-loaded*` is only the determinism cross-check (§A.0.1).

## B.0 Labels, profile and the one declared normalization

| label | binary | what it is | evidence files |
|---|---|---|---|
| `A` | released `oxicloud` 0.10.0 (musl, release, `bundled-assets`, `x86-64-v2`) | baseline, identity `9d364fdd…e080` (§A) | `A*.jsonl` |
| `c0` | `c0-head-unpatched` | HEAD `389e161b`, **no patches** — the drift control | `c0*.jsonl` |
| `c1l` | `c1l-limiter-only` | HEAD + `0001` (limiter window / atomicity / Retry-After) | `c1l*.jsonl` |
| `c1p` | `c1p-proxy-only` | HEAD + `0003` (rightmost untrusted XFF hop) | `c1p*.jsonl` |
| `c1` | `c1-patched` | HEAD + `0001..0004` (the deliverable tree) | `c1*.jsonl` |
| `b` | `b-v010-source` | `git worktree` at tag `v0.10.0` (`496d25b8`) | `b*.jsonl` |

Every build: **dev profile, default features, host `x86_64-unknown-linux-gnu`,
`-C target-cpu=native` (from the tree's `.cargo/config.toml`)**, with
`CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0
RUSTFLAGS="-C debuginfo=0"` and its own `CARGO_TARGET_DIR`. `A` is the only
non-dev build; **its deltas (profile, libc, CPU baseline, asset delivery) can
only ever be a normalization item on the `A ↔ rest` edge** and are never used to
explain a `c0 ↔ c1` difference.

The limiter is an in-process moka cache, so **every posture is one process**:
release posture (no `OXICLOUD_RATE_LIMIT_*`) for the corpus, and the three
override postures (`-rl3600`, `-regwin15`, `-max1win15`, `-xff`) only inside
`run-rate-limit-experiments.sh`. `OXICLOUD_RATE_LIMIT_SETUP_*` exists **only in
the patched tree** (patch `0002` adds it); it is never set here, so the
shared-vs-separate budget contrast is visible through the register budget alone.

## B.1 Binaries (build order, sha256, size)

| # | label | tree | `rev-parse` | tree `git diff` sha256 | binary sha256 | size |
|---|---|---|---|---|---|---|
| 1 | `c0` | `/data/oxi` clean | `389e161bc6b94d720f50505e4e9c8219e56124d3` | `e3b0c442…b855` (empty diff) | `12959e3dd464352d7be4f60fb4d6509ccc711a24eccb129d069e9cc298422b35` | 117 662 288 |
| 2 | `c1l` | HEAD + `0001`¹ + errors.rs² | `389e161b…` | `12f33782a0bcbdea77481ce0a0ac093ff590ea5c1b13cdf60d407814ed617553` | `5f3a5b9fc3d709ba54dd255397e25059ce7223abe8585f916be5dc450ef85083` | 117 743 040 |
| 3 | `c1p` | HEAD + `0003` + errors.rs² | `389e161b…` | `5fccb4a8ffec4025c42212cfccae711a1f0bb10e9958e06449cf8e2d7b99beaf` | `1695ac61548988b49cc3638161bced8f373334b44e60e60b915f0f71cf1f03cb` | 117 627 856 |
| 4 | `c1` | HEAD + `0001..0004` + errors.rs² | `389e161b…` | `f6d9897b43fb103c7f29e2ff688a81c8ac32ab0ca3da9d25605fb14b9cec8a31` | `4666faa8759d208bd8ebd002fbe2574c201b309f416c666a72e4311738d20ec3` | 117 701 088 |
| 5 | `b` | `/data/oxi-v010` `v0.10.0` | `496d25b89f672ffe3395042a0584f24d490ed761` | `e3b0c442…b855` (empty diff) | `b-v010-source` (see `binaries.txt`) | 117 6xx xxx |

¹ **`0001` as delivered does not compile** — `RateLimiter::retry_after_for` called
`remaining.div_ceil(Duration::from_secs(1))`, and `std::time::Duration::div_ceil`
does not exist in stable Rust: `error[E0599]: no method named div_ceil found for
struct std::time::Duration` on rustc 1.99.0, reproduced in a 4-line standalone
program. It is production code, so it blocked C1L *and* C1 from building at all.
The fix is the faithful stable ceil-to-seconds, one line:

```rust
remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0)
```

Copies: delivered `d4e00aa9…` (defective) → my corrected
`46b4cfab…` (used for C1L) → REPatches' republished `751ca5a0…` (used for C1/C1P).
**All three carry the identical code**: the README's added-non-comment-line
`digest()` yields `f6ffe6a71f13b1c5789507db710416db8f5b38b572442a60bad398689450281d`
(223 lines) for both the republished file and my corrected copy, so the arms are
interchangeable and C1L needed no rebuild (comment prose differs only).

² Each arm tree also carries `src/interfaces/errors.rs` from the delivered
working tree (sha256 `b3dd847c21a83cbf265633327b9ee929eea65f1cfe7103476448f83620a44e35`).
That file's `#[cfg(test)]` byte-length oracle is part of **no patch file** — it
exists only as an uncommitted working-tree change — so "arm tree = the patches
above **+ this file**" is the exact build input, stated in `binaries.txt`. It is
test-only (`cfg(test)`), so it cannot change any binary.

## B.2 Disk trace (the binding constraint)

| step | `df /data` free before | after | note |
|---|---|---|---|
| `c0` first attempt | 1.4 G | 979 M | abandoned: the pre-existing `/data/oxi-target` had been built with different flags, so cargo recompiled all 450 dependencies on top of the old 2.4 G artifact set |
| `cargo clean` | — | 3.7 G | removed 9 994 files / 2.7 GiB |
| `c0` | 3.7 G | 1.5 G | full from-scratch dev build, `Finished … in 5m 26s` |
| `c1l` | 1.7 G | 1.3 G | incremental (crate only), `3m …` |
| `c1p` | 928 M | 551 M | incremental |
| `c1` | 551 M | 437 M | incremental |
| OpenAPI regeneration | 951 M | 595 M | one attempt failed on `No space left on device` before `cargo clean -p oxicloud`; retried clean (see B.5) |
| `cargo clean` (arm target dir) | — | 3.2 G | freed before B, so two full artifact sets never coexist |
| `b` | 3.2 G | (see `binaries.txt`) | fresh target dir `CARGO_TARGET_DIR=/data/oxi-target-v010` |

No file outside `/data/oxi*`, `/data/re*` and `/tmp` was deleted; `/data/home/**`
and `/data/oxi`'s HEAD clone were never touched beyond the arm patch application.

## B.3 GREEN side — each arm's own tree (run BEFORE any graft)

Command (`docs/re/scripts/lib-re.sh` unchanged; the test runner is
`/data/re/re-test.sh`, which records the tree sha and `df` in the log header):

```sh
cargo test --lib -- rate_limit trusted_proxy error_response [setup_openapi_contract]
```

| arm | tree sha | tests | result | log |
|---|---|---|---|---|
| `c1l` | `12f33782…` | 13 | **ok. 13 passed; 0 failed** | `c1l-green-….test.log` |
| `c1p` | `5fccb4a8…` | 12 | **ok. 12 passed; 0 failed** | `c1p-green-….test.log` |
| `c1` (pre-regeneration) | `f6d9897b…` | 19 | 18 passed, **1 FAILED**: `setup_openapi_contract_tests::committed_spec_declares_the_setup_contract` — the 0004 artifact test, RED *by design* until the spec is regenerated (B.5) | `c1-green-….test.log` |
| `c1` (post-regeneration) | `f6d9897b…` + regenerated spec | 2 | **ok. 2 passed; 0 failed** → the same test flips GREEN | `c1-post-regen-setup_openapi_contract.test.log` |

The tests that matter, verbatim per arm:

* `c1l`: `rate_limit::rejected_requests_do_not_extend_the_window` ok,
  `rate_limit::retry_after_header_shrinks_as_the_window_elapses` ok,
  `rate_limit::two_routes_share_one_budget` ok — the **sharing** assertion, which
  is correct for an arm that does not carry `0002`;
  `trusted_proxy` here is HEAD's own module (no `0003`), 7 tests ok;
  `errors::error_response_length_oracle` ok.
* `c1p`: `trusted_proxy::client_ip_resolves_the_rightmost_untrusted_hop` ok,
  `trusted_proxy::spoofed_prefix_cannot_choose_the_rate_limit_key` ok, plus the
  5 pre-existing CIDR tests and `errors::*` ok.
* `c1`: everything above **plus**
  `rate_limit::setup_and_register_have_separate_budgets` ok (the *repointed*
  test that `0002` owns), `config::setup_rate_limit_budget_tests::*` (2) ok,
  `auth_handler::setup_openapi_contract_tests::setup_admin_attribute_declares_the_contract` ok.

**Atomicity: one deterministic schedule of the code shape, one flaky detector, and an HTTP level that can see neither.**

* **Deterministic (a schedule of the shape — not the production method).** A **hand-interleaved two-request schedule** — A
  reads, B reads, A writes, B writes — serves **2** on a budget of 1 in the
  original shape at *every* opt-level, and `Ok(1)` + `Err(())` in the patched one:
  no threads, no timing, no ambient load. This is what pins `0001`'s atomicity
  half (`docs/re/patches/README.md`, "Atomicity guard: standalone runs").
* **Flaky detector (never a RED column, at any level).**
  `concurrent_burst_admits_exactly_one_request` is a burst detector of the same
  defect: measured overshoot fractions for the original shape span **0/10 … 8/10**
  with **ambient load and arrival spread dominating**, and the ordering by
  opt-level *reverses* between otherwise identical runs (so the opt-level
  hypothesis is falsified). My own data points: **0/3** when the test ran
  alongside its four sibling tests in the same binary, **9/10 fail** when the same
  single test was run in isolation (`--exact`, 10 runs,
  `/data/re/logs/guard-v010.run{1..10}.log`), on the same tree and profile
  (`[profile.dev] opt-level = 1`, moka `=0.12.15`); the patched shape measured
  0/10 in all six patched configurations of each run (12 across both runs). Report the range and the load, never one number.
* **Server level (what an operator sees).** 64 simultaneous HTTP requests at
  `max=1` served exactly 1 in 5/5 trials on `c0` *and* `c1`
  (`c0-burst.json`, `c1-burst.json`), because the requests are spread over ≈0.4 s
  of HTTP handling — the same reason the flaky detector is flaky. There is no
  HTTP-level RED for this defect on either build.

## B.4 0004's artifact: regenerating `resources/gen/openapi.json`

```sh
cd /data/oxi && OXICLOUD_VERSION=0.10.0 CARGO_TARGET_DIR=/data/oxi-target \
  CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0 \
  RUSTFLAGS="-C debuginfo=0" cargo run --features dev_tools --bin generate-openapi
```

`OXICLOUD_VERSION=0.10.0` is required, not cosmetic: this clone is shallow, so
`git describe --tags` cannot reach `v0.10.0` from HEAD and `build.rs` falls back
to the bare commit SHA, which `src/common/spec_tag.rs::require_release_tag_or_exit`
**refuses** to stamp (that is the `"version": "d97bedda"` incident it exists to
prevent — it printed *"Nothing was written"*). On a full checkout build.rs would
compute `v0.10.0-210-g389e161b(-dirty)` and `strip_describe_suffix()` reduces
that to the bare tag `0.10.0` — exactly what the override emits.

| | HEAD's committed spec | regenerated |
|---|---|---|
| sha256 | `1a8fddbc348837e141a833c32db048b14da1049267cb4de32eadbebb7dce5b24` | `40bb734ea4523e521c6909d94e4883aca9ce8623c684b2991fa58677015e21d7` |
| size | 794 900 B | (see `test-runs.txt`) |
| `info.version` | `0.10.0` | `0.10.0` (unchanged — the stamp is not the change) |
| `…/post/responses` | `['201','403','503']` | `['201','400','403','429','503']` |

The structural diff is exactly the intended one: the 400 and 429 response objects
are added and the operation description is rewritten to list where each status
comes from. Nothing else in the document changes. The regenerated file is
delivered as `docs/re/evidence/gen-openapi.c1.json` (HEAD's copy beside it as
`gen-openapi.head.json`).

**Follow-up the maintainer must run** (deliberately NOT done here):
`cd frontend && npm run gen:openapi` (= the SDK half of `just gen-fe-api`) to
regenerate `frontend/src/lib/api/generated/**`. Reason: this wave's change set is
the four patches plus the one regenerated Rust-side artifact; keeping the
generated TypeScript SDK out of it keeps the diff reviewable and avoids mixing a
package-manager-driven regeneration into the RE evidence.

---

# C — Causal gate (RED/GREEN) and the differential verdicts

## C.0 The gate

**Reproducing `docs/re/evidence/differential.json`.** The artifact is generated;
the invocation of record is the script's *default* (it loads `A-loaded` as its own
label and then excludes it, so the quiet `A` pass stays the baseline):

```sh
python3 docs/re/scripts/diff-builds.py docs/re/evidence docs/re/evidence/differential.json
python3 docs/re/scripts/diff-builds.py --check docs/re/evidence/differential.json   # staleness guard
```

`--check` regenerates with those defaults and compares byte-for-byte, exiting
non-zero and printing a unified diff if the committed artifact is stale (exit 0
identical, 1 different, 2 unreadable). Both invocations are also in the module
docstring; there is no `B` label — the `v0.10.0` source build is `b`.

* **GREEN side** = each arm's *own* tree (`cargo test --lib -- rate_limit
  trusted_proxy error_response [setup_openapi_contract]`), run **before** any
  graft, so a defect in the arm's own test code cannot be confused with a graft
  that fails to compile on v0.10.0.
* **RED side** = `docs/re/scripts/graft-tests.sh <arm> /data/oxi-v010 <files>`:
  the module that starts at each file's `// ─── Tests ───…` banner is transplanted
  from the arm onto the **v0.10.0 worktree** (`496d25b8`, clean, verified empty
  `git status`), leaving the pre-patch production code untouched, then the same
  command runs there.
  * `rate_limit.rs` / `errors.rs`: the banner-to-EOF module is **appended**
    (v0.10.0 has no test module in either file).
  * `trusted_proxy.rs`: the target has its own module, so the arm's module
    **replaces** it from the banner to EOF (`72089037…` → `30512327…`).
  * `auth_handler.rs`: no banner; the `#[cfg(test)] mod setup_openapi_contract_tests`
    block is appended (v0.10.0 has no test module in that file).
  * **Every grafted module COMPILED on v0.10.0** — no "RED became did-not-compile"
    anywhere, so every RED below is a real assertion failure. (The script's
    symbol heuristic is advisory; the compilation is the authority.)
* All three RED runs used the *arm's own* test state, so the `0002` repoint is
  visible: `c1l` carries `two_routes_share_one_budget`, `c1` carries
  `setup_and_register_have_separate_budgets`.

## C.1 RED/GREEN matrix (per arm, per test)

| test | C1L GREEN | C1P GREEN | C1 GREEN | RED on v0.10.0 (arm) | patch |
|---|---|---|---|---|---|
| `rate_limit::rejected_requests_do_not_extend_the_window` | ok | – | ok | **FAILED** (c1l, c1) | 0001 |
| `rate_limit::retry_after_header_shrinks_as_the_window_elapses` | ok | – | ok | **FAILED** (c1l, c1) | 0001 |
| `rate_limit::two_routes_share_one_budget` (sharing guard) | ok | – | – | ok — guard, not evidence | 0001 |
| `rate_limit::setup_and_register_have_separate_budgets` (repoint) | – | – | ok | ok — passes on both trees (it builds the two instances it asserts about) | 0002 |
| `config::setup_rate_limit_budget_tests::*` (2) | – | – | ok | – | 0002 |
| `trusted_proxy::client_ip_resolves_the_rightmost_untrusted_hop` | – | ok | ok | **FAILED** (c1p, c1) | 0003 |
| `trusted_proxy::spoofed_prefix_cannot_choose_the_rate_limit_key` | – | ok | ok | **FAILED** (c1p, c1) | 0003 |
| `trusted_proxy::{ipv4_cidr_*,parse_cidr_*,ipv4_mapped_matches_v4_cidr,ipv6_cidr_loopback}` (7) | ok | ok | ok | ok (v0.10.0's own module) | — |
| `errors::error_response_length_oracle` | ok | ok | ok | ok (oracle on both wire forms) | oracle |
| `auth_handler::setup_openapi_contract_tests::committed_spec_declares_the_setup_contract` | – | – | ok (after regeneration) | **FAILED** (c1) | 0004 |
| `auth_handler::…::setup_admin_attribute_declares_the_contract` | – | – | ok | **FAILED** (c1) | 0004 |
| totals | 13 passed / 0 failed | 12 / 0 | 19 / 0 (post-regen) | 11 passed / **6 failed** (c1); 11 / **2** (c1l); 10 / **2** (c1p) | |

Verbatim RED failure texts (v0.10.0, `red-c1.test.log`, `red-c1l.test.log`):

```
---- interfaces::middleware::rate_limit::tests::rejected_requests_do_not_extend_the_window stdout ----
panicked at src/interfaces/middleware/rate_limit.rs:259:9:
assertion `left == right` failed: t≈5 s is one 4 s window past the FIRST request, so the client is let back in;
the pre-patch code kept it out until one window after the rejected attempt at t≈2 s (t≈6 s)
  left: Err(())   right: Ok(1)

---- interfaces::middleware::rate_limit::tests::retry_after_header_shrinks_as_the_window_elapses stdout ----
panicked at src/interfaces/middleware/rate_limit.rs:329:9:
≈3 s of the 6 s window are gone, so the wait must shrink: 6 s vs 6 s (equal values mean the header
is reporting the configured window again)

---- interfaces::middleware::trusted_proxy::tests::client_ip_resolves_the_rightmost_untrusted_hop stdout ----
panicked at src/interfaces/middleware/trusted_proxy.rs:433:9:
assertion `left == right` failed
  left: "6.6.6.6"      right: "203.0.113.9"

---- interfaces::middleware::trusted_proxy::tests::spoofed_prefix_cannot_choose_the_rate_limit_key stdout ----
panicked at src/interfaces/middleware/trusted_proxy.rs:627:9:
assertion `left == right` failed: the appended hop is the client

---- interfaces::api::handlers::auth_handler::setup_openapi_contract_tests::committed_spec_declares_the_setup_contract stdout ----
panicked at src/interfaces/api/handlers/auth_handler.rs:2161:13:
resources/gen/openapi.json: POST /api/setup must declare 400; it declares ["201", "403", "503"]

---- interfaces::api::handlers::auth_handler::setup_openapi_contract_tests::setup_admin_attribute_declares_the_contract stdout ----
panicked at src/interfaces/api/handlers/auth_handler.rs:2161:13:
ApiDoc::openapi(): POST /api/setup must declare 400; it declares ["201", "403", "503"]
```

**Atomicity — classification (final).** The atomicity change is **justified by an argument, not by a gate test** —
**the source shape plus a deterministic schedule of it** (the harness splits the
old method into `read`/`write_and_decide` and interleaves them by hand: 2 served
at budget 1 in the original shape at every opt-level, `Ok(1)`+`Err(())` in the
patched one — file/thread-free; the production method itself is a sequential
composition of those halves and is **not** executed under controlled
interleaving here) — instruments and both runs in
`docs/re/patches/README.md`, "Atomicity guard: standalone runs".
`concurrent_burst_admits_exactly_one_request` is a **flaky detector of that same
defect and is never in a RED column at any level**: measured overshoot fractions
for the original shape span 0/10 … 8/10, dominated by ambient load and arrival
spread, with the opt-level ordering reversing between identical runs. My data
points on this host (`[profile.dev] opt-level = 1`, moka `=0.12.15`): **0/3** when
the test ran with its four sibling tests in the same binary
(`/data/re/logs/red-guard-new.run{1,2,3}.log`, module grafted from
`/data/re/arm-src/c1l-new`), **9/10 fail** when the same test ran alone
(`--exact`, `/data/re/logs/guard-v010.run{1..10}.log`), and the patched shape
0/10. At **server level** there is no RED at all: 64 simultaneous HTTP requests
at `max=1` served exactly 1 in 5/5 trials on `c0` and on `c1`
(`c0-burst.json`, `c1-burst.json`), because the requests spread over ≈0.4 s.
The `c1l` GREEN side is measured: 3/3 runs `ok. 5 passed; 0 failed`
(`/data/re/logs/green-c1l-new.run{1,2,3}.log`), and so is the **`c1` leg** after
the aborted run was re-done: 3/3 runs `ok. 8 passed; 0 failed`
(`/data/re/logs/green-c1-new.run{1,2,3}.log`), with
`concurrent_burst_admits_exactly_one_request`, `setup_and_register_have_separate_budgets`,
both `0001` window tests and `config::per_caller_rate_limit_defaults…` all `ok` —
i.e. `0002` applied on top of `0001` exactly as documented.

**Correction to an earlier note in this section (kept visible):** I first wrote
that the republished `0002` "no longer applies on top of the republished `0001`"
and offered `git apply --check: patch failed: rate_limit.rs:526` as proof. **That
claim was wrong** — the check was run against a **clean HEAD tree**, where `0002`
has always failed by design (its `rate_limit.rs` hunk is a context diff on
`0001`'s test module, and `README` §"Order and dependencies" says so:
`HEAD + 0002` alone is deliberately not a valid arm). Applied in the documented
order the pair layers fine: `HEAD + 0001 + 0002 + 0003 + 0004` applied cleanly in
one step (tree diff sha `73dc9bd4…`). No defect; no blocker; the only thing that
was ever missing for the `c1` leg was the run itself.

**Why my two measurements differ (invocation, not profile).** The same single
test, same tree, same profile, gave **0/3 passes-of-the-invariant when run with
its four sibling tests in the same binary** and **9/10 violations when run alone**
(`--exact`). `cargo test` runs selected tests in parallel, so the 64
barrier-released threads shared the cores with four siblings; the arrival spread
widened and the first `insert` landed before the next `read` — the same mechanism
that makes the HTTP-level bursts look atomic (§C.5). Ambient load and arrival
spread dominate this detector; the record is the *range*, and the deterministic
hand-interleaved schedule is the proof.

Checks done, per the three named in the hand-off: **(1)** the module is the
current one — re-grafted after the test landed, snapshot `ac735bb0…` from
`HEAD+republished-0001+errors.rs` (tree diff `2256cb5c…`), target file after the
graft `c1259e31…`; **(2)** the worktree's lockfile pins `moka = 0.12.15`,
identical to HEAD's; **(3)** the arguments are the ones the harness describes —
`RateLimiter::new(1, 15, 128)`, `THREADS = 64`, one `std::sync::Barrier`, one key
`"burst"`. None of the three explains the spread; the invocation does (above).
The patch pair in force while these runs were taken was `0001 = 27bac478…` /
`0002 = 9da0aa45…`; the current row is `0001 = ba230d63…` / `0002 = 9da0aa45…`
(use both from the same row — the README says so), and re-snapshotting the arm
source for the current `0001` is a re-run of `/data/re/phase-e.sh` plus
`/data/re/phase-f.sh`.

## C.2 The headline row — `setup-pw8-bademail`, one request, six builds

```
POST /api/setup  {"username":"admin","email":"admin","password":"12345678"}
build status content-type      bytes  transferred  coding
A     400    application/json  219    219          identity
c0    400    application/json  218    218          identity
c1l   400    application/json  218    218          identity
c1p   400    application/json  218    218          identity
c1    400    application/json  218    218          identity
b     400    application/json  219    219          identity
```

`A` and `B` agree on 219; every HEAD-based build is 218 — one byte,
`ErrorKind::as_str()` → `wire_name()` (`"Invalid Input"` → `"InvalidInput"`)
from the 210 commits between `v0.10.0` and HEAD. **The length alone does not
identify the branch** (`docs/re/evidence/extractor-length-bound.md` reaches a
219-byte `400 text/plain` through the extractor), so `status` and
`content-type` are part of the row.

## C.3 Verdict table — one row per case, per build

**The six verdict columns of every row are generated mechanically** from
`docs/re/evidence/differential.json` (`cases[].claims` first, else
`cases[].vs_A`), by `docs/re/scripts/diff-builds.py` + `docs/re/scripts/gen-section-c.py`, the generator that emits
this section: no cell is typed by hand, and the column order is fixed by one
list, so a row cannot drift from its header. `b` is the `v0.10.0` source build
(there is no separate `B` label); `match` on the `A` column means "this is the
baseline". `vs_c0` (the causal edge) is deliberately NOT a column — it is
quoted per case where it matters.

| case | A | c0 | c1l | c1p | c1 | b | patch | proposed verdict + code path |
|---|---|---|---|---|---|---|---|---|
| `compression-bracket` | match | explained-drift | explained-drift | explained-drift | unexplained | explained-drift | — | v0.10.0 renders ErrorKind via as_str() ("Invalid Input", 13 B); HEAD renders it via wire_name() ("InvalidInput", 12 B). One byte, every DomainError 4xx JSON body, introduced by the 210 commits between tag v0.10.0 and HE… |
| `concurrent-burst-rejects-16` | match | match | match | match | explained-drift | match | 0002 | same 0002 posture effect as the 8-way burst on c1: 5 served (the setup default), 11 rejected |
| `concurrent-burst-rejects-8` | match | match | match | match | explained-drift | match | 0002 | 0002 moved /api/setup onto its own budget, so this register-only posture pins nothing: 5 of 8 are served (the setup default) and 3 are 429 with Retry-After 3600. With BOTH budgets pinned to max=1 the same build serves e… |
| `concurrent-valid-16-unconfounded` | match | match | match | match | match | match | — | {"unexplained": [{"field": "status_multiset", "a": [201, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403], "b": [201, 403, 403, 403, 403, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429]}… |
| `concurrent-valid-2` | match | match | match | match | match | match | — | unchanged: identical on every non-normalized field |
| `concurrent-valid-8` | match | match | match | match | match | match | — | unchanged: identical on every non-normalized field |
| `concurrent-valid-8-unconfounded` | match | match | match | match | match | match | — | {"unexplained": [{"field": "status_multiset", "a": [201, 403, 403, 403, 403, 403, 403, 403], "b": [201, 403, 403, 403, 403, 429, 429, 429]}], "informational": []} |
| `rate-limit-6x` | match | explained-drift | explained-drift | explained-drift | explained-drift | explained-drift | 0001 | v0.10.0 renders ErrorKind via as_str() ("Invalid Input", 13 B); HEAD renders it via wire_name() ("InvalidInput", 12 B). One byte, every DomainError 4xx JSON body, introduced by the 210 commits between tag v0.10.0 and HE… |
| `retry-after-constant` | match | match | fix-confirmed | match | fix-confirmed | match | 0001 | H3/0001: Retry-After reports the time left in the window (~3580 s after 20 s), not the configured constant |
| `setup-content-type-absent` | match | match | match | match | match | match | — | unchanged: identical on every non-normalized field |
| `setup-content-type-text` | match | match | match | match | match | match | — | unchanged: identical on every non-normalized field |
| `setup-email-forbidden` | match | explained-drift | explained-drift | explained-drift | explained-drift | match | — | v0.10.0 renders ErrorKind via as_str() ("Invalid Input", 13 B); HEAD renders it via wire_name() ("InvalidInput", 12 B). One byte, every DomainError 4xx JSON body, introduced by the 210 commits between tag v0.10.0 and HE… |
| `setup-email-nodot` | match | explained-drift | explained-drift | explained-drift | explained-drift | explained-drift | — | v0.10.0 renders ErrorKind via as_str() ("Invalid Input", 13 B); HEAD renders it via wire_name() ("InvalidInput", 12 B). One byte, every DomainError 4xx JSON body, introduced by the 210 commits between tag v0.10.0 and HE… |
| `setup-get` | match | match | match | match | match | match | — | v0.10.0 renders ErrorKind via as_str() ("Invalid Input", 13 B); HEAD renders it via wire_name() ("InvalidInput", 12 B). One byte, every DomainError 4xx JSON body, introduced by the 210 commits between tag v0.10.0 and HE… |
| `setup-json-missing-field` | match | match | match | match | match | match | — | unchanged: identical on every non-normalized field |
| `setup-json-syntax-error` | match | match | match | match | match | match | — | unchanged: identical on every non-normalized field |
| `setup-json-wrong-type` | match | match | match | match | match | match | — | unchanged: identical on every non-normalized field |
| `setup-pw7` | match | explained-drift | explained-drift | explained-drift | explained-drift | match | — | v0.10.0 renders ErrorKind via as_str() ("Invalid Input", 13 B); HEAD renders it via wire_name() ("InvalidInput", 12 B). One byte, every DomainError 4xx JSON body, introduced by the 210 commits between tag v0.10.0 and HE… |
| `setup-pw8-bademail` | match | explained-drift | explained-drift | explained-drift | explained-drift | match | — | H12: the same request measures 218 raw bytes on HEAD's wire_name() spelling ("InvalidInput") |
| `setup-username-badchars` | match | explained-drift | explained-drift | explained-drift | explained-drift | explained-drift | — | v0.10.0 renders ErrorKind via as_str() ("Invalid Input", 13 B); HEAD renders it via wire_name() ("InvalidInput", 12 B). One byte, every DomainError 4xx JSON body, introduced by the 210 commits between tag v0.10.0 and HE… |
| `shared-budget` | match | match | match | match | fix-confirmed | match | 0002 | H5/0002: /api/setup has its own budget, so five register calls no longer lock setup out (setup-after-register is not 429) |
| `terminal-201-then-403` | match | explained-drift | explained-drift | explained-drift | explained-drift | match | — | unrelated route retired between v0.10.0 and HEAD: `GET /api/auth/status` answers 401 (Token not provided). Named code path: the signal it carried moved to `config.initialized` — src/interfaces/api/handlers/auth_handler.… |
| `ttl-refresh` | match | match | fix-confirmed | match | explained-drift | match | 0001 | H2/0001: the window closes 15 s after the key's first request, so t21-req4 is served (400) and re-opens a window that t26-req5 fills (429, Retry-After 10 = remaining) |
| `ttl-refresh-control` | match | explained-drift | explained-drift | explained-drift | explained-drift | explained-drift | — | v0.10.0 renders ErrorKind via as_str() ("Invalid Input", 13 B); HEAD renders it via wire_name() ("InvalidInput", 12 B). One byte, every DomainError 4xx JSON body, introduced by the 210 commits between tag v0.10.0 and HE… |
| `xff-honoured-trusted` | match | match | explained-drift | fix-confirmed | fix-confirmed | match | 0003 | H1/0003: only the RIGHTMOST untrusted hop keys the bucket, so the two spoofed chains share one bucket: 4 pass, the 5th passes and the 6th is rejected, while the victim-shaped value burns its own key |
| `xff-ignored-untrusted` | match | explained-drift | explained-drift | explained-drift | explained-drift | explained-drift | 0003 | v0.10.0 renders ErrorKind via as_str() ("Invalid Input", 13 B); HEAD renders it via wire_name() ("InvalidInput", 12 B). One byte, every DomainError 4xx JSON body, introduced by the 210 commits between tag v0.10.0 and HE… |

Per-field verbatim values behind every row (status, content-type,
content-encoding, retry-after, decoded body length, `handler_ran`,
`app_sql_statement_count`/summaries, resolved client IP, limiter key, state
counters) live in `docs/re/evidence/differential.json` under
`cases[].builds.<label>`.

## C.4 The patch-targeted rows, raw values

`status @ decoded-body-bytes` (`ra=` is `Retry-After`); one column per build.

`setup-get`

| step | A | c0 | c1l | c1p | c1 | b |
|---|---|---|---|---|---|---|
| `get-setup` | 405 @ 0B | 405 @ 0B | 405 @ 0B | 405 @ 0B | 405 @ 0B | 405 @ 0B |
| `post-5` | 429 @ 53B ra=3600 | 429 @ 53B ra=3600 | 429 @ 53B ra=3599 | 429 @ 53B ra=3600 | 429 @ 53B ra=3599 | 429 @ 53B ra=3600 |

`ttl-refresh`

| step | A | c0 | c1l | c1p | c1 | b |
|---|---|---|---|---|---|---|
| `t0-req1` | 400 @ 167B | 400 @ 166B | 400 @ 166B | 400 @ 166B | 400 @ 166B | 400 @ 167B |
| `t0-req2` | 429 @ 51B ra=15 | 429 @ 51B ra=15 | 429 @ 51B ra=15 | 429 @ 51B ra=15 | 400 @ 166B | 429 @ 51B ra=15 |
| `t9-req3` | 429 @ 51B ra=15 | 429 @ 51B ra=15 | 429 @ 50B ra=6 | 429 @ 51B ra=15 | 400 @ 166B | 429 @ 51B ra=15 |
| `t21-req4` | 429 @ 51B ra=15 | 429 @ 51B ra=15 | 400 @ 166B | 429 @ 51B ra=15 | 400 @ 166B | 429 @ 51B ra=15 |
| `t26-req5` | 429 @ 51B ra=15 | 429 @ 51B ra=15 | 429 @ 51B ra=10 | 429 @ 51B ra=15 | 400 @ 166B | 429 @ 51B ra=15 |

`retry-after-constant`

| step | A | c0 | c1l | c1p | c1 | b |
|---|---|---|---|---|---|---|
| `post-6` | 429 @ 53B ra=3600 | 429 @ 53B ra=3600 | 429 @ 53B ra=3599 | 429 @ 53B ra=3600 | 429 @ 53B ra=3599 | 429 @ 53B ra=3600 |
| `post-after-20s` | 429 @ 53B ra=3600 | 429 @ 53B ra=3600 | 429 @ 53B ra=3578 | 429 @ 53B ra=3600 | 429 @ 53B ra=3578 | 429 @ 53B ra=3600 |

`shared-budget`

| step | A | c0 | c1l | c1p | c1 | b |
|---|---|---|---|---|---|---|
| `setup-after-register` | 429 @ 53B ra=3600 | 429 @ 53B ra=3600 | 429 @ 53B ra=3599 | 429 @ 53B ra=3600 | 400 @ 166B | 429 @ 53B ra=3600 |
| `register-after-setup` | 429 @ 53B ra=3600 | 429 @ 53B ra=3600 | 429 @ 53B ra=3599 | 429 @ 53B ra=3600 | 201 @ 140B | 429 @ 53B ra=3600 |

`xff-honoured-trusted`

| step | A | c0 | c1l | c1p | c1 | b |
|---|---|---|---|---|---|---|
| `xff-6-1` | 400 @ 167B | 400 @ 166B | 400 @ 166B | 400 @ 166B | 400 @ 166B | 400 @ 167B |
| `xff-7-1` | 400 @ 167B | 400 @ 166B | 400 @ 166B | 400 @ 166B | 400 @ 166B | 400 @ 167B |
| `xff-6-2` | 400 @ 167B | 400 @ 166B | 400 @ 166B | 400 @ 166B | 400 @ 166B | 400 @ 167B |
| `xff-7-2` | 400 @ 167B | 400 @ 166B | 400 @ 166B | 400 @ 166B | 400 @ 166B | 400 @ 167B |
| `xff-6-3` | 400 @ 167B | 400 @ 166B | 400 @ 166B | 400 @ 166B | 400 @ 166B | 400 @ 167B |
| `xff-7-3` | 400 @ 167B | 400 @ 166B | 400 @ 166B | 429 @ 53B ra=3600 | 429 @ 53B ra=3597 | 400 @ 167B |
| `peer-5` | 429 @ 53B ra=3600 | 429 @ 53B ra=3600 | 429 @ 53B ra=3599 | 429 @ 53B ra=3600 | 429 @ 53B ra=3599 | 429 @ 53B ra=3600 |
| `victim-1` | 400 @ 167B | 400 @ 166B | 400 @ 166B | 400 @ 166B | 400 @ 166B | 400 @ 167B |

## C.5 Invariants and the atomicity trials

| case | A | c0 | c1l | c1p | c1 | b |
|---|---|---|---|---|---|---|
| `terminal-201-then-403` | 201×1 owners=1 served=3 [200 201 403] | 201×1 owners=1 served=3 [201 401 403] | 201×1 owners=1 served=3 [201 401 403] | 201×1 owners=1 served=3 [201 401 403] | 201×1 owners=1 served=3 [201 401 403] | 201×1 owners=1 served=3 [200 201 403] |
| `concurrent-valid-2` | 201×1 owners=1 served=2 [201 403] | 201×1 owners=1 served=2 [201 403] | 201×1 owners=1 served=2 [201 403] | 201×1 owners=1 served=2 [201 403] | 201×1 owners=1 served=2 [201 403] | 201×1 owners=1 served=2 [201 403] |
| `concurrent-valid-8` | 201×1 owners=1 served=5 [201 403 429] | 201×1 owners=1 served=5 [201 403 429] | 201×1 owners=1 served=5 [201 403 429] | 201×1 owners=1 served=5 [201 403 429] | 201×1 owners=1 served=5 [201 403 429] | 201×1 owners=1 served=5 [201 403 429] |
| `concurrent-valid-8-unconfounded` | 201×1 owners=1 served=8 [201 403] | 201×1 owners=1 served=8 [201 403] | 201×1 owners=1 served=8 [201 403] | 201×1 owners=1 served=8 [201 403] | 201×1 owners=1 served=5 [201 403 429] | 201×1 owners=1 served=8 [201 403] |
| `concurrent-valid-16-unconfounded` | 201×1 owners=1 served=16 [201 403] | 201×1 owners=1 served=16 [201 403] | 201×1 owners=1 served=16 [201 403] | 201×1 owners=1 served=16 [201 403] | 201×1 owners=1 served=5 [201 403 429] | 201×1 owners=1 served=16 [201 403] |
| `concurrent-burst-rejects-8` | 201×0 owners=0 served=1 [400 429] | 201×0 owners=0 served=1 [400 429] | 201×0 owners=0 served=1 [400 429] | 201×0 owners=0 served=1 [400 429] | 201×0 owners=0 served=5 [400 403 429] | 201×0 owners=0 served=1 [400 429] |
| `concurrent-burst-rejects-16` | 201×0 owners=0 served=1 [400 429] | 201×0 owners=0 served=1 [400 429] | 201×0 owners=0 served=1 [400 429] | 201×0 owners=0 served=1 [400 429] | 201×0 owners=0 served=5 [400 403 429] | 201×0 owners=0 served=1 [400 429] |

* `c0` (`c0-head-unpatched`, N=64, both budgets pinned to max=1, window 15 s): overshoot in **0 of 5** trials, max served 1; per-trial served counts [1, 1, 1, 1, 1]; wall-clock per burst [0.4, 0.398, 0.396, 0.431, 0.383] s.
* `c1` (`c1-patched`, N=64, both budgets pinned to max=1, window 15 s): overshoot in **0 of 5** trials, max served 1; per-trial served counts [1, 1, 1, 1, 1]; wall-clock per burst [0.389, 0.385, 0.39, 0.388, 0.396] s.

## C.6 Every difference that is NOT `match` / `explained-drift` / `fix-confirmed`

Generated from `differential.json` with **no length cap and no truncation**:
each bullet names the case, the reference build it was compared against, the
build, and every differing field with both values.

* `compression-bracket`, A vs `c1`: step 3 `pw7-400` `body_decoded_len`: `167` → `166`; step 3 `pw7-400` `body_len`: `167` → `166`; step 3 `pw7-400` `response` — `content_length`: `167` → `166`; step 4 `bademail-400` `body_decoded_len`: `219` → `218`; step 4 `bademail-400` `body_len`: `219` → `218`; step 4 `bademail-400` `response` — `content_length`: `219` → `218`; step 5 `nodot-400` `body_decoded_len`: `251` → `250`; step 5 `nodot-400` `body_len`: `251` → `250`; step 5 `nodot-400` `response` — `content_length`: `251` → `250`; step 6 `forbidden-400` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin request received for user: admin', 'oxicloud::application::services::auth_application_service: Available disk space (N bytes) is less than default owner quota (107374182400 bytes) — capping quota', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin creation failed: Invalid Input: Error creating admin user: Validation error: Invalid email: contains forbidden characters', 'audit: 🔄 setup admin failed; released initialization claim so retries can proceed event="setup.claim_released" reason=Invalid Input: Error creating admin user: Validation error: Invalid email: contains forbidden characters', 'http::api: client_error status=400 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=400', 'tower_http::trace::on_eos: end of stream stream_duration=0 ms']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=127.0.0.1 endpoint="register"', 'http::api: client_error status=429 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=429']`; step 6 `forbidden-400` `app_sql_statement_count`: `5` → `0`; step 6 `forbidden-400` `app_sql_statement_summaries`: `['SELECT value FROM auth.admin_settings …', 'INSERT INTO auth.admin_settings (key, …', 'SELECT id, username, email, …', 'SELECT id, username, email, …', 'DELETE FROM auth.admin_settings WHERE …']` → `[]`; step 6 `forbidden-400` `body_decoded_len`: `259` → `53`; step 6 `forbidden-400` `body_decoded_source`: `gzip` → `identity`; step 6 `forbidden-400` `body_len`: `151` → `53`; step 6 `forbidden-400` `handler_ran`: `True` → `False`; step 6 `forbidden-400` `limiter_ip`: `` → `127.0.0.1`; step 6 `forbidden-400` `response` — `content_encoding`: `gzip` → ``; `content_length`: `` → `53`; `retry_after`: `` → `3599`; `status`: `400` → `429`; step 7 `badchars-400` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin request received for user: ad min', 'oxicloud::application::services::auth_application_service: Available disk space (N bytes) is less than default owner quota (107374182400 bytes) — capping quota', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin creation failed: Invalid Input: Error creating admin user: Invalid username: Username may only contain letters, digits, hyphens, underscores, and dots', 'audit: 🔄 setup admin failed; released initialization claim so retries can proceed event="setup.claim_released" reason=Invalid Input: Error creating admin user: Invalid username: Username may only contain letters, digits, hyphens, underscores, and dots', 'http::api: client_error status=400 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=400', 'tower_http::trace::on_eos: end of stream stream_duration=0 ms']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=127.0.0.1 endpoint="register"', 'http::api: client_error status=429 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=429']`; step 7 `badchars-400` `app_sql_statement_count`: `5` → `0`; step 7 `badchars-400` `app_sql_statement_summaries`: `['SELECT value FROM auth.admin_settings …', 'INSERT INTO auth.admin_settings (key, …', 'SELECT id, username, email, …', 'SELECT id, username, email, …', 'DELETE FROM auth.admin_settings WHERE …']` → `[]`; step 7 `badchars-400` `body_decoded_len`: `317` → `53`; step 7 `badchars-400` `body_decoded_source`: `gzip` → `identity`; step 7 `badchars-400` `body_len`: `167` → `53`; step 7 `badchars-400` `handler_ran`: `True` → `False`; step 7 `badchars-400` `limiter_ip`: `` → `127.0.0.1`; step 7 `badchars-400` `response` — `content_encoding`: `gzip` → ``; `content_length`: `` → `53`; `retry_after`: `` → `3598`; `status`: `400` → `429` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `compression-bracket`, c0 vs `c1`: step 6 `forbidden-400` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin request received for user: admin', 'oxicloud::application::services::auth_application_service: Available disk space (N bytes) is less than default owner quota (107374182400 bytes) — capping quota', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin creation failed: Invalid Input: Error creating admin user: Validation error: Invalid email: contains forbidden characters', 'audit: 🔄 setup admin failed; released initialization claim so retries can proceed event="setup.claim_released" reason=Invalid Input: Error creating admin user: Validation error: Invalid email: contains forbidden characters', 'http::api: client_error status=400 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=400', 'tower_http::trace::on_eos: end of stream stream_duration=0 ms']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=127.0.0.1 endpoint="register"', 'http::api: client_error status=429 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=429']`; step 6 `forbidden-400` `app_sql_statement_count`: `5` → `0`; step 6 `forbidden-400` `app_sql_statement_summaries`: `['SELECT value FROM auth.admin_settings …', 'INSERT INTO auth.admin_settings (key, …', 'SELECT id, username, email, …', 'SELECT id, username, email, …', 'DELETE FROM auth.admin_settings WHERE …']` → `[]`; step 6 `forbidden-400` `body_decoded_len`: `258` → `53`; step 6 `forbidden-400` `body_decoded_source`: `gzip` → `identity`; step 6 `forbidden-400` `body_len`: `151` → `53`; step 6 `forbidden-400` `handler_ran`: `True` → `False`; step 6 `forbidden-400` `limiter_ip`: `` → `127.0.0.1`; step 6 `forbidden-400` `response` — `content_encoding`: `gzip` → ``; `content_length`: `` → `53`; `retry_after`: `` → `3599`; `status`: `400` → `429`; step 7 `badchars-400` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin request received for user: ad min', 'oxicloud::application::services::auth_application_service: Available disk space (N bytes) is less than default owner quota (107374182400 bytes) — capping quota', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin creation failed: Invalid Input: Error creating admin user: Invalid username: Username may only contain letters, digits, hyphens, underscores, and dots', 'audit: 🔄 setup admin failed; released initialization claim so retries can proceed event="setup.claim_released" reason=Invalid Input: Error creating admin user: Invalid username: Username may only contain letters, digits, hyphens, underscores, and dots', 'http::api: client_error status=400 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=400', 'tower_http::trace::on_eos: end of stream stream_duration=0 ms']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=127.0.0.1 endpoint="register"', 'http::api: client_error status=429 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=429']`; step 7 `badchars-400` `app_sql_statement_count`: `5` → `0`; step 7 `badchars-400` `app_sql_statement_summaries`: `['SELECT value FROM auth.admin_settings …', 'INSERT INTO auth.admin_settings (key, …', 'SELECT id, username, email, …', 'SELECT id, username, email, …', 'DELETE FROM auth.admin_settings WHERE …']` → `[]`; step 7 `badchars-400` `body_decoded_len`: `316` → `53`; step 7 `badchars-400` `body_decoded_source`: `gzip` → `identity`; step 7 `badchars-400` `body_len`: `166` → `53`; step 7 `badchars-400` `handler_ran`: `True` → `False`; step 7 `badchars-400` `limiter_ip`: `` → `127.0.0.1`; step 7 `badchars-400` `response` — `content_encoding`: `gzip` → ``; `content_length`: `` → `53`; `retry_after`: `` → `3598`; `status`: `400` → `429` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `concurrent-burst-rejects-16`, A vs `c1`: `status_multiset`: `[400, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429]` → `[400, 400, 403, 403, 403, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429]` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `concurrent-burst-rejects-16`, c0 vs `c1`: `status_multiset`: `[400, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429]` → `[400, 400, 403, 403, 403, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429]` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `concurrent-burst-rejects-8`, A vs `c1`: `status_multiset`: `[400, 429, 429, 429, 429, 429, 429, 429]` → `[400, 400, 400, 403, 403, 429, 429, 429]` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `concurrent-burst-rejects-8`, c0 vs `c1`: `status_multiset`: `[400, 429, 429, 429, 429, 429, 429, 429]` → `[400, 400, 400, 403, 403, 429, 429, 429]` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `concurrent-valid-16-unconfounded`, A vs `c1`: `status_multiset`: `[201, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403]` → `[201, 403, 403, 403, 403, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429]` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `concurrent-valid-16-unconfounded`, c0 vs `c1`: `status_multiset`: `[201, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403, 403]` → `[201, 403, 403, 403, 403, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429, 429]` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `concurrent-valid-8-unconfounded`, A vs `c1`: `status_multiset`: `[201, 403, 403, 403, 403, 403, 403, 403]` → `[201, 403, 403, 403, 403, 429, 429, 429]` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `concurrent-valid-8-unconfounded`, c0 vs `c1`: `status_multiset`: `[201, 403, 403, 403, 403, 403, 403, 403]` → `[201, 403, 403, 403, 403, 429, 429, 429]` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `shared-budget`, A vs `c1`: step 6 `setup-after-register` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=127.0.0.1 endpoint="register"', 'http::api: client_error status=429 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=429']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin request received for user: admin', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin creation failed: Invalid Input: Password must be at least 8 characters long', 'audit: 🔄 setup admin failed; released initialization claim so retries can proceed event="setup.claim_released" reason=Invalid Input: Password must be at least 8 characters long', 'http::api: client_error status=400 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=400']`; step 6 `setup-after-register` `app_sql_statement_count`: `0` → `5`; step 6 `setup-after-register` `app_sql_statement_summaries`: `[]` → `['SELECT value FROM auth.admin_settings …', 'INSERT INTO auth.admin_settings (key, …', 'SELECT id, username, email, …', 'SELECT id, username, email, …', 'DELETE FROM auth.admin_settings WHERE …']`; step 6 `setup-after-register` `body_decoded_len`: `53` → `166`; step 6 `setup-after-register` `body_len`: `53` → `166`; step 6 `setup-after-register` `handler_ran`: `False` → `True`; step 6 `setup-after-register` `limiter_ip`: `127.0.0.1` → ``; step 6 `setup-after-register` `response` — `content_length`: `53` → `166`; `retry_after`: `3600` → ``; `status`: `429` → `400`; step 7 `setup-1` `body_decoded_len`: `167` → `166`; step 7 `setup-1` `body_len`: `167` → `166`; step 7 `setup-1` `response` — `content_length`: `167` → `166`; step 8 `setup-2` `body_decoded_len`: `167` → `166`; step 8 `setup-2` `body_len`: `167` → `166`; step 8 `setup-2` `response` — `content_length`: `167` → `166`; step 9 `setup-3` `body_decoded_len`: `167` → `166`; step 9 `setup-3` `body_len`: `167` → `166`; step 9 `setup-3` `response` — `content_length`: `167` → `166`; step 10 `setup-4` `body_decoded_len`: `167` → `166`; step 10 `setup-4` `body_len`: `167` → `166`; step 10 `setup-4` `response` — `content_length`: `167` → `166`; step 11 `setup-5` `body_decoded_len`: `167` → `166`; step 11 `setup-5` `body_len`: `167` → `166`; step 11 `setup-5` `response` — `content_length`: `167` → `166`; step 12 `register-after-setup` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=127.0.0.1 endpoint="register"', 'http::api::auth: client_error status=429 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=429']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::infrastructure::repositories::pg::transaction_utils: Starting database transaction for: create_user', 'oxicloud::infrastructure::repositories::pg::transaction_utils: Transaction committed successfully for: create_user', 'audit: event="user.created" user_id=<uuid> username=<uuid> is_external=false', 'user_lifecycle: Default personal drive + root folder + owner grant provisioned (atomic CTE) hook="personal_drive" user_id=<uuid> drive_id=<uuid> root_folder_id=<uuid>', 'user_lifecycle: Default calendar provisioned hook="default_calendar" user_id=<uuid> calendar_id=<uuid>', 'user_lifecycle: Default address book provisioned hook="default_address_book" user_id=<uuid> address_book_id=<uuid>', 'audit: 🛂 user registered event="auth.register" reason="created" user_id=<uuid> username=<uuid> email=re-reg@example.test is_external=false', 'tower_http::trace::on_response: finished processing request status=201']`; step 12 `register-after-setup` `app_sql_statement_count`: `0` → `19`; step 12 `register-after-setup` `app_sql_statement_summaries`: `[]` → `['SELECT value FROM auth.admin_settings …', 'SELECT id, username, email, …', 'SELECT oid::regtype::text, typtype, typcategory, …', 'SELECT enumlabel FROM pg_catalog.pg_enum …', 'INSERT INTO auth.users ( …', 'COMMIT', 'SELECT d.id, d.kind, d.default_for_user, …', 'INSERT INTO storage.drives (kind, …', 'INSERT INTO storage.folders (name, …', 'UPDATE storage.drives SET root_folder_id …', 'INSERT INTO storage.role_grants (subject_type, …', 'SELECT d.id, d.kind, d.default_for_user, …', 'COMMIT', 'SELECT EXISTS(SELECT 1 FROM …', 'INSERT INTO caldav.calendars (id, …', 'INSERT INTO storage.role_grants (subject_type, …', 'SELECT EXISTS(SELECT 1 FROM …', 'INSERT INTO carddav.address_books (id, …', 'INSERT INTO storage.role_grants (subject_type, …']`; step 12 `register-after-setup` `body_decoded_len`: `53` → `140`; step 12 `register-after-setup` `body_len`: `53` → `140`; step 12 `register-after-setup` `limiter_ip`: `127.0.0.1` → ``; step 12 `register-after-setup` `response` — `content_length`: `53` → `140`; `retry_after`: `3600` → ``; `status`: `429` → `201`; `state_after` — `users`: `0` → `1` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `shared-budget`, c0 vs `c1`: step 6 `setup-after-register` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=127.0.0.1 endpoint="register"', 'http::api: client_error status=429 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=429']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin request received for user: admin', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin creation failed: Invalid Input: Password must be at least 8 characters long', 'audit: 🔄 setup admin failed; released initialization claim so retries can proceed event="setup.claim_released" reason=Invalid Input: Password must be at least 8 characters long', 'http::api: client_error status=400 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=400']`; step 6 `setup-after-register` `app_sql_statement_count`: `0` → `5`; step 6 `setup-after-register` `app_sql_statement_summaries`: `[]` → `['SELECT value FROM auth.admin_settings …', 'INSERT INTO auth.admin_settings (key, …', 'SELECT id, username, email, …', 'SELECT id, username, email, …', 'DELETE FROM auth.admin_settings WHERE …']`; step 6 `setup-after-register` `body_decoded_len`: `53` → `166`; step 6 `setup-after-register` `body_len`: `53` → `166`; step 6 `setup-after-register` `handler_ran`: `False` → `True`; step 6 `setup-after-register` `limiter_ip`: `127.0.0.1` → ``; step 6 `setup-after-register` `response` — `content_length`: `53` → `166`; `retry_after`: `3600` → ``; `status`: `429` → `400`; step 12 `register-after-setup` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=127.0.0.1 endpoint="register"', 'http::api::auth: client_error status=429 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=429']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::infrastructure::repositories::pg::transaction_utils: Starting database transaction for: create_user', 'oxicloud::infrastructure::repositories::pg::transaction_utils: Transaction committed successfully for: create_user', 'audit: event="user.created" user_id=<uuid> username=<uuid> is_external=false', 'user_lifecycle: Default personal drive + root folder + owner grant provisioned (atomic CTE) hook="personal_drive" user_id=<uuid> drive_id=<uuid> root_folder_id=<uuid>', 'user_lifecycle: Default calendar provisioned hook="default_calendar" user_id=<uuid> calendar_id=<uuid>', 'user_lifecycle: Default address book provisioned hook="default_address_book" user_id=<uuid> address_book_id=<uuid>', 'audit: 🛂 user registered event="auth.register" reason="created" user_id=<uuid> username=<uuid> email=re-reg@example.test is_external=false', 'tower_http::trace::on_response: finished processing request status=201']`; step 12 `register-after-setup` `app_sql_statement_count`: `0` → `19`; step 12 `register-after-setup` `app_sql_statement_summaries`: `[]` → `['SELECT value FROM auth.admin_settings …', 'SELECT id, username, email, …', 'SELECT oid::regtype::text, typtype, typcategory, …', 'SELECT enumlabel FROM pg_catalog.pg_enum …', 'INSERT INTO auth.users ( …', 'COMMIT', 'SELECT d.id, d.kind, d.default_for_user, …', 'INSERT INTO storage.drives (kind, …', 'INSERT INTO storage.folders (name, …', 'UPDATE storage.drives SET root_folder_id …', 'INSERT INTO storage.role_grants (subject_type, …', 'SELECT d.id, d.kind, d.default_for_user, …', 'COMMIT', 'SELECT EXISTS(SELECT 1 FROM …', 'INSERT INTO caldav.calendars (id, …', 'INSERT INTO storage.role_grants (subject_type, …', 'SELECT EXISTS(SELECT 1 FROM …', 'INSERT INTO carddav.address_books (id, …', 'INSERT INTO storage.role_grants (subject_type, …']`; step 12 `register-after-setup` `body_decoded_len`: `53` → `140`; step 12 `register-after-setup` `body_len`: `53` → `140`; step 12 `register-after-setup` `limiter_ip`: `127.0.0.1` → ``; step 12 `register-after-setup` `response` — `content_length`: `53` → `140`; `retry_after`: `3600` → ``; `status`: `429` → `201`; `state_after` — `users`: `0` → `1` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `terminal-201-then-403`, A vs `c0`: step 3 `status` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: System status check: initialized=true, admin_count=1', 'tower_http::trace::on_response: finished processing request status=200']` → `['tower_http::trace::on_request: started processing request', 'http::api: client_error status=401 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=401']`; step 3 `status` `app_sql_statement_count`: `2` → `0`; step 3 `status` `app_sql_statement_summaries`: `['SELECT value FROM auth.admin_settings …', 'SELECT COUNT(*) FROM auth.users …']` → `[]`; step 3 `status` `body_decoded_len`: `86` → `117`; step 3 `status` `body_len`: `86` → `117`; step 3 `status` `response` — `content_length`: `86` → `117`; `status`: `200` → `401` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `terminal-201-then-403`, A vs `c1l`: step 3 `status` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: System status check: initialized=true, admin_count=1', 'tower_http::trace::on_response: finished processing request status=200']` → `['tower_http::trace::on_request: started processing request', 'http::api: client_error status=401 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=401']`; step 3 `status` `app_sql_statement_count`: `2` → `0`; step 3 `status` `app_sql_statement_summaries`: `['SELECT value FROM auth.admin_settings …', 'SELECT COUNT(*) FROM auth.users …']` → `[]`; step 3 `status` `body_decoded_len`: `86` → `117`; step 3 `status` `body_len`: `86` → `117`; step 3 `status` `response` — `content_length`: `86` → `117`; `status`: `200` → `401` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `terminal-201-then-403`, A vs `c1p`: step 3 `status` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: System status check: initialized=true, admin_count=1', 'tower_http::trace::on_response: finished processing request status=200']` → `['tower_http::trace::on_request: started processing request', 'http::api: client_error status=401 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=401']`; step 3 `status` `app_sql_statement_count`: `2` → `0`; step 3 `status` `app_sql_statement_summaries`: `['SELECT value FROM auth.admin_settings …', 'SELECT COUNT(*) FROM auth.users …']` → `[]`; step 3 `status` `body_decoded_len`: `86` → `117`; step 3 `status` `body_len`: `86` → `117`; step 3 `status` `response` — `content_length`: `86` → `117`; `status`: `200` → `401` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `terminal-201-then-403`, A vs `c1`: step 3 `status` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: System status check: initialized=true, admin_count=1', 'tower_http::trace::on_response: finished processing request status=200']` → `['tower_http::trace::on_request: started processing request', 'http::api: client_error status=401 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=401']`; step 3 `status` `app_sql_statement_count`: `2` → `0`; step 3 `status` `app_sql_statement_summaries`: `['SELECT value FROM auth.admin_settings …', 'SELECT COUNT(*) FROM auth.users …']` → `[]`; step 3 `status` `body_decoded_len`: `86` → `117`; step 3 `status` `body_len`: `86` → `117`; step 3 `status` `response` — `content_length`: `86` → `117`; `status`: `200` → `401` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `terminal-201-then-403`, c0 vs `b`: step 3 `status` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'http::api: client_error status=401 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=401']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: System status check: initialized=true, admin_count=1', 'tower_http::trace::on_response: finished processing request status=200']`; step 3 `status` `app_sql_statement_count`: `0` → `2`; step 3 `status` `app_sql_statement_summaries`: `[]` → `['SELECT value FROM auth.admin_settings …', 'SELECT COUNT(*) FROM auth.users …']`; step 3 `status` `body_decoded_len`: `117` → `86`; step 3 `status` `body_len`: `117` → `86`; step 3 `status` `response` — `content_length`: `117` → `86`; `status`: `401` → `200` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `ttl-refresh`, A vs `c1l`: step 1 `t0-req1` `body_decoded_len`: `167` → `166`; step 1 `t0-req1` `body_len`: `167` → `166`; step 1 `t0-req1` `response` — `content_length`: `167` → `166`; step 3 `t9-req3` `body_decoded_len`: `51` → `50`; step 3 `t9-req3` `body_len`: `51` → `50`; step 3 `t9-req3` `response` — `content_length`: `51` → `50`; `retry_after`: `15` → `6`; step 4 `t21-req4` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=127.0.0.1 endpoint="register"', 'http::api: client_error status=429 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=429']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin request received for user: admin', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin creation failed: Invalid Input: Password must be at least 8 characters long', 'audit: 🔄 setup admin failed; released initialization claim so retries can proceed event="setup.claim_released" reason=Invalid Input: Password must be at least 8 characters long', 'http::api: client_error status=400 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=400']`; step 4 `t21-req4` `app_sql_statement_count`: `0` → `5`; step 4 `t21-req4` `app_sql_statement_summaries`: `[]` → `['SELECT value FROM auth.admin_settings …', 'INSERT INTO auth.admin_settings (key, …', 'SELECT id, username, email, …', 'SELECT id, username, email, …', 'DELETE FROM auth.admin_settings WHERE …']`; step 4 `t21-req4` `body_decoded_len`: `51` → `166`; step 4 `t21-req4` `body_len`: `51` → `166`; step 4 `t21-req4` `handler_ran`: `False` → `True`; step 4 `t21-req4` `limiter_ip`: `127.0.0.1` → ``; step 4 `t21-req4` `response` — `content_length`: `51` → `166`; `retry_after`: `15` → ``; `status`: `429` → `400`; step 5 `t26-req5` `response` — `retry_after`: `15` → `10` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `ttl-refresh`, A vs `c1`: step 1 `t0-req1` `body_decoded_len`: `167` → `166`; step 1 `t0-req1` `body_len`: `167` → `166`; step 1 `t0-req1` `response` — `content_length`: `167` → `166`; step 2 `t0-req2` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=127.0.0.1 endpoint="register"', 'http::api: client_error status=429 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=429']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin request received for user: admin', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin creation failed: Invalid Input: Password must be at least 8 characters long', 'audit: 🔄 setup admin failed; released initialization claim so retries can proceed event="setup.claim_released" reason=Invalid Input: Password must be at least 8 characters long', 'http::api: client_error status=400 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=400']`; step 2 `t0-req2` `app_sql_statement_count`: `0` → `5`; step 2 `t0-req2` `app_sql_statement_summaries`: `[]` → `['SELECT value FROM auth.admin_settings …', 'INSERT INTO auth.admin_settings (key, …', 'SELECT id, username, email, …', 'SELECT id, username, email, …', 'DELETE FROM auth.admin_settings WHERE …']`; step 2 `t0-req2` `body_decoded_len`: `51` → `166`; step 2 `t0-req2` `body_len`: `51` → `166`; step 2 `t0-req2` `handler_ran`: `False` → `True`; step 2 `t0-req2` `limiter_ip`: `127.0.0.1` → ``; step 2 `t0-req2` `response` — `content_length`: `51` → `166`; `retry_after`: `15` → ``; `status`: `429` → `400`; step 3 `t9-req3` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=127.0.0.1 endpoint="register"', 'http::api: client_error status=429 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=429']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin request received for user: admin', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin creation failed: Invalid Input: Password must be at least 8 characters long', 'audit: 🔄 setup admin failed; released initialization claim so retries can proceed event="setup.claim_released" reason=Invalid Input: Password must be at least 8 characters long', 'http::api: client_error status=400 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=400']`; step 3 `t9-req3` `app_sql_statement_count`: `0` → `5`; step 3 `t9-req3` `app_sql_statement_summaries`: `[]` → `['SELECT value FROM auth.admin_settings …', 'INSERT INTO auth.admin_settings (key, …', 'SELECT id, username, email, …', 'SELECT id, username, email, …', 'DELETE FROM auth.admin_settings WHERE …']`; step 3 `t9-req3` `body_decoded_len`: `51` → `166`; step 3 `t9-req3` `body_len`: `51` → `166`; step 3 `t9-req3` `handler_ran`: `False` → `True`; step 3 `t9-req3` `limiter_ip`: `127.0.0.1` → ``; step 3 `t9-req3` `response` — `content_length`: `51` → `166`; `retry_after`: `15` → ``; `status`: `429` → `400`; step 4 `t21-req4` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=127.0.0.1 endpoint="register"', 'http::api: client_error status=429 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=429']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin request received for user: admin', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin creation failed: Invalid Input: Password must be at least 8 characters long', 'audit: 🔄 setup admin failed; released initialization claim so retries can proceed event="setup.claim_released" reason=Invalid Input: Password must be at least 8 characters long', 'http::api: client_error status=400 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=400']`; step 4 `t21-req4` `app_sql_statement_count`: `0` → `5`; step 4 `t21-req4` `app_sql_statement_summaries`: `[]` → `['SELECT value FROM auth.admin_settings …', 'INSERT INTO auth.admin_settings (key, …', 'SELECT id, username, email, …', 'SELECT id, username, email, …', 'DELETE FROM auth.admin_settings WHERE …']`; step 4 `t21-req4` `body_decoded_len`: `51` → `166`; step 4 `t21-req4` `body_len`: `51` → `166`; step 4 `t21-req4` `handler_ran`: `False` → `True`; step 4 `t21-req4` `limiter_ip`: `127.0.0.1` → ``; step 4 `t21-req4` `response` — `content_length`: `51` → `166`; `retry_after`: `15` → ``; `status`: `429` → `400`; step 5 `t26-req5` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=127.0.0.1 endpoint="register"', 'http::api: client_error status=429 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=429']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin request received for user: admin', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin creation failed: Invalid Input: Password must be at least 8 characters long', 'audit: 🔄 setup admin failed; released initialization claim so retries can proceed event="setup.claim_released" reason=Invalid Input: Password must be at least 8 characters long', 'http::api: client_error status=400 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=400']`; step 5 `t26-req5` `app_sql_statement_count`: `0` → `5`; step 5 `t26-req5` `app_sql_statement_summaries`: `[]` → `['SELECT value FROM auth.admin_settings …', 'INSERT INTO auth.admin_settings (key, …', 'SELECT id, username, email, …', 'SELECT id, username, email, …', 'DELETE FROM auth.admin_settings WHERE …']`; step 5 `t26-req5` `body_decoded_len`: `51` → `166`; step 5 `t26-req5` `body_len`: `51` → `166`; step 5 `t26-req5` `handler_ran`: `False` → `True`; step 5 `t26-req5` `limiter_ip`: `127.0.0.1` → ``; step 5 `t26-req5` `response` — `content_length`: `51` → `166`; `retry_after`: `15` → ``; `status`: `429` → `400` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `ttl-refresh`, c0 vs `c1l`: step 3 `t9-req3` `body_decoded_len`: `51` → `50`; step 3 `t9-req3` `body_len`: `51` → `50`; step 3 `t9-req3` `response` — `content_length`: `51` → `50`; `retry_after`: `15` → `6`; step 4 `t21-req4` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=127.0.0.1 endpoint="register"', 'http::api: client_error status=429 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=429']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin request received for user: admin', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin creation failed: Invalid Input: Password must be at least 8 characters long', 'audit: 🔄 setup admin failed; released initialization claim so retries can proceed event="setup.claim_released" reason=Invalid Input: Password must be at least 8 characters long', 'http::api: client_error status=400 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=400']`; step 4 `t21-req4` `app_sql_statement_count`: `0` → `5`; step 4 `t21-req4` `app_sql_statement_summaries`: `[]` → `['SELECT value FROM auth.admin_settings …', 'INSERT INTO auth.admin_settings (key, …', 'SELECT id, username, email, …', 'SELECT id, username, email, …', 'DELETE FROM auth.admin_settings WHERE …']`; step 4 `t21-req4` `body_decoded_len`: `51` → `166`; step 4 `t21-req4` `body_len`: `51` → `166`; step 4 `t21-req4` `handler_ran`: `False` → `True`; step 4 `t21-req4` `limiter_ip`: `127.0.0.1` → ``; step 4 `t21-req4` `response` — `content_length`: `51` → `166`; `retry_after`: `15` → ``; `status`: `429` → `400`; step 5 `t26-req5` `response` — `retry_after`: `15` → `10` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `ttl-refresh`, c0 vs `c1`: step 2 `t0-req2` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=127.0.0.1 endpoint="register"', 'http::api: client_error status=429 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=429']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin request received for user: admin', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin creation failed: Invalid Input: Password must be at least 8 characters long', 'audit: 🔄 setup admin failed; released initialization claim so retries can proceed event="setup.claim_released" reason=Invalid Input: Password must be at least 8 characters long', 'http::api: client_error status=400 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=400']`; step 2 `t0-req2` `app_sql_statement_count`: `0` → `5`; step 2 `t0-req2` `app_sql_statement_summaries`: `[]` → `['SELECT value FROM auth.admin_settings …', 'INSERT INTO auth.admin_settings (key, …', 'SELECT id, username, email, …', 'SELECT id, username, email, …', 'DELETE FROM auth.admin_settings WHERE …']`; step 2 `t0-req2` `body_decoded_len`: `51` → `166`; step 2 `t0-req2` `body_len`: `51` → `166`; step 2 `t0-req2` `handler_ran`: `False` → `True`; step 2 `t0-req2` `limiter_ip`: `127.0.0.1` → ``; step 2 `t0-req2` `response` — `content_length`: `51` → `166`; `retry_after`: `15` → ``; `status`: `429` → `400`; step 3 `t9-req3` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=127.0.0.1 endpoint="register"', 'http::api: client_error status=429 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=429']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin request received for user: admin', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin creation failed: Invalid Input: Password must be at least 8 characters long', 'audit: 🔄 setup admin failed; released initialization claim so retries can proceed event="setup.claim_released" reason=Invalid Input: Password must be at least 8 characters long', 'http::api: client_error status=400 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=400']`; step 3 `t9-req3` `app_sql_statement_count`: `0` → `5`; step 3 `t9-req3` `app_sql_statement_summaries`: `[]` → `['SELECT value FROM auth.admin_settings …', 'INSERT INTO auth.admin_settings (key, …', 'SELECT id, username, email, …', 'SELECT id, username, email, …', 'DELETE FROM auth.admin_settings WHERE …']`; step 3 `t9-req3` `body_decoded_len`: `51` → `166`; step 3 `t9-req3` `body_len`: `51` → `166`; step 3 `t9-req3` `handler_ran`: `False` → `True`; step 3 `t9-req3` `limiter_ip`: `127.0.0.1` → ``; step 3 `t9-req3` `response` — `content_length`: `51` → `166`; `retry_after`: `15` → ``; `status`: `429` → `400`; step 4 `t21-req4` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=127.0.0.1 endpoint="register"', 'http::api: client_error status=429 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=429']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin request received for user: admin', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin creation failed: Invalid Input: Password must be at least 8 characters long', 'audit: 🔄 setup admin failed; released initialization claim so retries can proceed event="setup.claim_released" reason=Invalid Input: Password must be at least 8 characters long', 'http::api: client_error status=400 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=400']`; step 4 `t21-req4` `app_sql_statement_count`: `0` → `5`; step 4 `t21-req4` `app_sql_statement_summaries`: `[]` → `['SELECT value FROM auth.admin_settings …', 'INSERT INTO auth.admin_settings (key, …', 'SELECT id, username, email, …', 'SELECT id, username, email, …', 'DELETE FROM auth.admin_settings WHERE …']`; step 4 `t21-req4` `body_decoded_len`: `51` → `166`; step 4 `t21-req4` `body_len`: `51` → `166`; step 4 `t21-req4` `handler_ran`: `False` → `True`; step 4 `t21-req4` `limiter_ip`: `127.0.0.1` → ``; step 4 `t21-req4` `response` — `content_length`: `51` → `166`; `retry_after`: `15` → ``; `status`: `429` → `400`; step 5 `t26-req5` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=127.0.0.1 endpoint="register"', 'http::api: client_error status=429 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=429']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin request received for user: admin', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin creation failed: Invalid Input: Password must be at least 8 characters long', 'audit: 🔄 setup admin failed; released initialization claim so retries can proceed event="setup.claim_released" reason=Invalid Input: Password must be at least 8 characters long', 'http::api: client_error status=400 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=400']`; step 5 `t26-req5` `app_sql_statement_count`: `0` → `5`; step 5 `t26-req5` `app_sql_statement_summaries`: `[]` → `['SELECT value FROM auth.admin_settings …', 'INSERT INTO auth.admin_settings (key, …', 'SELECT id, username, email, …', 'SELECT id, username, email, …', 'DELETE FROM auth.admin_settings WHERE …']`; step 5 `t26-req5` `body_decoded_len`: `51` → `166`; step 5 `t26-req5` `body_len`: `51` → `166`; step 5 `t26-req5` `handler_ran`: `False` → `True`; step 5 `t26-req5` `limiter_ip`: `127.0.0.1` → ``; step 5 `t26-req5` `response` — `content_length`: `51` → `166`; `retry_after`: `15` → ``; `status`: `429` → `400` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `xff-honoured-trusted`, A vs `c1p`: step 1 `xff-6-1` `body_decoded_len`: `167` → `166`; step 1 `xff-6-1` `body_len`: `167` → `166`; step 1 `xff-6-1` `resolved_client_ip`: `6.6.6.6` → `10.0.0.5`; step 1 `xff-6-1` `response` — `content_length`: `167` → `166`; step 2 `xff-7-1` `body_decoded_len`: `167` → `166`; step 2 `xff-7-1` `body_len`: `167` → `166`; step 2 `xff-7-1` `resolved_client_ip`: `7.7.7.7` → `10.0.0.5`; step 2 `xff-7-1` `response` — `content_length`: `167` → `166`; step 3 `xff-6-2` `body_decoded_len`: `167` → `166`; step 3 `xff-6-2` `body_len`: `167` → `166`; step 3 `xff-6-2` `resolved_client_ip`: `6.6.6.6` → `10.0.0.5`; step 3 `xff-6-2` `response` — `content_length`: `167` → `166`; step 4 `xff-7-2` `body_decoded_len`: `167` → `166`; step 4 `xff-7-2` `body_len`: `167` → `166`; step 4 `xff-7-2` `resolved_client_ip`: `7.7.7.7` → `10.0.0.5`; step 4 `xff-7-2` `response` — `content_length`: `167` → `166`; step 5 `peer-1` `body_decoded_len`: `167` → `166`; step 5 `peer-1` `body_len`: `167` → `166`; step 5 `peer-1` `response` — `content_length`: `167` → `166`; step 6 `xff-peer` `body_decoded_len`: `167` → `166`; step 6 `xff-peer` `body_len`: `167` → `166`; step 6 `xff-peer` `response` — `content_length`: `167` → `166`; step 7 `peer-2` `body_decoded_len`: `167` → `166`; step 7 `peer-2` `body_len`: `167` → `166`; step 7 `peer-2` `response` — `content_length`: `167` → `166`; step 8 `peer-3` `body_decoded_len`: `167` → `166`; step 8 `peer-3` `body_len`: `167` → `166`; step 8 `peer-3` `response` — `content_length`: `167` → `166`; step 9 `peer-4` `body_decoded_len`: `167` → `166`; step 9 `peer-4` `body_len`: `167` → `166`; step 9 `peer-4` `response` — `content_length`: `167` → `166`; step 11 `xff-6-3` `body_decoded_len`: `167` → `166`; step 11 `xff-6-3` `body_len`: `167` → `166`; step 11 `xff-6-3` `resolved_client_ip`: `6.6.6.6` → `10.0.0.5`; step 11 `xff-6-3` `response` — `content_length`: `167` → `166`; step 12 `xff-7-3` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin request received for user: admin', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin creation failed: Invalid Input: Password must be at least 8 characters long', 'audit: 🔄 setup admin failed; released initialization claim so retries can proceed event="setup.claim_released" reason=Invalid Input: Password must be at least 8 characters long', 'http::api: client_error status=400 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=400']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=10.0.0.5 endpoint="register"', 'http::api: client_error status=429 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=429']`; step 12 `xff-7-3` `app_sql_statement_count`: `5` → `0`; step 12 `xff-7-3` `app_sql_statement_summaries`: `['SELECT value FROM auth.admin_settings …', 'INSERT INTO auth.admin_settings (key, …', 'SELECT id, username, email, …', 'SELECT id, username, email, …', 'DELETE FROM auth.admin_settings WHERE …']` → `[]`; step 12 `xff-7-3` `body_decoded_len`: `167` → `53`; step 12 `xff-7-3` `body_len`: `167` → `53`; step 12 `xff-7-3` `handler_ran`: `True` → `False`; step 12 `xff-7-3` `limiter_ip`: `` → `10.0.0.5`; step 12 `xff-7-3` `resolved_client_ip`: `7.7.7.7` → `10.0.0.5`; step 12 `xff-7-3` `response` — `content_length`: `167` → `53`; `retry_after`: `` → `3600`; `status`: `400` → `429`; step 13 `victim-1` `body_decoded_len`: `167` → `166`; step 13 `victim-1` `body_len`: `167` → `166`; step 13 `victim-1` `response` — `content_length`: `167` → `166` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `xff-honoured-trusted`, A vs `c1`: step 1 `xff-6-1` `body_decoded_len`: `167` → `166`; step 1 `xff-6-1` `body_len`: `167` → `166`; step 1 `xff-6-1` `resolved_client_ip`: `6.6.6.6` → `10.0.0.5`; step 1 `xff-6-1` `response` — `content_length`: `167` → `166`; step 2 `xff-7-1` `body_decoded_len`: `167` → `166`; step 2 `xff-7-1` `body_len`: `167` → `166`; step 2 `xff-7-1` `resolved_client_ip`: `7.7.7.7` → `10.0.0.5`; step 2 `xff-7-1` `response` — `content_length`: `167` → `166`; step 3 `xff-6-2` `body_decoded_len`: `167` → `166`; step 3 `xff-6-2` `body_len`: `167` → `166`; step 3 `xff-6-2` `resolved_client_ip`: `6.6.6.6` → `10.0.0.5`; step 3 `xff-6-2` `response` — `content_length`: `167` → `166`; step 4 `xff-7-2` `body_decoded_len`: `167` → `166`; step 4 `xff-7-2` `body_len`: `167` → `166`; step 4 `xff-7-2` `resolved_client_ip`: `7.7.7.7` → `10.0.0.5`; step 4 `xff-7-2` `response` — `content_length`: `167` → `166`; step 5 `peer-1` `body_decoded_len`: `167` → `166`; step 5 `peer-1` `body_len`: `167` → `166`; step 5 `peer-1` `response` — `content_length`: `167` → `166`; step 6 `xff-peer` `body_decoded_len`: `167` → `166`; step 6 `xff-peer` `body_len`: `167` → `166`; step 6 `xff-peer` `response` — `content_length`: `167` → `166`; step 7 `peer-2` `body_decoded_len`: `167` → `166`; step 7 `peer-2` `body_len`: `167` → `166`; step 7 `peer-2` `response` — `content_length`: `167` → `166`; step 8 `peer-3` `body_decoded_len`: `167` → `166`; step 8 `peer-3` `body_len`: `167` → `166`; step 8 `peer-3` `response` — `content_length`: `167` → `166`; step 9 `peer-4` `body_decoded_len`: `167` → `166`; step 9 `peer-4` `body_len`: `167` → `166`; step 9 `peer-4` `response` — `content_length`: `167` → `166`; step 10 `peer-5` `response` — `retry_after`: `3600` → `3599`; step 11 `xff-6-3` `body_decoded_len`: `167` → `166`; step 11 `xff-6-3` `body_len`: `167` → `166`; step 11 `xff-6-3` `resolved_client_ip`: `6.6.6.6` → `10.0.0.5`; step 11 `xff-6-3` `response` — `content_length`: `167` → `166`; step 12 `xff-7-3` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin request received for user: admin', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin creation failed: Invalid Input: Password must be at least 8 characters long', 'audit: 🔄 setup admin failed; released initialization claim so retries can proceed event="setup.claim_released" reason=Invalid Input: Password must be at least 8 characters long', 'http::api: client_error status=400 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=400']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=10.0.0.5 endpoint="register"', 'http::api: client_error status=429 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=429']`; step 12 `xff-7-3` `app_sql_statement_count`: `5` → `0`; step 12 `xff-7-3` `app_sql_statement_summaries`: `['SELECT value FROM auth.admin_settings …', 'INSERT INTO auth.admin_settings (key, …', 'SELECT id, username, email, …', 'SELECT id, username, email, …', 'DELETE FROM auth.admin_settings WHERE …']` → `[]`; step 12 `xff-7-3` `body_decoded_len`: `167` → `53`; step 12 `xff-7-3` `body_len`: `167` → `53`; step 12 `xff-7-3` `handler_ran`: `True` → `False`; step 12 `xff-7-3` `limiter_ip`: `` → `10.0.0.5`; step 12 `xff-7-3` `resolved_client_ip`: `7.7.7.7` → `10.0.0.5`; step 12 `xff-7-3` `response` — `content_length`: `167` → `53`; `retry_after`: `` → `3597`; `status`: `400` → `429`; step 13 `victim-1` `body_decoded_len`: `167` → `166`; step 13 `victim-1` `body_len`: `167` → `166`; step 13 `victim-1` `response` — `content_length`: `167` → `166` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `xff-honoured-trusted`, c0 vs `c1p`: step 1 `xff-6-1` `resolved_client_ip`: `6.6.6.6` → `10.0.0.5`; step 2 `xff-7-1` `resolved_client_ip`: `7.7.7.7` → `10.0.0.5`; step 3 `xff-6-2` `resolved_client_ip`: `6.6.6.6` → `10.0.0.5`; step 4 `xff-7-2` `resolved_client_ip`: `7.7.7.7` → `10.0.0.5`; step 11 `xff-6-3` `resolved_client_ip`: `6.6.6.6` → `10.0.0.5`; step 12 `xff-7-3` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin request received for user: admin', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin creation failed: Invalid Input: Password must be at least 8 characters long', 'audit: 🔄 setup admin failed; released initialization claim so retries can proceed event="setup.claim_released" reason=Invalid Input: Password must be at least 8 characters long', 'http::api: client_error status=400 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=400']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=10.0.0.5 endpoint="register"', 'http::api: client_error status=429 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=429']`; step 12 `xff-7-3` `app_sql_statement_count`: `5` → `0`; step 12 `xff-7-3` `app_sql_statement_summaries`: `['SELECT value FROM auth.admin_settings …', 'INSERT INTO auth.admin_settings (key, …', 'SELECT id, username, email, …', 'SELECT id, username, email, …', 'DELETE FROM auth.admin_settings WHERE …']` → `[]`; step 12 `xff-7-3` `body_decoded_len`: `166` → `53`; step 12 `xff-7-3` `body_len`: `166` → `53`; step 12 `xff-7-3` `handler_ran`: `True` → `False`; step 12 `xff-7-3` `limiter_ip`: `` → `10.0.0.5`; step 12 `xff-7-3` `resolved_client_ip`: `7.7.7.7` → `10.0.0.5`; step 12 `xff-7-3` `response` — `content_length`: `166` → `53`; `retry_after`: `` → `3600`; `status`: `400` → `429` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.
* `xff-honoured-trusted`, c0 vs `c1`: step 1 `xff-6-1` `resolved_client_ip`: `6.6.6.6` → `10.0.0.5`; step 2 `xff-7-1` `resolved_client_ip`: `7.7.7.7` → `10.0.0.5`; step 3 `xff-6-2` `resolved_client_ip`: `6.6.6.6` → `10.0.0.5`; step 4 `xff-7-2` `resolved_client_ip`: `7.7.7.7` → `10.0.0.5`; step 10 `peer-5` `response` — `retry_after`: `3600` → `3599`; step 11 `xff-6-3` `resolved_client_ip`: `6.6.6.6` → `10.0.0.5`; step 12 `xff-7-3` `app_log_lines`: `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin request received for user: admin', 'oxicloud::interfaces::api::handlers::auth_handler: Setup admin creation failed: Invalid Input: Password must be at least 8 characters long', 'audit: 🔄 setup admin failed; released initialization claim so retries can proceed event="setup.claim_released" reason=Invalid Input: Password must be at least 8 characters long', 'http::api: client_error status=400 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=400']` → `['tower_http::trace::on_request: started processing request', 'oxicloud::interfaces::middleware::rate_limit: Rate limit exceeded ip=10.0.0.5 endpoint="register"', 'http::api: client_error status=429 content_type="application/json" user_agent="curl/8.14.1"', 'tower_http::trace::on_response: finished processing request status=429']`; step 12 `xff-7-3` `app_sql_statement_count`: `5` → `0`; step 12 `xff-7-3` `app_sql_statement_summaries`: `['SELECT value FROM auth.admin_settings …', 'INSERT INTO auth.admin_settings (key, …', 'SELECT id, username, email, …', 'SELECT id, username, email, …', 'DELETE FROM auth.admin_settings WHERE …']` → `[]`; step 12 `xff-7-3` `body_decoded_len`: `166` → `53`; step 12 `xff-7-3` `body_len`: `166` → `53`; step 12 `xff-7-3` `handler_ran`: `True` → `False`; step 12 `xff-7-3` `limiter_ip`: `` → `10.0.0.5`; step 12 `xff-7-3` `resolved_client_ip`: `7.7.7.7` → `10.0.0.5`; step 12 `xff-7-3` `response` — `content_length`: `166` → `53`; `retry_after`: `` → `3597`; `status`: `400` → `429` — **verdict `unexplained`**; the mechanism is named in the case prose (§C.3/§C.4), this list is the raw difference.

## C.7 The reviewer's approval conditions for `0002`

Case list: `docs/re/scripts/run-approval-cases.sh` (same `lib-re.sh` plumbing,
same JSONL emitter and trim rule as `run-corpus.sh`). Three postures, because
cases 3 and 4 need short windows: release posture for cases 1–2
(`apv-<label>.jsonl`); `OXICLOUD_RATE_LIMIT_SETUP_{MAX=1,WINDOW_SECS=15}` **and**
`REGISTER_{MAX=1,WINDOW_SECS=15}` for case 3 (`apv-<label>-setupbudget.jsonl`);
the same pair of budgets for case 4 (`apv-<label>-setupdeadline.jsonl`), where the
register half is what makes ONE posture meaningful on both binaries — on c1 the
setup route runs on setup's own limiter, on A `SETUP_*` is inert and the register
limiter serves it, so A's row is the **old re-arming behaviour as a contrast**,
not a failure of the case.

Posture labelling, stated because the harness's `config` object cannot carry it
alone: `lib-re.sh::re_cfg_json` filters `env.sh` through an allow-list that
predates `0002`, so `OXICLOUD_RATE_LIMIT_SETUP_*` never appears in `config`.
The record therefore carries the posture **twice** — the allow-listed vars in
`config` (`REGISTER_MAX`/`REGISTER_WINDOW_SECS`, `RUST_LOG`) and all four posture
vars verbatim in `readings` (`posture_setup_max`, `posture_setup_window_secs`,
`posture_register_max`, `posture_register_window_secs`, plus
`config_from_env_sh`) — so no number below can be read without its posture.

| case | build | steps (status @ decoded bytes) | state after | file |
|---|---|---|---|---|
| `setup-validation-then-valid` | A | `pw7` 400@167, `valid-after-failure` 201@159 | users=1 owners=1 si_rows=1 si='true' | `evidence/apv-a.jsonl` |
| `setup-after-owner-across-restart` | A | `valid-1` 201@159, `after-restart` 403@224 | users=1 owners=1 si_rows=1 si='true' | `evidence/apv-a.jsonl` |
| `setup-validation-then-valid` | c1 | `pw7` 400@166, `valid-after-failure` 201@159 | users=1 owners=1 si_rows=1 si='true' | `evidence/apv-c1.jsonl` |
| `setup-after-owner-across-restart` | c1 | `valid-1` 201@159, `after-restart` 403@224 | users=1 owners=1 si_rows=1 si='true' | `evidence/apv-c1.jsonl` |
| `setup-get-spends-setup-budget` | A | `get-setup` 405@0, `post-after-get` 429@51, `register-probe` 429@51, `post-setup-again` 429@51 | users=0 owners=0 si_rows=0 si=— | `evidence/apv-a-setupbudget.jsonl` |
| `setup-get-spends-setup-budget` | c1 | `get-setup` 405@0, `post-after-get` 429@51, `register-probe` 201@140, `post-setup-again` 429@51 | users=1 owners=0 si_rows=0 si=— | `evidence/apv-c1-setupbudget.jsonl` |
| `setup-deadline-on-c1` | A | `t0-req1` 400@167, `t0-req2` 429@51, `t9-req3` 429@51, `t21-req4` 429@51, `t26-req5` 429@51 | users=0 owners=0 si_rows=0 si=— | `evidence/apv-a-setupdeadline.jsonl` |
| `setup-deadline-on-c1` | c1 | `t0-req1` 400@166, `t0-req2` 429@51, `t9-req3` 429@50, `t21-req4` 400@166, `t26-req5` 429@51 | users=0 owners=0 si_rows=0 si=— | `evidence/apv-c1-setupdeadline.jsonl` |

Reading each case:

* **`setup-validation-then-valid`** — one instance, **no DB reset between the two
  steps**. The 400 (handler ran, claim written and released) is followed by a 201
  with `users=1 owners=1 si_rows=1`, so the #677 rollback is shown *in sequence*,
  not as two independent halves. A: `400 → 201`; c1: `400 → 201` (body 167 vs 166,
  the `as_str()`→`wire_name()` byte).
* **`setup-after-owner-across-restart`** — 201, then the app is **restarted with the
  database intact** (only the process goes away), then `POST /api/setup` → 403
  (`224` B). The citable facts, in the order they carry weight: the step's
  **`handler_ran=True` with a non-zero `app_sql_statement_count`** (the refusal
  came from the handler reading the DB, not from middleware), `state_after` with
  **`users=1 owners=1 si_rows=1`**, and the fact that the process handling it is a
  **fresh one whose RAM limiter is empty** — so a spent-budget 429 cannot explain
  it. Corroboration only: the 403 carries **no `limiter_ip`**; absence is a one-way
  signal here (every one of the 225 429s measured carries one, but 60 non-429 steps
  carry one too, so presence proves nothing — absence is what is informative).
  The reading `restart_kept_db=by-design` is an **assertion by the script**, not a
  measurement: it records that no `re_db_reset` ran between the two steps; the state
  above is the evidence. A: `201 → 403`; c1: `201 → 403`.
* **`setup-get-spends-setup-budget`** — `SETUP_MAX=1`: `GET /api/setup` (405, no SQL,
  handler not run) then a POST. A: POST **429**, register probe **429** — the GET
  spent the one shared register budget. c1: POST **429**, register probe **201** —
  the GET spent one of **setup's** slots and register's budget is untouched, which
  is 0002's split made visible in the same case.

* **`setup-deadline-on-c1`** — the deadline condition on the **approved
  composition** (0001+0002), because `ttl-refresh`'s trace is c1l-only: with
  `SETUP_MAX=1`/`SETUP_WINDOW_SECS=15` on c1, the window opens on the first request
  and closes 15 s later, `ra` reports the **remaining** time (`15 → 6 → 10`) instead
  of the configured window, and the fourth request is **served (400)** because the
  window really closed. Same posture on A (register limiter serves the route there):
  `ra=15` on every rejection and no request served after the first — the old
  re-arming path, shown side by side rather than asserted.

**Open policy question (recorded, not changed).** A non-POST (`405`) request spends
a rate-limit slot on every build tested. Whether that is intended — a method the
route does not serve consuming a client's budget — is a policy decision, not a
defect this wave fixes; the behaviour is pinned here so changing it later is
deliberate.

### C.7.1 The four conditions satisfied by existing cases

Every `observed` cell below is **asserted** at generation time against the
artifact it cites — `gen-section-c.py::verify_citations()` re-derives each
status/`ra`/state value from `differential.json` or the `apv-*` records and
exits 2 (printing the failing citation) if any of them drifts. The rows stay
readable prose, but they can no longer survive a regeneration as a wrong
citation: this is the third instance of that class in the project, so the check
is structural rather than editorial.

| reviewer condition | case id (build) | observed |
|---|---|---|
| register exhausted → setup still has its own quota | `shared-budget` (c1) | `setup-after-register` = **400**, while A/B/c0/c1l/c1p answer 429 |
| setup exhausted → register unaffected | `shared-budget` (c1, reverse) | `register-after-setup` = **201** (`140` B); A/B/c0/c1l/c1p answer 429 |
| 6th setup request → 429 | `rate-limit-6x` (A, c0, c1l, c1p) | 5 × 400 then **429**, `Retry-After` 3600 (A/c0/c1p) or 3599 (c1l/c1); on **c1** the register-only posture no longer governs setup — the ≥6th 429 is shown by `concurrent-burst-rejects-8/16` on c1 (429 with `Retry-After` 3600 after the setup default of 5 is spent) |
| a 429 does not extend the deadline | **`setup-deadline-on-c1` (c1)** — the approved composition 0001+0002 | t0-req1 400 → t0-req2 429 ra=15 → t9-req3 429 ra=6 → t21-req4 400 → t26-req5 429 ra=10 — **the window closed on schedule** (`t21-req4` = 400, the client is let back in) and `ra` shrinks with elapsed time instead of staying at the configured window. Isolating evidence that 0001 alone is the change: `ttl-refresh` (c1l) t0-req1 400 → t0-req2 429 ra=15 → t9-req3 429 ra=6 → t21-req4 400 → t26-req5 429 ra=10, while `ttl-refresh` on c1 t0-req1 400 → t0-req2 400 → t9-req3 400 → t21-req4 400 → t26-req5 400 (the register-only posture does not govern setup there — see row 3) and A/B/c0/c1p re-arm with `ra`=15 throughout. Contrast, same posture on A where `SETUP_*` is inert and the register limiter serves the route: t0-req1 400 → t0-req2 429 ra=15 → t9-req3 429 ra=15 → t21-req4 429 ra=15 → t26-req5 429 ra=15 |
| concurrent setup → at most one owner | `concurrent-valid-2`, `-8`, `-8-unconfounded`, `-16-unconfounded` | exactly one 201 and `owners=1` on every build (`c0-burst.json` / `c1-burst.json` add 64-thread single-request trials at the limiter) |
| default stays 5/3600 per IP, separate counter | `config::setup_rate_limit_budget_tests::{default_is_five_per_hour, setup_env_vars_do_not_move_the_register_budget}` (unit) + the `OXICLOUD_RATE_LIMIT_SETUP_*` block in `0002`'s header | both tests `ok` on c1 (§C.1); the env names exist **only** in the patched tree |


