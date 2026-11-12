# A — case table (generated from the JSONL, one row per case)

`statuses`, `body bytes`, `handler`, `sql` are **per step, in order**;
`handler` is `Y`/`n` per step; `sql` is the number of sqlx queries logged
inside that request's own span (the per-request DB-I/O oracle).
`state after` = `users/owners/system_initialized-rows/updated_by(8)`.

| case | postures | statuses | body bytes | decoded | handler | sql | state after |
|---|---|---|---|---|---|---|---|
| `setup-pw7` | register 5/3600 s (release), trust none (release) | 400 | 167 | - | `Y` | 5 | 0/0/0/- |
| `setup-pw8-bademail` | register 5/3600 s (release), trust none (release) | 400 | 219 | - | `Y` | 5 | 0/0/0/- |
| `setup-email-nodot` | register 5/3600 s (release), trust none (release) | 400 | 251 | - | `Y` | 5 | 0/0/0/- |
| `setup-email-forbidden` | register 5/3600 s (release), trust none (release) | 400/400 | 259,151 | 259 | `YY` | 5,5 | 0/0/0/- |
| `setup-username-badchars` | register 5/3600 s (release), trust none (release) | 400/400 | 317,167 | 317 | `YY` | 5,5 | 0/0/0/- |
| `setup-json-missing-field` | register 5/3600 s (release), trust none (release) | 422 | 99 | - | `n` | 0 | 0/0/0/- |
| `setup-json-wrong-type` | register 5/3600 s (release), trust none (release) | 422 | 134 | - | `n` | 0 | 0/0/0/- |
| `setup-json-syntax-error` | register 5/3600 s (release), trust none (release) | 400 | 89 | - | `n` | 0 | 0/0/0/- |
| `setup-content-type-text` | register 5/3600 s (release), trust none (release) | 415 | 54 | - | `n` | 0 | 0/0/0/- |
| `setup-content-type-absent` | register 5/3600 s (release), trust none (release) | 415 | 54 | - | `n` | 0 | 0/0/0/- |
| `setup-get` | register 5/3600 s (release), trust none (release) | 405/400/400/400/400/429 | 0,167,167,167,167,53 | - | `nYYYYn` | 0,5,5,5,5,0 | 0/0/0/- |
| `rate-limit-6x` | register 5/3600 s (release), trust none (release) | 400/400/400/400/400/429 | 167,167,167,167,167,53 | - | `YYYYYn` | 5,5,5,5,5,0 | 0/0/0/- |
| `shared-budget` | register 5/3600 s (release), trust none (release) | 201/409/409/409/409/429/400/400/400/400/400/429 | 140,129,129,129,129,53,167,167,167,167,167,53 | - | `nnnnnnYYYYYn` | 19,2,2,2,2,0,5,5,5,5,5,0 | 0/0/0/- |
| `xff-ignored-untrusted` | register 5/3600 s (release), trust none (release) | 400/400/400/400/400/429 | 167,167,167,167,167,53 | - | `YYYYYn` | 5,5,5,5,5,0 | 0/0/0/- |
| `terminal-201-then-403` | register 5/3600 s (release), trust none (release) | 201/403/200 | 159,224,86 | - | `YYn` | 22,1,2 | 1/1/1/fb19bedd |
| `concurrent-valid-2` | register 5/3600 s (release), trust none (release) | 403/201 | 224,159 | - | `YY` | 2,22 | 1/1/1/97c9e23b |
| `concurrent-valid-8` | register 5/3600 s (release), trust none (release) | 201/429/403/403/429/403/429/403 | 159,53,224,224,53,224,53,224 | - | `YnYYnYnY` | 22,0,1,1,0,1,0,1 | 1/1/1/88e9f9d3 |
| `retry-after-constant` | register 5/3600 s (release), trust none (release) | 400/400/400/400/400/429/429 | 167,167,167,167,167,53,53 | - | `YYYYYnn` | 5,5,5,5,5,0,0 | 0/0/0/- |
| `xff-honoured-trusted` | register 5/3600 s (release), trust 127.0.0.1/32 | 400/400/400/400/400/400/400/400/400/429/400/400/400 | 167,167,167,167,167,167,167,167,167,53,167,167,167 | - | `YYYYYYYYYnYYY` | 5,5,5,5,5,5,5,5,5,0,5,5,5 | 0/0/0/- |
| `ttl-refresh` | register 1/15 s, trust none (release) | 400/429/429/429/429 | 167,51,51,51,51 | - | `Ynnnn` | 5,0,0,0,0 | 0/0/0/- |
| `ttl-refresh-control` | register 1/15 s, trust none (release) | 400/400 | 167,167 | - | `YY` | 5,5 | 0/0/0/- |
| `concurrent-burst-rejects-8` | register 1/15 s, trust none (release) | 429/429/429/429/429/429/400/429 | 51,51,51,51,51,51,167,51 | - | `nnnnnnYn` | 0,0,0,0,0,0,5,0 | 0/0/0/- |
| `concurrent-burst-rejects-16` | register 1/15 s, trust none (release) | 400/429/429/429/429/429/429/429/429/429/429/429/429/429/429/429 | 167,51,51,51,51,51,51,51,51,51,51,51,51,51,51,51 | - | `Ynnnnnnnnnnnnnnn` | 5,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0 | 0/0/0/- |
| `concurrent-valid-8-unconfounded` | register 100/3600 s, trust none (release) | 403/403/403/403/403/201/403/403 | 224,224,224,224,224,159,224,224 | - | `YYYYYYYY` | 1,1,1,1,1,22,1,1 | 1/1/1/d6a312a8 |
| `concurrent-valid-16-unconfounded` | register 100/3600 s, trust none (release) | 403/403/403/403/403/403/201/403/403/403/403/403/403/403/403/403 | 224,224,224,224,224,224,159,224,224,224,224,224,224,224,224,224 | - | `YYYYYYYYYYYYYYYY` | 1,1,1,1,1,1,22,1,1,1,1,1,1,1,1,1 | 1/1/1/690d56cf |
| `compression-bracket` | register 100/3600 s, trust none (release) | 415/400/400/400/400/400/400 | 54,89,167,219,251,151,167 | 259,317 | `nnYYYYY` | 0,0,5,5,5,5,5 | 0/0/0/- |

