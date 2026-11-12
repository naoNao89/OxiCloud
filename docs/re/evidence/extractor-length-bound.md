# Can `POST /api/setup` return a **219-byte `400 text/plain`**?

Investigates the one hole left by `docs/re/runtime-evidence.md` §A.8/§A.10 and stated in
`docs/re/final-report.md` F1: the axum *extractor* rejections are `text/plain` bodies whose length
"depends on the submitted payload … measured instances are all far from 219, but that family is not
provably bounded".

**Answer: it *is* reachable.** A 219-byte `400` with `Content-Type: text/plain; charset=utf-8` was
produced on the released binary by `POST /api/setup`, from a 149-byte payload. The family is not
bounded below the request-body cap at all.

## 0. Verdict

| | |
|---|---|
| status of a 219-byte `text/plain` `400` from this endpoint | **reachable — measured** (`08-syntax-unknown-key-219-exact.json`, `Content-Length: 219`) |
| measured max of the `400`/`text/plain` family | **4176 bytes** (4096-byte key); growth is linear in the payload, i.e. **unbounded** up to the route's body limit |
| computed max, *fixed-detail* subfamily only (no input echo) | **142 bytes** at the route's 10 GiB cap (measured instance: 127) |
| computed max, whole family | `≈ body_limit + 78` ≈ **10.7 GB** at `DefaultBodyLimit::max(10 GiB)` — bounded only by the body cap |
| consequence for F1 | **F1 stays a candidate identification.** The "219 ⇒ must be the JSON envelope" step is now blocked by a counterexample, so the content-type stops carrying the argument; only capturing the operator's body bytes can promote F1. |
| what the fixtures 54/89/99/134 are | **instances, not constants** — three of them depend on the payload (see §5) |

Reproduced 219-byte body (byte-exact, `Content-Length: 219`, `sha256`
`64da5b2c6b14…` — full digest in the JSONL), payload
`{"` + 140 × `A` + `": @@@}`:

```
Failed to parse the request body as JSON: AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA: expected value at line 1 column 146
```

## 1. Artifacts and how to re-run

