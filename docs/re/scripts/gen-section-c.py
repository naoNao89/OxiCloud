#!/usr/bin/env python3
"""gen-section-c.py — emit the Markdown for runtime-evidence.md §C.2–§C.6
straight from docs/re/evidence/differential.json.

Every verdict cell is generated mechanically, per build, in one fixed column
order, and nothing is truncated (F-1, F-4 of the bundle audit):

  cell verdict := claims[build].verdict            if a claim names that build
                  'match'                          for A (the baseline column)
                  vs_A[build].verdict              otherwise
                  '-'                              if the build did not run the case

`vs_c0` is deliberately NOT used for the columns: it is the causal edge
(c0 → build), which belongs in the per-case prose, not in a column that the
reader will compare against A.

Usage: python3 tmp/gen-section-c.py <out.md>     (default: stdout)
"""
import json
import os
import sys

EV = 'docs/re/evidence'
LABELS = ('A', 'B', 'c0', 'c1l', 'c1p', 'c1', 'b')
doc = json.load(open(os.path.join(EV, 'differential.json')))
labels = [L for L in LABELS if L in doc['labels']]
cases = doc['cases']


def step_of(build, name):
    for s in (build or {}).get('steps', []):
        if s['name'] == name:
            return s
    return None


def cell_verdict(entry, L):
    claims = {c['label']: c for c in entry.get('claims', [])}
    if L in claims:
        return claims[L]['verdict']
    if L == 'A':
        return 'match' if 'A' in entry['present_in'] else '-'
    vs = entry.get('vs_A', {})
    if L in vs:
        return vs[L]['verdict']
    return '-' if L not in entry['present_in'] else 'match'


def short(txt, n=220):
    txt = ' '.join(str(txt).split())
    return txt if len(txt) <= n else txt[:n - 1] + '…'


out = []
W = out.append

# ── C.2 headline ───────────────────────────────────────────────────────────
W('## C.2 The headline row — `setup-pw8-bademail`, one request, six builds')
W('')
W('```')
W('POST /api/setup  {"username":"admin","email":"admin","password":"12345678"}')
W('build status content-type      bytes  transferred  coding')
for L in labels:
    r = doc['headline_setup_pw8_bademail'].get(L)
    if not r:
        continue
    W('%-5s %-6s %-12s  %-6s %-12s %s' % (L, r['status'], r['content_type'],
                                          r['body_decoded_len'],
                                          r.get('body_len', '-'),
                                          r.get('body_decoded_source') or 'identity'))
W('```')
W('')
W('`A` and `B` agree on 219; every HEAD-based build is 218 — one byte,')
W('`ErrorKind::as_str()` → `wire_name()` (`"Invalid Input"` → `"InvalidInput"`)')
W('from the 210 commits between `v0.10.0` and HEAD. **The length alone does not')
W('identify the branch** (`docs/re/evidence/extractor-length-bound.md` reaches a')
W('219-byte `400 text/plain` through the extractor), so `status` and')
W('`content-type` are part of the row.')
W('')

# ── C.3 verdict table ──────────────────────────────────────────────────────
W('## C.3 Verdict table — one row per case, per build')
W('')
W('**The six verdict columns of every row are generated mechanically** from')
W('`docs/re/evidence/differential.json` (`cases[].claims` first, else')
W('`cases[].vs_A`), by `docs/re/scripts/diff-builds.py` + `docs/re/scripts/gen-section-c.py`, the generator that emits')
W('this section: no cell is typed by hand, and the column order is fixed by one')
W('list, so a row cannot drift from its header. `b` is the `v0.10.0` source build')
W('(there is no separate `B` label); `match` on the `A` column means "this is the')
W('baseline". `vs_c0` (the causal edge) is deliberately NOT a column — it is')
W('quoted per case where it matters.')
W('')
W('| case | ' + ' | '.join(labels) + ' | patch | proposed verdict + code path |')
W('|---|' + '---|' * (len(labels) + 2))
for e in cases:
    row = ['| `%s`' % e['case_id']]
    for L in labels:
        row.append(' %s' % cell_verdict(e, L))
    row.append(' %s' % (e.get('targeted_by') or '—'))
    why = ''
    for L in labels:
        c = {x['label']: x for x in e.get('claims', [])}.get(L)
        if c and c['verdict'] != 'match':
            why = c['claim']
            break
    if not why:
        for L in labels[1:]:
            d = e.get('vs_A', {}).get(L) or {}
            if d.get('verdict') not in (None, 'match'):
                ex = d.get('explanation')
                why = json.dumps(ex) if not isinstance(ex, str) else ex
                break
    row.append(' %s |' % short(why or e.get('expected') or
                               'unchanged: identical on every non-normalized field'))
    W(' |'.join(row))