## notes recorded with each case

| case | note |
|---|---|
| `setup-pw7` | 7-char password: expect 400, init stays false, claim not left behind |
| `setup-pw8-bademail` | 8-char pw, email 'admin' (no @): expect 400. CANDIDATE branch for the operator-observed 219-byte body |
| `setup-email-nodot` | email admin@localhost: expect 400 |
| `setup-email-forbidden` | email 'a b@example.test' > 256-byte raw body: record whether the response is gzip-compressed and the transferred size |
| `setup-username-badchars` | username 'ad min' > 256-byte raw body: compression threshold probe |
| `setup-json-missing-field` | body without email: expect 422 (axum Json extractor) |
| `setup-json-wrong-type` | username as number: expect 422 |
| `setup-json-syntax-error` | truncated JSON: expect 400 |
| `setup-content-type-text` | valid JSON body but Content-Type: text/plain: expect 415 |
| `setup-content-type-absent` | valid JSON body, Content-Type header removed: expect 415 |
| `setup-get` | GET /api/setup (405) and whether it consumed a register slot |
| `rate-limit-6x` | six setup-pw7 POSTs: 5 pass the limiter (400), the 6th must be 429 with Retry-After |
| `shared-budget` | exhaust with register then setup -> 429; then re_fresh and the reverse direction |
| `xff-ignored-untrusted` | empty trust list: X-Forwarded-For must not split the bucket |
| `terminal-201-then-403` | valid setup -> 201; repeat -> 403 SystemAlreadyInitialized; state must hold exactly one owner |
| `concurrent-valid-2` | N=2 simultaneous valid setup POSTs on a fresh DB |
| `concurrent-valid-8` | N=8 simultaneous valid setup POSTs, release posture (3 of them may be 429: documented confound) |
| `retry-after-constant` | register 5/3600: consume the budget, sleep 20 s, hit again — Retry-After must still read 3600, not ~3580 |
| `xff-honoured-trusted` | trust 127.0.0.1/32: two distinct leftmost XFF hops must not share a bucket; XFF=127.0.0.1 must share the bare-peer bucket |
| `ttl-refresh` | max=1 window=15 s: rejected requests keep pushing the deadline forward |
| `ttl-refresh-control` | control: same posture, request, idle one full window (17 s), request -> allowed (the window really expires when idle) |
| `concurrent-burst-rejects-8` | max=1: N=8 simultaneous REJECTION posts — how many get served (400) vs 429 |
| `concurrent-burst-rejects-16` | max=1: N=16 simultaneous rejection posts (second sample of the same race) |
| `concurrent-valid-8-unconfounded` | register max=100: N=8 simultaneous VALID setups — expect exactly one 201 and one owner row |
| `concurrent-valid-16-unconfounded` | register max=100: N=16 simultaneous VALID setups — expect exactly one 201 and one owner row |
| `compression-bracket` | Accept-Encoding: gzip on every 400 branch: raw vs transferred length pairs |