| file | contents |
|---|---|
| `docs/re/scripts/extractor-length-sweep.sh` | the sweep: boots a private instance (port 18087, DB `re_extractor`) through `lib-re.sh`, sends every payload, emits the JSONL |
| `docs/re/evidence/extractor-payloads/*.json` | the 15 payloads, byte-for-byte (SSOT — the JSONL carries each one's length and sha256) |
| `docs/re/evidence/extractor-length-sweep.jsonl` | one record per request: payload file/len/sha256, status, content-type, `Content-Length`, body len/sha256 and the **verbatim body** |
| this file | source quotes, arithmetic, the table, the verdict |

```sh
# on the sandbox (the released binary must be at $RE_BIN, lib-re.sh beside this script):
X_OUT=/data/re-scratch/out-extractor bash docs/re/scripts/extractor-length-sweep.sh
```

Boot notes that a re-run must reproduce:

* the 0.10.0 binary has **no `OXICLOUD_RATE_LIMIT_SETUP_*` knob** — `strings` over the release lists
  `OXICLOUD_RATE_LIMIT_REGISTER_MAX/_WINDOW_SECS` and no `…_SETUP_…`, and `config.rs`'s "setup gets its
  own budget" is a **HEAD-only** change. `/api/setup` therefore shares the **register** budget
  (5/3600 s), so the sweep sets `OXICLOUD_RATE_LIMIT_REGISTER_MAX=100000` to stay off the limiter;
* `re_boot` **refuses** to start if anything already answers on the port — kill a previous sweep first.

## 2. The alphabet, from the sources this binary was built from

Vendored tree: `/data/home/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`
(`axum-0.8.8`, `axum-core-0.5.6`, `serde_json-1.0.145|150|154`, `serde_path_to_error-0.1.20`).
HEAD's `Cargo.lock` pins axum 0.8.8 / axum-core 0.5.6 / serde_json 1.0.150; **the alphabet below is
identical in all three vendored serde_json copies** (compared mechanically: 23 message literals and the
same `Category::Syntax` arm set in each), so the release-vs-HEAD serde_json pin cannot change it.

### 2.1 Which rejection — which status and content-type

`axum-0.8.8/src/json.rs:174-190` picks the rejection from serde_json's own classification:

```rust
fn make_rejection(err: serde_path_to_error::Error<serde_json::Error>) -> JsonRejection {
    match err.inner().classify() {
        serde_json::error::Category::Data => JsonDataError::from_err(err).into(),
        serde_json::error::Category::Syntax | serde_json::error::Category::Eof => {
            JsonSyntaxError::from_err(err).into()
        }
        serde_json::error::Category::Io => { /* … in release: JsonSyntaxError */ }
    }
}
```
and `json.rs:192-201` wraps the trailing-garbage case in the *same* rejection:

```rust
serde_path_to_error::deserialize(&mut deserializer)
    .map_err(make_rejection)
    .and_then(|value| { deserializer.end().map(|()| Self(value))
        .map_err(|err| JsonSyntaxError::from_err(err).into()) })
```

`axum-0.8.8/src/extract/rejection.rs:10-33` fixes the statuses and the fixed first line of each body:

```rust
#[status = UNPROCESSABLE_ENTITY]
#[body = "Failed to deserialize the JSON body into the target type"]
pub struct JsonDataError(Error);
…
#[status = BAD_REQUEST]
#[body = "Failed to parse the request body as JSON"]
pub struct JsonSyntaxError(Error);
```

`axum-core-0.5.6/src/macros.rs:121-140` turns that into the wire body and the content type:

```rust
let body_text = self.body_text();
… (status, body_text).into_response()      // (StatusCode, String) ⇒ text/plain; charset=utf-8
…
impl std::fmt::Display for $name {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str($body)?;
        f.write_str(": ")?;
        self.0.fmt(f)
    }
}
```
`axum-core-0.5.6/src/error.rs:25-29` makes `self.0` print the inner error's `Display`.

**So the wire template is exactly**

```
400: "Failed to parse the request body as JSON: "        ← 42 bytes
   + [ <path> ": "  when the path is not "only unknown" ] ← serde_path_to_error::Display
   + <detail>                                            ← serde_json's message
   + " at line " L " column " C                          ← serde_json's ErrorImpl::Display
422: "Failed to deserialize the JSON body into the target type: "  ← 58 bytes + the same three parts
```

### 2.2 The detail — 23 fixed strings for a 400, input-carrying `Message(..)` for a 422

`serde_json-1.0.150/src/error.rs:54-84` is the whole selection rule:

```rust
match self.err.code {
    ErrorCode::Message(_) => Category::Data,
    ErrorCode::Io(_) => Category::Io,
    ErrorCode::EofWhileParsingList | … | ErrorCode::EofWhileParsingValue => Category::Eof,
    ErrorCode::ExpectedColon | … | ErrorCode::RecursionLimitExceeded => Category::Syntax,
}
```

`Message(msg)` is what `serde::de::Error::custom/invalid_type/unknown_field/…` builds (it carries the
input-derived text and is ≥1 byte long), and it maps to **Data ⇒ 422**, never to the 400 family.
The 400 family's detail therefore comes from a **closed set of 23 literals**
(`error.rs:349-391`, `ErrorCode::fmt`), and `error.rs:405-419` appends the position:

```rust
if self.line == 0 { Display::fmt(&self.code, f) }
else { write!(f, "{} at line {} column {}", self.code, self.line, self.column) }
```

| # | `ErrorCode` | detail literal | bytes |
|---|---|---|---|
| 1 | `ControlCharacterWhileParsingString` | `control character (\u0000-\u001F) found while parsing a string` | **62** |
| 2 | `ExpectedNumericKey` | `invalid value: expected key to be a number in quotes` | 52 |
| 3 | `FloatKeyMustBeFinite` | `float key must be finite (got NaN or +/-inf)` | 44 |
| 4 | `LoneLeadingSurrogateInHexEscape` | `lone leading surrogate in hex escape` | 36 |
| 5 | `UnexpectedEndOfHexEscape` | `unexpected end of hex escape` | 28 |
| 6 | `EofWhileParsingObject` | `EOF while parsing an object` | 27 |
| 7 | `EofWhileParsingString` | `EOF while parsing a string` | 26 |
| 8 | `InvalidUnicodeCodePoint` | `invalid unicode code point` | 26 |
| 9 | `EofWhileParsingValue` | `EOF while parsing a value` | 25 |
| 10 | `EofWhileParsingList` | `EOF while parsing a list` | 24 |
| 11 | `RecursionLimitExceeded` | `recursion limit exceeded` | 24 |
| 12 | `KeyMustBeAString` | `key must be a string` | 20 |
| 13 | `ExpectedListCommaOrEnd` | ``expected `,` or `]` `` | 19 |
| 14 | `ExpectedObjectCommaOrEnd` | ``expected `,` or `}` `` | 19 |
| 15 | `NumberOutOfRange` | `number out of range` | 19 |
| 16 | `TrailingCharacters` | `trailing characters` | 19 |
| 17 | `ExpectedSomeIdent` | `expected ident` | 14 |
| 18 | `ExpectedSomeValue` | `expected value` | 14 |
| 19 | `InvalidEscape` | `invalid escape` | 14 |
| 20 | `InvalidNumber` | `invalid number` | 14 |
| 21 | `TrailingComma` | `trailing comma` | 14 |
| 22 | `ExpectedColon` | ``expected `:` `` | 12 |
| 23 | `ExpectedDoubleQuote` | ``expected `"` `` | 12 |

**Longest fixed detail = 62 bytes.** (Rows 6-10 are `Category::Eof`, rows 1-5 and 11-23
`Category::Syntax`; `axum::Json` sends both to the same 400.) Nothing in this set echoes input —
which is why an analysis that stops here concludes "bounded". It is wrong, because of §2.3.
The fourth category is inert here: `Category::Io` can only arise from a reader, and `axum::Json`
parses a fully buffered `Bytes` (`json.rs:182-184`), so a 400 body never carries an I/O error string.

### 2.3 Where the payload enters the body: the `serde_path_to_error` path segment

`serde_path_to_error-0.1.20/src/lib.rs:119-126`:

```rust
impl<E: Display> Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if !self.path.is_only_unknown() { write!(f, "{}: ", self.path)?; }
        write!(f, "{}", self.original)
    }
}
```

A `Segment::Map { key }` is printed verbatim (`path.rs:146-155`), and the key is **the text that was
read out of the payload**, not the target struct's field name:

* `de.rs:1299-1305` — the visitor that captures it: `fn visit_str<E>(self, v: &str) { *self.key = Some(v.to_owned()); … }`
* `de.rs:1503-1518` — `next_key_seed` puts that captured string into `Chain::Map { parent: chain, key }`
* `de.rs:1521-1535` — `next_value_seed` carries that chain into the **value** deserialization
* `de.rs:1131-1142` — `deserialize_struct` delegates through `CaptureKey`, so a *struct* field's path
  segment is the same captured string

`SetupAdminDto` is a flat struct of three `String`s with **no `deny_unknown_fields`**
(`src/application/dtos/user_dto.rs:451-457`), so an unknown key is *ignored* — its value is parsed, and a
syntax error inside that value carries `Chain::Map { key: <the whole unknown key> }`. That is the
input-controlled term, and it is unbounded: `09-syntax-unknown-key-4096.json` echoes 4096 bytes of key.

`path.rs:141-144` (`is_only_unknown`) is why `{"a\x01b": 1}` prints **no** path: an error raised while
reading a *key* has no captured key yet (`Chain::NonStringKey` ⇒ `Segment::Unknown` ⇒ `"?"`, elided).

## 3. Arithmetic

The two fixed prefixes are 42 and 58 bytes (§2.1); `S(L,C)` = `" at line " + L + " column " + C`
= 17 + digits(L) + digits(C).

```
400 (path elided)      len = 42 + detail + S(L,C)
400 (path echoed, key) len = 42 + k + 2 + detail + S(L,C)      k = bytes of the input key
422                    len = 58 + [path + 2] + detail + S(L,C) detail ∈ Message(…) ∪ field-name list
```

Digit bound. The column is a byte offset inside the buffered body and the line counts newlines in it, so
with the route's limit `B = DefaultBodyLimit::max(10 * 1024 * 1024 * 1024)` = 10 737 418 240
(`src/main.rs:1417-1420`; 64-bit arm), `digits(C) ≤ 11` and `digits(L) ≤ 10`, i.e. `S ≤ 38`.
Measured support for "the limit is not axum's 2 MiB default": a **3 145 785-byte** body was parsed and
answered `400 … trailing characters at line 1 column 3145785` (86 bytes) — a 413 length refusal, which is
what a 2 MiB cap would have produced, did not occur.

| family | computed max | measured |
|---|---|---|
| 400, fixed detail, path elided | `42 + 62 + 38` = **142** | **127** (`15-…in-key-bigcol.json`, column 10003) |
| 400, path echoed | `≤ B + 78` ≈ **10.7 GB** (`k + 76 + digits(k+6)` with `k ≤ B − 9`) | **4176** (`09-…4096.json`) |
| 422 | `≤ B + ~94` (the path segment *and* a `Message(..)` can both carry input) | **219** (`10-…219-exact.json`) |
| 415 `MissingJsonContentType` | 54, constant (`"Expected request with \`Content-Type: application/json\`"`) | 54 (§A.8) |

**The 219-byte 400 is inside the reachable interval, so the 400 family is not bounded below 219.**
Adapting the ticket's arithmetic (`k + 76 + digits(k+6) = 219`) gives `k = 140` — and that is exactly the
payload that produced it.

Everything reproduces per case (§4), e.g. `42 + 140 + 2 + 14 + 21 = 219`,
`42 + 200 + 2 + 14 + 21 = 279`, `42 + 4096 + 2 + 14 + 22 = 4176`, `42 + 27 + 20 = 89`,
`58 + 21 + 20 = 99`, `58 + 8 + 2 + 46 + 20 = 134`.

## 4. Measured

All on the released binary (sha256 `9d364fdd…e080`), port 18087, DB `re_extractor`, identity-encoded
(`Content-Encoding` absent in every response), `Content-Length` equal to the bytes saved for every row.
`<n × 'A'>` renders a literal run of `n` bytes; the byte-exact bodies are in the JSONL.

| payload | bytes | status | content-type | `Content-Length` | body | body (rendered) |
|---|---|---|---|---|---|---|
| `01-syntax-eof-object.json` | 72 | 400 | `text/plain; charset=utf-8` | 89 | 89 | `Failed to parse the request body as JSON: EOF while parsing an object at line 1 column 72` |
| `02-syntax-control-char.json` | 59 | 400 | `text/plain; charset=utf-8` | 134 | 134 | `… JSON: username: control character (\u0000-\u001F) found while parsing a string at line 1 column 15` |
| `03-syntax-control-char-column-2014.json` | 2057 | 400 | `text/plain; charset=utf-8` | 136 | 136 | same detail, `… at line 1 column 2014` |
| `04-syntax-trailing-characters.json` | 58 | 400 | `text/plain; charset=utf-8` | 81 | 81 | `… JSON: trailing characters at line 1 column 58` |
| `05-syntax-trailing-characters-long-line.json` | 11 057 | 400 | `text/plain; charset=utf-8` | 84 | 84 | `… trailing characters at line 1 column 11057` |
| `06-syntax-number-out-of-range.json` | 18 | 400 | `text/plain; charset=utf-8` | 91 | 91 | `… JSON: username: number out of range at line 1 column 17` |
| `07-syntax-unknown-key-200.json` | 209 | 400 | `text/plain; charset=utf-8` | 279 | 279 | `… JSON: <200 × 'A'>: expected value at line 1 column 206` |
| **`08-syntax-unknown-key-219-exact.json`** | **149** | **400** | **`text/plain; charset=utf-8`** | **219** | **219** | `… JSON: <140 × 'A'>: expected value at line 1 column 146` |
| `09-syntax-unknown-key-4096.json` | 4105 | 400 | `text/plain; charset=utf-8` | 4176 | 4176 | `… JSON: <4096 × 'A'>: expected value at line 1 column 4102` |
| `10-data-bare-string-219-exact.json` | 89 | 422 | `text/plain; charset=utf-8` | 219 | 219 | `Failed to deserialize the JSON body into the target type: invalid type: string "<87 × 'A'>", expected struct SetupAdminDto at line 1 column 89` |
| `11-data-unknown-key-ignored.json` | 7 | 422 | `text/plain; charset=utf-8` | 101 | 101 | `… target type: missing field \`username\` at line 1 column 7` |
| `12-data-missing-field.json` | 42 | 422 | `text/plain; charset=utf-8` | 99 | 99 | `… target type: missing field \`email\` at line 1 column 42` |
| `13-data-invalid-type-integer.json` | 70 | 422 | `text/plain; charset=utf-8` | 134 | 134 | `… target type: username: invalid type: integer \`123\`, expected a string at line 1 column 15` |
| `14-syntax-control-char-in-key.json` | 10 | 400 | `text/plain; charset=utf-8` | 123 | 123 | `… JSON: control character (\u0000-\u001F) found while parsing a string at line 1 column 4` (no path) |
| `15-syntax-control-char-in-key-bigcol.json` | 10 009 | 400 | `text/plain; charset=utf-8` | 127 | 127 | same, `… at line 1 column 10003` (no path) |

Side measurement (payload generated on the fly — not committed, 3 MiB):

```
3 145 785-byte body  ->  400 text/plain; charset=utf-8, Content-Length: 86
Failed to parse the request body as JSON: trailing characters at line 1 column 3145785
```

Growth of the path-echo subfamily is linear and unbounded: key 140 → 219, 200 → 279, 4096 → 4176 bytes,
all `400 text/plain`.

**The rejections really are pre-handler.** In every one of the 15 per-step app-log windows the handler
marker `Setup admin request received` is absent (`grep -ac … → 0` for all 15). The 1–2 `sqlx::query`
lines that land in some of those windows are the background drains the corpus already documents
(`WITH drained AS …` for `storage.tree_etag_dirty`, `SELECT id, file_id, op FROM storage.search_index_dirty`),
not request SQL — they carry no request span. The DB was left pristine: the harness's own oracle reads
`re_state re_extractor` = `0|0|0||` (0 users, 0 owners, no `system_initialized` row).

## 5. Verdict, and what the old fixtures actually are

* **Reachable.** `08-syntax-unknown-key-219-exact.json` produces a 219-byte `400 text/plain;
  charset=utf-8` on this endpoint. Therefore **F1 remains a candidate**: "the endpoint answered 400 with
  219 bytes" no longer selects the `application/json` domain-validation envelope, because the
  `text/plain` extractor family can also hit 219 exactly. The `Content-Type` is no longer the deciding
  fact; only the bytes are.
* The *fixed-detail* half of the family is genuinely small — **≤ 142 bytes computed, 127 measured** — so
  the earlier intuition ("far from 219") is right for every body whose path is elided or "?"; it fails
  only where the echoed path segment is long.
* `docs/re/runtime-evidence.md` §A.8's `54 / 89 / 99 / 134` are **instances, not constants**:
  * `54` — 415 `MissingJsonContentType`, a *different* rejection with a constant body (always 54);
  * `89` — 400, reproduced here as case 01 (payload-dependent: the column moves with the payload);
  * `99` / `134` — 422 `JsonDataError`, reproduced as cases 12 / 13; the 422 detail carries input
    (`invalid type: integer \`123\``, the field list, the path) so its length is payload-dependent too
    — case 10 shows the same family at 219.
  Quoting them as if they were the family's `{54, 89, 99, 134}` (as F1's row does) is what made the
  family look small.