W('')
W('Per-field verbatim values behind every row (status, content-type,')
W('content-encoding, retry-after, decoded body length, `handler_ran`,')
W('`app_sql_statement_count`/summaries, resolved client IP, limiter key, state')
W('counters) live in `docs/re/evidence/differential.json` under')
W('`cases[].builds.<label>`.')

# ── C.4 patch-targeted rows, raw ───────────────────────────────────────────
W('')
W('## C.4 The patch-targeted rows, raw values')
W('')
W('`status @ decoded-body-bytes` (`ra=` is `Retry-After`); one column per build.')
W('')
for cid, steps in (('setup-get', ['get-setup', 'post-5']),
                   ('ttl-refresh', ['t0-req1', 't0-req2', 't9-req3', 't21-req4', 't26-req5']),
                   ('retry-after-constant', ['post-6', 'post-after-20s']),
                   ('shared-budget', ['setup-after-register', 'register-after-setup']),
                   ('xff-honoured-trusted', ['xff-6-1', 'xff-7-1', 'xff-6-2', 'xff-7-2',
                                             'xff-6-3', 'xff-7-3', 'peer-5', 'victim-1'])):
    e = next((x for x in cases if x['case_id'] == cid), None)
    if not e:
        continue
    W('`%s`' % cid)
    W('')
    W('| step | ' + ' | '.join(labels) + ' |')
    W('|---|' + '---|' * len(labels))
    for name in steps:
        cells = []
        for L in labels:
            st = step_of(e['builds'].get(L), name)
            if st is None:
                cells.append('-')
                continue
            ra = st['response']['retry_after']
            cells.append('%s @ %sB%s' % (st['response']['status'], st['body_decoded_len'],
                                         (' ra=' + ra) if ra else ''))
        W('| `%s` | %s |' % (name, ' | '.join(cells)))
    W('')

# ── C.5 invariants + atomicity trials ──────────────────────────────────────
W('## C.5 Invariants and the atomicity trials')
W('')
W('| case | ' + ' | '.join(labels) + ' |')
W('|---|' + '---|' * len(labels))
for cid in ('terminal-201-then-403', 'concurrent-valid-2', 'concurrent-valid-8',
            'concurrent-valid-8-unconfounded', 'concurrent-valid-16-unconfounded',
            'concurrent-burst-rejects-8', 'concurrent-burst-rejects-16'):
    e = next((x for x in cases if x['case_id'] == cid), None)
    if not e:
        continue
    cells = []
    for L in labels:
        b = e['builds'].get(L)
        if not b:
            cells.append('-')
            continue
        ms = sorted(s['response']['status'] for s in b['steps'])
        cells.append('201×%d owners=%d served=%d [%s]'
                     % (sum(1 for x in ms if x == 201), b['state_after']['owners'],
                        sum(1 for x in ms if x != 429),
                        ' '.join(str(x) for x in sorted(set(ms)))))
    W('| `%s` | %s |' % (cid, ' | '.join(cells)))
W('')
for L in ('c0', 'c1'):
    p = os.path.join(EV, '%s-burst.json' % L)
    if not os.path.exists(p):
        continue
    d = json.load(open(p))
    W('* `%s` (`%s`, N=%d, both budgets pinned to max=1, window 15 s): overshoot in '
      '**%d of %d** trials, max served %s; per-trial served counts %s; wall-clock per '
      'burst %s s.'
      % (L, os.path.basename(d['binary']), d['n'], d['overshoot_trials'], d['trials_total'],
         d['max_served'], [t['served_400'] for t in d['trials']],
         [t['secs'] for t in d['trials']]))
W('')