* Reaching 219 through the `text/plain` family needs a payload that is *malformed* (a syntax error
  inside an unknown key's value, or an error past a ≈140-byte prefix). The SPA's setup form submits
  well-formed `{"username","email","password"}` JSON, which cannot take that route — an argument about
  the client's payload shape, not a measurement of the operator's missing bytes. It is the strongest
  remaining support for F1, and it is deliberately *not* used to keep F1 above "candidate".

## 6. Residuals — what this does not settle

1. **The route's body cap was not driven to failure.** 10 GiB would need an upload this box cannot
   host (`/data` had <1 GB free). Measured: 3 145 785 bytes parsed fine (so axum's 2 MiB default is not
   in force). The 142-byte figure for the fixed-detail subfamily and the `≈ B + 78` figure for the rest
   are therefore *upper bounds parameterised by* `B = 10 GiB`; the reachability verdict at 219 depends
   only on the measured 149-byte payload and is independent of `B`.
2. **`BytesRejection` (also 400) was not exercised.** `LengthLimitError` is 413; `UnknownBodyError` is
   400 with the body `"Failed to buffer the request body: " + <boxed stream error>` — it needs a
   request-body stream failure, and its text is not echo of the payload. Unverified, not ruled out.
3. **The operator's 219-byte body is still uncaptured**, and production must not be POSTed (§A.23.1).
   This sweep narrows F1's evidence but cannot promote it.
4. **Release drift found while setting the sweep up:** 0.10.0 accepts no `OXICLOUD_RATE_LIMIT_SETUP_*`
   variable (setup shares the register budget); a HEAD build does. Anyone re-running against a source
   build should raise either knob, not both.