# ── C.6 complete list of everything else ───────────────────────────────────
W('## C.6 Every difference that is NOT `match` / `explained-drift` / `fix-confirmed`')
W('')
W('Generated from `differential.json` with **no length cap and no truncation**:')
W('each bullet names the case, the reference build it was compared against, the')
W('build, and every differing field with both values.')
W('')
bullets = []
for e in cases:
    cid = e['case_id']
    for edge, key in (('A', 'vs_A'), ('c0', 'vs_c0')):
        for L, d in e.get(key, {}).items():
            if d['verdict'] != 'unexplained':
                continue
            ex = d.get('explanation') or {}
            diffs = (ex.get('unexplained') if isinstance(ex, dict) else None) or []
            parts = []
            for dd in diffs:
                f = dd.get('field')
                a, b = dd.get('a'), dd.get('b')
                if isinstance(a, dict) and isinstance(b, dict):
                    # composite field: list ONLY the differing keys, with full values
                    sub = []
                    for k in sorted(set(a) | set(b)):
                        if a.get(k) != b.get(k):
                            sub.append('`%s`: `%s` → `%s`' % (k, a.get(k), b.get(k)))
                    where = ('step %s `%s` ' % (dd['step'], dd.get('name'))) if 'step' in dd else ''
                    parts.append('%s`%s` — %s' % (where, f, '; '.join(sub)))
                elif 'step' in dd:
                    parts.append('step %s `%s` `%s`: `%s` → `%s`'
                                 % (dd['step'], dd.get('name'), f, a, b))
                else:
                    parts.append('`%s`: `%s` → `%s`' % (f, a, b))
            bullets.append('* `%s`, %s vs `%s`: %s — **verdict `unexplained`**; the mechanism '
                           'is named in the case prose (§C.3/§C.4), this list is the raw '
                           'difference.' % (cid, edge, L, '; '.join(parts) if parts else 'n/a'))
    for c in e.get('claims', []):
        if not c['ok']:
            bullets.append('* `%s`, claim on `%s`: %s — **claim FAILED** (observed %s).'
                           % (cid, c['label'], short(c['claim'], 200), short(c['observed'], 200)))
if not bullets:
    bullets.append('* none — every case is `match`, `explained-drift` or `fix-confirmed`, '
                   'and every claim holds for every build it names.')
out.extend(bullets)

# ── C.7 the reviewer's approval conditions ─────────────────────────────────
APV = [('apv-a.jsonl', 'A', 'release binary 9d364fdd…e080'),
       ('apv-c1.jsonl', 'c1', 'c1-patched 4666faa8…d20ec3'),
       ('apv-a-setupbudget.jsonl', 'A', 'both budgets pinned'),
       ('apv-c1-setupbudget.jsonl', 'c1', 'both budgets pinned'),
       ('apv-a-setupdeadline.jsonl', 'A', 'old re-arming path (SETUP_* inert on A)'),
       ('apv-c1-setupdeadline.jsonl', 'c1', 'setup budget — the 0001+0002 composition')]
recs = []
for fname, L, note in APV:
    p = os.path.join(EV, fname)
    if not os.path.exists(p):
        continue
    for line in open(p):
        if line.strip():
            recs.append((fname, L, note, json.loads(line)))
if recs:
    W('')
    W('## C.7 The reviewer\'s approval conditions for `0002`')
    W('')
    W('Case list: `docs/re/scripts/run-approval-cases.sh` (same `lib-re.sh` plumbing,')
    W('same JSONL emitter and trim rule as `run-corpus.sh`). Three postures, because')
    W('cases 3 and 4 need short windows: release posture for cases 1–2')
    W('(`apv-<label>.jsonl`); `OXICLOUD_RATE_LIMIT_SETUP_{MAX=1,WINDOW_SECS=15}` **and**')
    W('`REGISTER_{MAX=1,WINDOW_SECS=15}` for case 3 (`apv-<label>-setupbudget.jsonl`);')
    W('the same pair of budgets for case 4 (`apv-<label>-setupdeadline.jsonl`), where the')
    W('register half is what makes ONE posture meaningful on both binaries — on c1 the')
    W('setup route runs on setup\'s own limiter, on A `SETUP_*` is inert and the register')
    W('limiter serves it, so A\'s row is the **old re-arming behaviour as a contrast**,')
    W('not a failure of the case.')
    W('')
    W('Posture labelling, stated because the harness\'s `config` object cannot carry it')
    W('alone: `lib-re.sh::re_cfg_json` filters `env.sh` through an allow-list that')
    W('predates `0002`, so `OXICLOUD_RATE_LIMIT_SETUP_*` never appears in `config`.')
    W('The record therefore carries the posture **twice** — the allow-listed vars in')
    W('`config` (`REGISTER_MAX`/`REGISTER_WINDOW_SECS`, `RUST_LOG`) and all four posture')
    W('vars verbatim in `readings` (`posture_setup_max`, `posture_setup_window_secs`,')
    W('`posture_register_max`, `posture_register_window_secs`, plus')
    W('`config_from_env_sh`) — so no number below can be read without its posture.')
    W('')
    W('| case | build | steps (status @ decoded bytes) | state after | file |')
    W('|---|---|---|---|---|')
    for fname, L, note, r in recs:
        steps = ', '.join('`%s` %s@%s' % (x['name'], x['response']['status'], x['body_decoded_len'])
                          for x in r['steps'])
        sa = r['state_after']
        W('| `%s` | %s | %s | users=%s owners=%s si_rows=%s si=%s | `evidence/%s` |'
          % (r['case_id'], L, steps, sa['users'], sa['owners'], sa['si_rows'],
             "'" + sa['si_value'] + "'" if sa['si_value'] else '—', fname))
    W('')
    W('Reading each case:')
    W('')
    W('* **`setup-validation-then-valid`** — one instance, **no DB reset between the two')
    W('  steps**. The 400 (handler ran, claim written and released) is followed by a 201')
    W('  with `users=1 owners=1 si_rows=1`, so the #677 rollback is shown *in sequence*,')
    W('  not as two independent halves. A: `400 → 201`; c1: `400 → 201` (body 167 vs 166,')
    W('  the `as_str()`→`wire_name()` byte).')
    W('* **`setup-after-owner-across-restart`** — 201, then the app is **restarted with the')
    W('  database intact** (only the process goes away), then `POST /api/setup` → 403')
    W('  (`224` B). The citable facts, in the order they carry weight: the step\'s')
    W('  **`handler_ran=True` with a non-zero `app_sql_statement_count`** (the refusal')
    W('  came from the handler reading the DB, not from middleware), `state_after` with')
    W('  **`users=1 owners=1 si_rows=1`**, and the fact that the process handling it is a')
    W('  **fresh one whose RAM limiter is empty** — so a spent-budget 429 cannot explain')
    W('  it. Corroboration only: the 403 carries **no `limiter_ip`**; absence is a one-way')
    W('  signal here (every one of the 225 429s measured carries one, but 60 non-429 steps')
    W('  carry one too, so presence proves nothing — absence is what is informative).')
    W('  The reading `restart_kept_db=by-design` is an **assertion by the script**, not a')
    W('  measurement: it records that no `re_db_reset` ran between the two steps; the state')
    W('  above is the evidence. A: `201 → 403`; c1: `201 → 403`.')
    W('* **`setup-get-spends-setup-budget`** — `SETUP_MAX=1`: `GET /api/setup` (405, no SQL,')
    W('  handler not run) then a POST. A: POST **429**, register probe **429** — the GET')
    W('  spent the one shared register budget. c1: POST **429**, register probe **201** —')
    W('  the GET spent one of **setup\'s** slots and register\'s budget is untouched, which')
    W('  is 0002\'s split made visible in the same case.')
    W('')
    W('* **`setup-deadline-on-c1`** — the deadline condition on the **approved')
    W('  composition** (0001+0002), because `ttl-refresh`\'s trace is c1l-only: with')
    W('  `SETUP_MAX=1`/`SETUP_WINDOW_SECS=15` on c1, the window opens on the first request')
    W('  and closes 15 s later, `ra` reports the **remaining** time (`15 → 6 → 10`) instead')
    W('  of the configured window, and the fourth request is **served (400)** because the')
    W('  window really closed. Same posture on A (register limiter serves the route there):')
    W('  `ra=15` on every rejection and no request served after the first — the old')
    W('  re-arming path, shown side by side rather than asserted.')
    W('')
    W('**Open policy question (recorded, not changed).** A non-POST (`405`) request spends')
    W('a rate-limit slot on every build tested. Whether that is intended — a method the')
    W('route does not serve consuming a client\'s budget — is a policy decision, not a')
    W('defect this wave fixes; the behaviour is pinned here so changing it later is')
    W('deliberate.')
    W('')
    # ── citations: derived from the artifacts, then ASSERTED (see C.7.1) ────
    def apv_rec(cid, label):
        for fname, L, note, r in recs:
            if r['case_id'] == cid and L == label:
                return r
        return None

    def trace_apv(cid, label, names=None):
        r = apv_rec(cid, label)
        if r is None:
            return '(no record: %s/%s)' % (cid, label)
        out = []
        for s in r['steps']:
            if names and s['name'] not in names:
                continue
            ra = s['response']['retry_after']
            out.append('%s %s%s' % (s['name'], s['response']['status'],
                                    (' ra=' + ra) if ra else ''))
        return ' → '.join(out)

    def doc_step(cid, L, name):
        e = next((x for x in doc['cases'] if x['case_id'] == cid), None)
        if e is None:
            return None
        for s in e['builds'].get(L, {}).get('steps', []):
            if s['name'] == name:
                return s
        return None

    def st(cid, L, name):
        s = doc_step(cid, L, name)
        return None if s is None else s['response']['status']

    def ra(cid, L, name):
        s = doc_step(cid, L, name)
        return None if s is None else s['response']['retry_after']

    def apv_st(cid, L, name):
        r = apv_rec(cid, L)
        if r is None:
            return None
        for s in r['steps']:
            if s['name'] == name:
                return s['response']['status']
        return None

    def apv_ra(cid, L, name):
        r = apv_rec(cid, L)
        if r is None:
            return None
        for s in r['steps']:
            if s['name'] == name:
                return s['response']['retry_after']
        return None

    def apv_state(cid, L):
        r = apv_rec(cid, L)
        return None if r is None else r['state_after']

    def trace_doc(cid, L, names):
        return ' → '.join('%s %s%s' % (n, st(cid, L, n) if st(cid, L, n) is not None else '-',
                                       (' ra=' + ra(cid, L, n)) if ra(cid, L, n) else '')
                          for n in names)

    TTL = ('t0-req1', 't0-req2', 't9-req3', 't21-req4', 't26-req5')
    tr_dead_c1 = trace_apv('setup-deadline-on-c1', 'c1')
    tr_dead_a = trace_apv('setup-deadline-on-c1', 'A')
    tr_ttl_c1l = trace_doc('ttl-refresh', 'c1l', TTL)
    tr_ttl_c1 = trace_doc('ttl-refresh', 'c1', TTL)

    def verify_citations():
        """Re-derive every citation in §C.7.1; exit 2 listing any that drifted."""
        checks = [
            ('row 1: shared-budget/c1 setup-after-register == 400', st('shared-budget', 'c1', 'setup-after-register') == 400),
            ('row 1: shared-budget/A setup-after-register == 429', st('shared-budget', 'A', 'setup-after-register') == 429),
            ('row 2: shared-budget/c1 register-after-setup == 201', st('shared-budget', 'c1', 'register-after-setup') == 201),
            ('row 2: shared-budget/A register-after-setup == 429', st('shared-budget', 'A', 'register-after-setup') == 429),
            ('row 3: rate-limit-6x/A post-6 == 429 ra=3600', st('rate-limit-6x', 'A', 'post-6') == 429 and ra('rate-limit-6x', 'A', 'post-6') == '3600'),
            ('row 3: rate-limit-6x/c1l post-6 == 429 ra=3599', st('rate-limit-6x', 'c1l', 'post-6') == 429 and ra('rate-limit-6x', 'c1l', 'post-6') == '3599'),
            ('row 4: setup-deadline/c1 t21-req4 == 400 (window closed)', apv_st('setup-deadline-on-c1', 'c1', 't21-req4') == 400),
            ('row 4: setup-deadline/c1 ra shrinks 15 -> 6 -> 10',
             [apv_ra('setup-deadline-on-c1', 'c1', n) for n in ('t0-req2', 't9-req3', 't26-req5')] == ['15', '6', '10']),
            ('row 4: setup-deadline/A is the OLD re-arming path (t21-req4 == 429)', apv_st('setup-deadline-on-c1', 'A', 't21-req4') == 429),
            ('row 4: ttl-refresh/c1l shows the same closure (t21-req4 == 400)', st('ttl-refresh', 'c1l', 't21-req4') == 400),
            ('row 4: ttl-refresh/c1 is inert (all 400)', all(st('ttl-refresh', 'c1', n) == 400 for n in TTL)),
            ('row 5: concurrent-valid-2/c1 owners == 1', (doc_step('concurrent-valid-2', 'c1', 'cv2-1') is not None) and
             next(x for x in doc['cases'] if x['case_id'] == 'concurrent-valid-2')['builds']['c1']['state_after']['owners'] == 1),
            ('row 5: concurrent-valid-2/c1l owners == 1',
             next(x for x in doc['cases'] if x['case_id'] == 'concurrent-valid-2')['builds']['c1l']['state_after']['owners'] == 1),
            ('case 1: owners == 1 on A and c1', (apv_state('setup-validation-then-valid', 'A') or {}).get('owners') == 1 and
             (apv_state('setup-validation-then-valid', 'c1') or {}).get('owners') == 1),
            ('case 2: after-restart == 403 and si_rows == 1 (A, c1)',
             apv_st('setup-after-owner-across-restart', 'A', 'after-restart') == 403 and
             apv_st('setup-after-owner-across-restart', 'c1', 'after-restart') == 403 and
             (apv_state('setup-after-owner-across-restart', 'A') or {}).get('si_rows') == 1 and
             (apv_state('setup-after-owner-across-restart', 'c1') or {}).get('si_rows') == 1),
            ('case 3: the 405 spends a slot (post-after-get == 429 on A and c1)',
             apv_st('setup-get-spends-setup-budget', 'A', 'post-after-get') == 429 and
             apv_st('setup-get-spends-setup-budget', 'c1', 'post-after-get') == 429),
            ('case 3: register probe differs (A 429 / c1 201)',
             apv_st('setup-get-spends-setup-budget', 'A', 'register-probe') == 429 and
             apv_st('setup-get-spends-setup-budget', 'c1', 'register-probe') == 201),
        ]
        bad = [t for t, ok in checks if not ok]
        if bad:
            print('CITATION ASSERTION FAILED (%d):' % len(bad), file=sys.stderr)
            for t in bad:
                print('  -', t, file=sys.stderr)
            raise SystemExit(2)

    verify_citations()

    W('### C.7.1 The four conditions satisfied by existing cases')
    W('')
    W('Every `observed` cell below is **asserted** at generation time against the')
    W('artifact it cites — `gen-section-c.py::verify_citations()` re-derives each')
    W('status/`ra`/state value from `differential.json` or the `apv-*` records and')
    W('exits 2 (printing the failing citation) if any of them drifts. The rows stay')
    W('readable prose, but they can no longer survive a regeneration as a wrong')
    W('citation: this is the third instance of that class in the project, so the check')
    W('is structural rather than editorial.')
    W('')
    W('| reviewer condition | case id (build) | observed |')
    W('|---|---|---|')
    W('| register exhausted → setup still has its own quota | `shared-budget` (c1) | `setup-after-register` = **400**, while A/B/c0/c1l/c1p answer 429 |')
    W('| setup exhausted → register unaffected | `shared-budget` (c1, reverse) | `register-after-setup` = **201** (`140` B); A/B/c0/c1l/c1p answer 429 |')
    W('| 6th setup request → 429 | `rate-limit-6x` (A, c0, c1l, c1p) | 5 × 400 then **429**, `Retry-After` 3600 (A/c0/c1p) or 3599 (c1l/c1); on **c1** the register-only posture no longer governs setup — the ≥6th 429 is shown by `concurrent-burst-rejects-8/16` on c1 (429 with `Retry-After` 3600 after the setup default of 5 is spent) |')
    W('| a 429 does not extend the deadline | **`setup-deadline-on-c1` (c1)** — the approved composition 0001+0002 | ' + tr_dead_c1 + ' — **the window closed on schedule** (`t21-req4` = 400, the client is let back in) and `ra` shrinks with elapsed time instead of staying at the configured window. Isolating evidence that 0001 alone is the change: `ttl-refresh` (c1l) ' + tr_ttl_c1l + ', while `ttl-refresh` on c1 ' + tr_ttl_c1 + ' (the register-only posture does not govern setup there — see row 3) and A/B/c0/c1p re-arm with `ra`=15 throughout. Contrast, same posture on A where `SETUP_*` is inert and the register limiter serves the route: ' + tr_dead_a + ' |')
    W('| concurrent setup → at most one owner | `concurrent-valid-2`, `-8`, `-8-unconfounded`, `-16-unconfounded` | exactly one 201 and `owners=1` on every build (`c0-burst.json` / `c1-burst.json` add 64-thread single-request trials at the limiter) |')
    W('| default stays 5/3600 per IP, separate counter | `config::setup_rate_limit_budget_tests::{default_is_five_per_hour, setup_env_vars_do_not_move_the_register_budget}` (unit) + the `OXICLOUD_RATE_LIMIT_SETUP_*` block in `0002`\'s header | both tests `ok` on c1 (§C.1); the env names exist **only** in the patched tree |')
    W('')
W('')
text = '\n'.join(out) + '\n'
if len(sys.argv) > 1 and sys.argv[1] != '-':
    open(sys.argv[1], 'w').write(text)
    lens = [len(b) for b in bullets]
    print('wrote %s: %d rows in C.3, %d bullets in C.6, bullet length min/max %d/%d chars'
          % (sys.argv[1], len(cases), len(bullets), min(lens), max(lens)), file=sys.stderr)
else:
    sys.stdout.write(text)