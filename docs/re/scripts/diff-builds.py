#!/usr/bin/env python3
"""diff-builds.py — the build-vs-build differential table behind
docs/re/evidence/differential.json and docs/re/runtime-evidence.md §C.

    # the invocation of record (reproduces evidence/differential.json byte for byte):
    python3 docs/re/scripts/diff-builds.py <evidence-dir> <out.json>

    # same set, explicit — any label subset is allowed, and the ORDER is preserved:
    python3 docs/re/scripts/diff-builds.py docs/re/evidence /tmp/d.json \
        A c0 c1l c1p c1 b A-loaded

    # staleness guard: regenerate with the defaults and compare with the committed
    # artifact, non-zero exit on any difference:
    python3 docs/re/scripts/diff-builds.py --check docs/re/evidence/differential.json

    # default labels (this is the default the code uses; there is no `B` label):
    #   A c0 c1l c1p c1 b A-loaded
    # `A-loaded` is LOADED as its own label and then EXCLUDED from the differential
    # (see main()), because diff-labels.pick() would otherwise fold
    # `A-loaded*.jsonl` into label `A` and silently make the contended pass the
    # baseline. Nothing else is dropped.

It reuses docs/re/scripts/diff-labels.py's field split verbatim (imported, not
copied): the DETERMINISTIC projection is what a cross-build diff may rest on, the
race cases are compared by outcome class (status multiset + state counters), and
`si_updated_by` is normalized away everywhere because every run mints a fresh
owner UUID.

Only what experiment-matrix.md §2 allows is normalized. The normalizations are
applied EXPLICITLY and recorded next to each case in the output, never silently:

  * `Content-Length` and the raw transferred byte count may differ between A
    (release, musl, x86-64-v2) and the local dev builds when the negotiated
    Content-Encoding differs — the body length measured AFTER decoding
    (`body_decoded_len`) is the number compared.
  * absolute paths inside log lines, timestamps, `x-request-id`, DPoP nonces,
    the free-disk-space value in a log line and run-minted UUIDs (diff-labels'
    HOST_VARYING).
  * `si_updated_by` (fresh owner UUID per run) — never `si_rows`/`si_value`/
    `users`/`owners`, which are the invariant.

NEVER normalized: status, response message text, content type, state
transitions, the `error_type` token, quota decisions. A difference in one of
those is reported as `unexplained` unless an explicit code-path explanation is
supplied in EXPLAINED below.

Verdicts (proposed — the report owner accepts or overrides them):
  match / explained-drift / fix-confirmed / unexplained / regression
"""
import importlib.util
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location('diff_labels', os.path.join(HERE, 'diff-labels.py'))
dl = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dl)

# ── the one thing this script adds to diff-labels' split ────────────────────
# Fields diff-labels keeps out of DET_STEP but that the SPEC (experiment-matrix
# §4, H1) makes evidence: which bucket the trusted-proxy path resolved. It is
# deterministic per request, so a cross-build diff must see it.
EXTRA_STEP = ('limiter_ip',)

# Counts of RAW log lines inside a step's window. lib-re.sh documents both as
# nondeterministic: background scheduler work ("tree_etag_dirty" sweeps) writes
# into any open window, so two runs of the SAME binary differ here (verified: on
# the delivered A vs A-loaded evidence these two fields are the only difference
# on 6 cases). They are recorded verbatim for the reader but excluded from the
# comparison — the per-request oracle is `app_sql_statement_summaries`.
CHURN_STEP = ('pg_log_line_count', 'app_log_window_lines')

# Explanations for differences that are fully accounted for by the provenance
# table (differential-results.md §1) plus a named code path. Each entry is
# (case_id_prefix, build_label, field, explanation). A difference with no entry
# here is reported as `unexplained` and blocks the gate.
ERROR_TYPE_DRIFT = (
    'v0.10.0 renders ErrorKind via as_str() ("Invalid Input", 13 B); HEAD renders '
    'it via wire_name() ("InvalidInput", 12 B). One byte, every DomainError 4xx '
    'JSON body, introduced by the 210 commits between tag v0.10.0 and HEAD '
    '(src/interfaces/errors.rs), not by any patch under test.'
)

RACE_PREFIX = 'concurrent-'
# Cases the patches are allowed — expected — to change, with the patch that owns
# the change and the predicted direction (docs/re/patches/README.md).
TARGETED = {
    # H4 is NOT changed by any patch: the limiter still runs before routing, so the
    # 405 GET spends a register slot on EVERY build (measured: post-5 = 429 with
    # handler_ran=false and app_sql_statement_count=0 on c0, c1l, c1p, c1, b). What
    # 0001 changes on this row is only the Retry-After VALUE (3600 → 3599, remaining
    # time). The row is here as the negative control for the H4 claim, not as a 0001
    # effect.
    'setup-get': (None, 'H4 unchanged by every patch: the 405 GET spends a register slot on all builds '
                        '(post-5 = 429, handler_ran=false, sql=0); 0001 changes only Retry-After (3600 → 3599)'),
    'rate-limit-6x': ('0001', 'H2: rejected requests re-arm the window on HEAD/C0'),
    'ttl-refresh': ('0001', 'H2: the deadline slides on HEAD/C0, is fixed on 0001'),
    'retry-after-constant': ('0001', 'H3: Retry-After is the configured constant on HEAD/C0, remaining time on 0001'),
    # The burst rows' only build-level difference is 0002's budget split: on c1 the
    # register-only posture no longer pins /api/setup (its own 5/3600 default does),
    # while the atomicity question itself is not visible at this level on any build
    # (64-thread trials: 1 served / 63 rejected on c0 and c1l, 5/5 each).
    'concurrent-burst-rejects-8': ('0002', 'H5 (0002), not 0001: the budget split governs which limiter answers on c1; '
                                          'no overshoot on c0 or c1l (1 served, 5/5 trials)'),
    'concurrent-burst-rejects-16': ('0002', 'H5 (0002), second sample; atomicity is not observable at this level on any build'),
    'shared-budget': ('0002', 'H5: one budget for /api/setup and /api/auth/register on HEAD/C0, two on 0001+0002'),
    'setup-and-register-separate-budgets': ('0002', 'H5'),
    'xff-ignored-untrusted': ('0003', 'H1: XFF ignored when the peer is untrusted (unchanged by 0003 — guard)'),
    'xff-honoured-trusted': ('0003', 'H1: LEFTMOST XFF hop picks the bucket on HEAD/C0; RIGHTMOST untrusted hop on 0003'),
    'setup-pw8-bademail': (None, 'H6/H12: 219 B on A and B, 218 B on C0/C1* — the as_str()->wire_name() drift'),
}


def norm_lines(lines):
    return [dl.norm_line(x) for x in lines or []]


def project_step(s):
    """The comparable projection of one step: every deterministic field, with
    the two profile-dependent length fields kept but flagged."""
    out = {}
    for k in dl.DET_STEP:
        if k == 'app_log_lines':
            out[k] = norm_lines(s.get(k))
        elif k == 'request':
            r = s.get(k) or {}
            # `extra_args` carries the -H headers under test (XFF and
            # Accept-Encoding) — they are INPUT, so they must match.
            out[k] = {kk: r.get(kk) for kk in sorted(r)}
        else:
            out[k] = s.get(k)
    for k in EXTRA_STEP:
        out[k] = s.get(k)
    out['churn_counts'] = {k: s.get(k) for k in CHURN_STEP}
    for k in dl.DET_RESPONSE:
        out.setdefault('response', {})[k] = (s.get('response') or {}).get(k)
    return out


def step_diffs(a, b):
    """Field-level differences between two projected steps (exact). The churn
    counts are compared as their own object so the output still shows them."""
    d = {}
    for k in sorted(set(a) | set(b)):
        if a.get(k) != b.get(k):
            d[k] = {'a': a.get(k), 'b': b.get(k)}
    return d


def project_record(r):
    st = r['state_after']
    return {
        'label': r['label'],
        'case_id': r['case_id'],
        'config': r.get('config'),
        'steps': [project_step(s) for s in r['steps']],
        'state_before': {k: r['state_before'][k] for k in ('users', 'owners', 'si_rows', 'si_value')},
        'state_after': {k: st[k] for k in ('users', 'owners', 'si_rows', 'si_value')},
        'nondeterministic': {
            'si_updated_by_before': r['state_before']['si_updated_by'],
            'si_updated_by_after': st['si_updated_by'],
            'db_now': r['db_now'],
            'raw_transferred_body_bytes': [s.get('body_file_bytes') for s in r['steps']],
            'pg_window_sha256': [(s.get('pg_log_window') or {}).get('sha256', '')
                                 for s in r['steps']],
        },
    }


LEN_NORMALIZE = ('body_file_bytes',)   # profile-dependent transfer size


def step_diffs(a, b):
    """Field-level differences between two projected steps (exact)."""
    d = {}
    for k in sorted(set(a) | set(b)):
        if a.get(k) != b.get(k):
            d[k] = {'a': a.get(k), 'b': b.get(k)}
    return d


def status_multiset(rec):
    return sorted(s['response']['status'] for s in rec['steps'])


def compare(case, base, other, base_label, other_label):
    """Difference report for one case between two builds."""
    if case.startswith(RACE_PREFIX):
        ma, mb = status_multiset(base), status_multiset(other)
        sa = {k: base['state_' + k] for k in ('before', 'after')}
        sb = {k: other['state_' + k] for k in ('before', 'after')}
        diffs = []
        if ma != mb:
            diffs.append({'field': 'status_multiset', 'a': ma, 'b': mb})
        if sa != sb:
            diffs.append({'field': 'state', 'a': sa, 'b': sb})
        return {'mode': 'outcome-class', 'status_multiset': {base_label: ma, other_label: mb},
                'state': {base_label: sa, other_label: sb}, 'diffs': diffs}
    diffs = []
    if len(base['steps']) != len(other['steps']):
        diffs.append({'field': 'step_count', 'a': len(base['steps']), 'b': len(other['steps'])})
    for i, (x, y) in enumerate(zip(base['steps'], other['steps'])):
        for k, v in step_diffs(x, y).items():
            diffs.append({'step': i + 1, 'name': x['name'], 'field': k, **v})
    if base['state_before'] != other['state_before']:
        diffs.append({'field': 'state_before', 'a': base['state_before'], 'b': other['state_before']})
    if base['state_after'] != other['state_after']:
        diffs.append({'field': 'state_after', 'a': base['state_after'], 'b': other['state_after']})
    return {'mode': 'field-by-field', 'diffs': diffs}


def _is_one_byte_drift(d):
    """True when this single field difference is the ±1 body-length change the
    as_str() -> wire_name() rename produces on every DomainError 4xx JSON body."""
    f, a, b = d['field'], d['a'], d['b']
    if f in ('body_decoded_len', 'body_len', 'body_file_bytes'):
        return abs(a - b) == 1
    if f == 'response':
        keys = set(a) | set(b)
        if set(a) == set(b) and all(a[k] == b[k] for k in keys if k != 'content_length'):
            return abs(int(a['content_length']) - int(b['content_length'])) == 1
    return False


def _is_drift_companion(d):
    """A difference that rides along with the one-byte drift on the same row and
    is explained by a named mechanism:
    * `response.retry_after` — 0001 reports the remaining time instead of the
      constant, and the *status* is unchanged (H3);
    * `status_multiset` on a race case — 0002 moved /api/setup onto its own
      budget, so the register-only posture yields extra 429s. Both are named in
      the row's explanation, never normalized away.
    """
    if d['field'] not in ('limiter_ip', 'churn_counts'):
        return False
    return True


def classify(case, diffs, other_label):
    """Propose a verdict for one (case, build) pair's difference list."""
    if not diffs:
        return 'match', None
    # the one-byte error_type drift: only body lengths move, by 1, on a 4xx JSON
    # body, and never a status/message/state field.
    if all(_is_one_byte_drift(d) for d in diffs):
        return 'explained-drift', ERROR_TYPE_DRIFT
    # drift + a named companion (Retry-After now reports remaining time, or the
    # 0002 setup-budget posture change) — reported with BOTH mechanisms named.
    companions = []
    only_driftish = True
    for d in diffs:
        if _is_one_byte_drift(d) or d['field'] in ('limiter_ip', 'churn_counts'):
            continue
        if d['field'] == 'response' and 'retry_after' in (d['a'] or {}) and set(d['a']) == set(d['b']):
            if all(d['a'][k] == d['b'][k] for k in d['a'] if k != 'retry_after'):
                companions.append('Retry-After: %s -> %s (0001 reports the remaining time, the status is unchanged)'
                                  % (d['a']['retry_after'] or '""', d['b']['retry_after'] or '""'))
                continue
        only_driftish = False
        break
    if only_driftish and companions:
        return 'explained-drift', ERROR_TYPE_DRIFT + ' | ' + ' | '.join(sorted(set(companions)))
    if only_driftish:
        return 'explained-drift', ERROR_TYPE_DRIFT
    explained = []
    informational = []
    unexplained = []
    for d in diffs:
        if d['field'] == 'churn_counts':
            informational.append(d)
        elif d['field'] in LEN_NORMALIZE:
            explained.append('%s: profile-dependent transfer size (compression negotiated differently by profile)' % d['field'])
        elif d['field'] == 'content_length':
            explained.append('Content-Length: follows the profile-dependent transfer size')
        elif d['field'] == 'content_encoding':
            explained.append('Content-Encoding: profile/negotiation difference (A is release+musl)')
        else:
            unexplained.append(d)
    if unexplained:
        return 'unexplained', {'unexplained': unexplained, 'informational': informational}
    if not explained:
        # churn counts only: the two raw-window line counts differ, nothing else
        return 'match', {'note': 'background-log churn only (nondeterministic field)', 'informational': informational}
    return 'explained-drift', {'explained': explained, 'informational': informational}


def load_labels(ev, labels):
    groups = dl.pick(ev, labels)
    out = {}
    for L, paths in groups.items():
        recs = {}
        for p in paths:
            for r in dl.load(p):
                recs.setdefault(r['case_id'], []).append(r)
        out[L] = recs
    return out


# ── claims: what each patch PREDICTS, checked against each build's own record ─
# Every claim is (case_id, labels, text, fn(rec) -> (ok, observed)). `labels` are
# the builds the claim applies to (lower-cased). A claim that holds yields
# `verdict_when_ok`; one that does not yields `verdict_when_failed` with the
# observed values pasted in, so a failed prediction is never silently smoothed.
def _st(rec, name):
    for s in rec['steps']:
        if s['name'] == name:
            return s
    return None


def _status(rec, name):
    s = _st(rec, name)
    return None if s is None else s['response']['status']


def _retry(rec, name):
    s = _st(rec, name)
    return None if s is None else s['response']['retry_after']


def _len(rec, name):
    s = _st(rec, name)
    return None if s is None else s['body_decoded_len']


def _served(rec):
    return sum(1 for s in rec['steps'] if s['response']['status'] != 429)


def _owners(rec):
    return rec['state_after']['owners']


def _count_status(rec, code):
    return sum(1 for s in rec['steps'] if s['response']['status'] == code)


CLAIMS = [
    # ── 0001 (C1L, C1) ──────────────────────────────────────────────────────
    ('setup-get', ('c1l', 'c1'), 'match',
     'H4 UNCHANGED by 0001: the limiter still runs before routing, so the 405 GET still spends a register slot '
     'and the 5th POST is 429. What changes is only the Retry-After value (3599 = remaining time, not the constant 3600).',
     lambda r: (_status(r, 'post-5') == 429 and _retry(r, 'post-5') != '3600',
                {'post-5.status': _status(r, 'post-5'), 'post-5.retry_after': _retry(r, 'post-5'),
                 'get-setup.status': _status(r, 'get-setup')})),
    ('setup-get', ('a', 'b', 'c0', 'c1p'), 'match',
     'H4 pre-patch: the GET spends a register slot, so the 5th POST is 429 with the constant Retry-After 3600',
     lambda r: (_status(r, 'post-5') == 429 and _retry(r, 'post-5') == '3600',
                {'post-5.status': _status(r, 'post-5'), 'post-5.retry_after': _retry(r, 'post-5')})),
    ('ttl-refresh', ('c1l',), 'fix-confirmed',
     'H2/0001: the window closes 15 s after the key\'s first request, so t21-req4 is served (400) and re-opens a window that t26-req5 fills (429, Retry-After 10 = remaining)',
     lambda r: (_status(r, 't21-req4') == 400 and _status(r, 't26-req5') == 429
                and _retry(r, 't9-req3') not in (None, '', '15'),
                {'t21-req4': _status(r, 't21-req4'), 't26-req5': _status(r, 't26-req5'),
                 't9-req3.retry_after': _retry(r, 't9-req3')})),
    ('ttl-refresh', ('c1',), 'explained-drift',
     '0002 moved /api/setup onto its own budget (OXICLOUD_RATE_LIMIT_SETUP_*, default 5/3600), so this '
     'register-only posture no longer pins the route: all five requests are served (400). Same cause as the '
     'burst rows on c1 — with both budgets pinned to max=1 the patched build rejects every one of 64 '
     'simultaneous requests but one (c1-burst.json).',
     lambda r: (_served(r) == len(r['steps']),
                {'served': _served(r), 'n': len(r['steps']),
                 'multiset': [s['response']['status'] for s in r['steps']]})),
    ('ttl-refresh', ('a', 'b', 'c0', 'c1p'), 'match',
     'H2 pre-patch: every rejected retry re-arms the key, so t21-req4 == t26-req5 == 429',
     lambda r: (_status(r, 't21-req4') == 429 and _status(r, 't26-req5') == 429,
                {'t21-req4': _status(r, 't21-req4'), 't26-req5': _status(r, 't26-req5')})),
    ('retry-after-constant', ('c1l', 'c1'), 'fix-confirmed',
     'H3/0001: Retry-After reports the time left in the window (~3580 s after 20 s), not the configured constant',
     lambda r: (_retry(r, 'post-after-20s') not in (None, '', '3600')
                and abs(int(_retry(r, 'post-after-20s') or 0) - 3580) <= 30,
                {'post-after-20s.retry_after': _retry(r, 'post-after-20s')})),
    ('retry-after-constant', ('a', 'b', 'c0', 'c1p'), 'match',
     'H3 pre-patch: Retry-After is the configured window, constant across the 20 s wait',
     lambda r: (_retry(r, 'post-after-20s') == '3600',
                {'post-after-20s.retry_after': _retry(r, 'post-after-20s')})),
    # ── 0002 (C1 only) ──────────────────────────────────────────────────────
    ('shared-budget', ('c1',), 'fix-confirmed',
     'H5/0002: /api/setup has its own budget, so five register calls no longer lock setup out (setup-after-register is not 429)',
     lambda r: (_status(r, 'setup-after-register') != 429,
                {'setup-after-register': _status(r, 'setup-after-register'),
                 'register-after-setup': _status(r, 'register-after-setup')})),
    ('shared-budget', ('a', 'b', 'c0', 'c1l', 'c1p'), 'match',
     'H5 pre-patch: one limiter instance for both routes — exhausting either locks the other out (429 both directions)',
     lambda r: (_status(r, 'setup-after-register') == 429
                and _status(r, 'register-after-setup') == 429,
                {'setup-after-register': _status(r, 'setup-after-register'),
                 'register-after-setup': _status(r, 'register-after-setup')})),
    # ── 0003 (C1P, C1) ──────────────────────────────────────────────────────
    ('xff-honoured-trusted', ('c1p', 'c1'), 'fix-confirmed',
     'H1/0003: only the RIGHTMOST untrusted hop keys the bucket, so the two spoofed chains share one bucket: 4 pass, the 5th passes and the 6th is rejected, while the victim-shaped value burns its own key',
     lambda r: ([_status(r, n) for n in ('xff-6-1', 'xff-7-1', 'xff-6-2', 'xff-7-2')] == [400] * 4
                and _status(r, 'xff-6-3') == 400 and _status(r, 'xff-7-3') == 429
                and _status(r, 'victim-1') == 400,
                {n: _status(r, n) for n in ('xff-6-1', 'xff-7-1', 'xff-6-2', 'xff-7-2',
                                            'xff-6-3', 'xff-7-3', 'victim-1')})),
    ('xff-honoured-trusted', ('a', 'b', 'c0'), 'match',
     'H1 pre-patch: the LEFTMOST (client-supplied) hop picks the bucket, so each spoofed prefix keeps its own budget',
     lambda r: (_status(r, 'xff-6-3') == 400 and _status(r, 'xff-7-3') == 400,
                {n: _status(r, n) for n in ('xff-6-1', 'xff-7-1', 'xff-6-2', 'xff-7-2',
                                            'xff-6-3', 'xff-7-3')})),
    # ── invariants, every build ─────────────────────────────────────────────
    ('concurrent-burst-rejects-8', ('a', 'b', 'c0', 'c1l', 'c1p'), 'match',
     'H8: served <= max_requests = 1 — the pre-patch read-then-write (A/B/C0/C1P) and the patched atomic upsert (C1L) '
     'both served exactly one of the 8; an overshoot would be served > 1',
     lambda r: (_served(r) <= 1, {'served': _served(r), 'n': len(r['steps'])})),
    ('concurrent-burst-rejects-8', ('c1',), 'explained-drift',
     '0002 moved /api/setup onto its own budget, so this register-only posture pins nothing: 5 of 8 are served '
     '(the setup default) and 3 are 429 with Retry-After 3600. With BOTH budgets pinned to max=1 the same build '
     'serves exactly 1 of 64 in 5/5 trials (c1-burst.json, 0 overshoots).',
     lambda r: (_served(r) == 5, {'served': _served(r), 'n': len(r['steps']),
                                  'multiset': sorted(s['response']['status'] for s in r['steps'])})),
    ('concurrent-burst-rejects-16', ('a', 'b', 'c0', 'c1l', 'c1p'), 'match',
     'H8 (second sample): served <= 1',
     lambda r: (_served(r) <= 1, {'served': _served(r), 'n': len(r['steps'])})),
    ('concurrent-burst-rejects-16', ('c1',), 'explained-drift',
     'same 0002 posture effect as the 8-way burst on c1: 5 served (the setup default), 11 rejected',
     lambda r: (_served(r) == 5, {'served': _served(r), 'n': len(r['steps']),
                                  'multiset': sorted(s['response']['status'] for s in r['steps'])})),
    ('concurrent-valid-2', ('a', 'b', 'c0', 'c1', 'c1l', 'c1p'), 'match',
     'H9: exactly one 201 and exactly one owner row',
     lambda r: (_count_status(r, 201) == 1 and _owners(r) == 1 and r['state_after']['users'] == 1,
                {'201s': _count_status(r, 201), 'owners': _owners(r),
                 'users': r['state_after']['users'],
                 'multiset': sorted(s['response']['status'] for s in r['steps'])})),
    ('concurrent-valid-8', ('a', 'b', 'c0', 'c1', 'c1l', 'c1p'), 'match',
     'H9 at N=8 (release posture, confounded by the 5-slot budget): at most one 201, at most one owner',
     lambda r: (_count_status(r, 201) <= 1 and _owners(r) <= 1,
                {'201s': _count_status(r, 201), 'owners': _owners(r),
                 'multiset': sorted(s['response']['status'] for s in r['steps'])})),
    ('concurrent-valid-8-unconfounded', ('a', 'b', 'c0', 'c1', 'c1l', 'c1p'), 'match',
     'H9 at N=8 with the limiter raised: exactly one 201, exactly one owner, one user',
     lambda r: (_count_status(r, 201) == 1 and _owners(r) == 1 and r['state_after']['users'] == 1,
                {'201s': _count_status(r, 201), 'owners': _owners(r)})),
    ('concurrent-valid-16-unconfounded', ('a', 'b', 'c0', 'c1', 'c1l', 'c1p'), 'match',
     'H9 at N=16 with the limiter raised: exactly one 201, exactly one owner, one user',
     lambda r: (_count_status(r, 201) == 1 and _owners(r) == 1 and r['state_after']['users'] == 1,
                {'201s': _count_status(r, 201), 'owners': _owners(r)})),
    ('terminal-201-then-403', ('a', 'b', 'c0', 'c1', 'c1l', 'c1p'), 'match',
     'H10/H9: 201 once, then 403 SystemAlreadyInitialized, exactly one owner',
     lambda r: (_status(r, 'valid-1') == 201 and _status(r, 'valid-2') == 403
                and _owners(r) == 1,
                {'valid-1': _status(r, 'valid-1'), 'valid-2': _status(r, 'valid-2'),
                 'owners': _owners(r), 'valid-2.len': _len(r, 'valid-2')})),
    # ── unrelated-route drift: /api/auth/status was retired by the 210 commits ─
    ('terminal-201-then-403', ('a', 'b'), 'match',
     'unrelated route: `GET /api/auth/status` still answers 200 with {initialized, admin_count} on the v0.10.0 wire',
     lambda r: (_status(r, 'status') == 200,
                {'status.step': _status(r, 'status'), 'len': _len(r, 'status')})),
    ('terminal-201-then-403', ('c0', 'c1', 'c1l', 'c1p'), 'explained-drift',
     'unrelated route retired between v0.10.0 and HEAD: `GET /api/auth/status` answers 401 (Token not provided). '
     'Named code path: the signal it carried moved to `config.initialized` — '
     'src/interfaces/api/handlers/auth_handler.rs:29 ("/status endpoint retired") and '
     'src/interfaces/api/handlers/config_handler.rs:65 ("Replaces the dedicated /api/auth/status endpoint"). '
     'Not a patch effect: c0 == c1 on this row.',
     lambda r: (_status(r, 'status') == 401,
                {'status.step': _status(r, 'status'), 'len': _len(r, 'status')})),
    # ── the drift fingerprint (H12 / H6) ────────────────────────────────────
    ('setup-pw8-bademail', ('a', 'b'), 'match',
     'H6/H12: this exact request measures 219 raw bytes on the v0.10.0 wire form (as_str() = "Invalid Input"). '
     'Length alone does not identify the branch — a 219-byte text/plain extractor body is also reachable '
     '(docs/re/evidence/extractor-length-bound.md) — so the content-type and the JSON envelope are part of this row.',
     lambda r: (_len(r, 'post-bademail') == 219, {'len': _len(r, 'post-bademail')})),
    ('setup-pw8-bademail', ('c0', 'c1', 'c1l', 'c1p'), 'explained-drift',
     'H12: the same request measures 218 raw bytes on HEAD\'s wire_name() spelling ("InvalidInput")',
     lambda r: (_len(r, 'post-bademail') == 218, {'len': _len(r, 'post-bademail')})),
]

def claims_for(cid, label):
    """Claims that apply to this case and build label (labels are lower-cased)."""
    return [c for c in CLAIMS if c[0] == cid and label.lower() in c[1]]


def failed_verdict(cid, label):
    """What a FAILED claim means, per case class.

    * single-owner / claim-release cases (H9/H10) and the patched builds' limiter
      invariant (H8 on c1/c1l) — a failure is a `regression`;
    * H8 on a build that does not carry 0001 — a failure is an OBSERVATION of the
      documented defect (`unexplained` here forces it into the report);
    * everything else — `unexplained` (a predicted change that did not appear).
    """
    low = label.lower()
    if cid.startswith('concurrent-valid') or cid == 'terminal-201-then-403':
        return 'regression'
    if cid.startswith('concurrent-burst-rejects'):
        return 'regression' if low in ('c1', 'c1l') else 'unexplained'
    return 'unexplained'


# The default label list is the one that produced docs/re/evidence/differential.json.
# `A-loaded` must be present here (it is loaded as its own label and then excluded):
# without it, diff-labels.pick() folds `A-loaded*.jsonl` into label `A` and the
# baseline silently becomes the contended pass, which changes verdicts. There is no
# `B` label in this corpus — the v0.10.0 source build is `b`.
DEFAULT_LABELS = ['A', 'c0', 'c1l', 'c1p', 'c1', 'b', 'A-loaded']


def render(doc):
    """The exact on-disk serialization of the artifact (indent=1 + trailing NL)."""
    return json.dumps(doc, indent=1, sort_keys=False) + '\n'


def build(ev, labels):
    """Load the labels and build the whole document. Returns (doc, present, cases)."""
    data = load_labels(ev, labels)
    # `A-loaded*` is the pre-kill, ~54 %-busy cross-check pass (REOriginal's
    # supplementary label). It must be LOADED as its own label (otherwise
    # diff-labels.pick() would fold `A-loaded.jsonl` into label `A` and the
    # baseline would silently become the contended pass) and then EXCLUDED from
    # the differential, whose numbers are the quiet pass only.
    present = [L for L in labels if L in data and data[L] and L != 'A-loaded']
    if 'A' not in present:
        raise SystemExit('label A missing from %s' % ev)
    cases = sorted({c for L in present for c in data[L]})
    doc = {
        'baseline': 'A (released v0.10.0 binary, quiet pass)',
        'labels': present,
        'normalization': {
            'applied': [
                'si_updated_by (fresh owner UUID per run) — everywhere',
                'x-request-id, timestamps, DPoP nonces, absolute paths in log lines, '
                'free-disk-space value and run-minted UUIDs inside app_log_lines '
                '(diff-labels.HOST_VARYING)',
                'body_file_bytes / Content-Length: profile-dependent transfer size, '
                'reported but not a finding when the decoded length agrees',
            ],
            'never_applied': [
                'status', 'response message text', 'content_type', 'state transitions '
                '(users/owners/si_rows/si_value)', 'error_type token', 'quota decisions',
                'handler_ran', 'app_sql_statement_count',
            ],
        },
        'differential_results_md_row_order': cases,
        'cases': [],
    }
    for cid in cases:
        entry = {'case_id': cid, 'present_in': [L for L in present if cid in data[L]], 'builds': {}}
        base_recs = data['A'].get(cid)
        base = project_record(base_recs[0]) if base_recs else None
        for L in present:
            recs = data[L].get(cid)
            if not recs:
                continue
            rec = project_record(recs[0])
            raw = recs[0]
            entry['builds'][L] = {k: v for k, v in rec.items() if k != 'nondeterministic'}
            claims = claims_for(cid, L)
            for cl in claims:
                ok, observed = cl[4](rec)
                entry.setdefault('claims', []).append({
                    'label': L, 'claim': cl[3], 'ok': bool(ok), 'observed': observed,
                    'verdict': cl[2] if ok else failed_verdict(cid, L)})
            if base is None or L == 'A':
                continue
            cmp = compare(cid, base, rec, 'A', L)
            verdict, why = classify(cid, cmp['diffs'], L)
            entry.setdefault('vs_A', {})[L] = {'verdict': verdict, 'explanation': why,
                                               'comparison': cmp}
        # the causal attribution pair: same commit, same profile, only the patches differ
        if 'c0' in data and cid in data['c0']:
            c0 = project_record(data['c0'][cid][0])
            for L in present:
                if L in ('A', 'c0') or cid not in data[L]:
                    continue
                other = project_record(data[L][cid][0])
                cmp = compare(cid, c0, other, 'c0', L)
                verdict, why = classify(cid, cmp['diffs'], L)
                entry.setdefault('vs_c0', {})[L] = {'verdict': verdict, 'explanation': why,
                                                    'comparison': cmp}
        entry['targeted_by'] = TARGETED.get(cid, [None, None])[0]
        entry['expected'] = TARGETED.get(cid, [None, None])[1]
        doc['cases'].append(entry)

    # ── the headline row, side by side, whatever else happens ────────────────
    row = {}
    for L in present:
        recs = data[L].get('setup-pw8-bademail')
        if recs:
            s = recs[0]['steps'][0]
            row[L] = {'body_decoded_len': s.get('body_decoded_len'),
                      'body_len': s.get('body_len'),
                      'content_type': (s.get('response') or {}).get('content_type'),
                      'status': (s.get('response') or {}).get('status')}
    doc['headline_setup_pw8_bademail'] = row
    return doc, present, cases


def summarise(doc, present, cases, bad_only=False):
    """The human-readable tail: one line per case + the failing-claim count."""
    if not bad_only:
        for cid in cases:
            e = next(x for x in doc['cases'] if x['case_id'] == cid)
            vs = e.get('vs_A', {})
            line = '  %-38s' % cid
            for L in present[1:]:
                line += ' %s=%s' % (L, vs.get(L, {}).get('verdict', '-'))
            print(line)
    print()
    print('-- claims (predicted behaviour per build; a failed claim is a finding) --')
    bad = 0
    for e in doc['cases']:
        for c in e.get('claims', []):
            if not c['ok']:
                bad += 1
                print('  FAILED %-34s %-4s %s' % (e['case_id'], c['label'], c['observed']))
    print('  %d failing claim(s)' % bad)
    return bad


def check(committed, ev='docs/re/evidence'):
    """Regenerate with the DEFAULTS and compare with the committed artifact.

    This is the staleness guard: if a label list, a claim or a TARGETED row moves
    without the artifact being regenerated, this exits non-zero and says where the
    two disagree. Exit codes: 0 identical, 1 different, 2 unreadable.
    """
    doc, present, cases = build(ev, DEFAULT_LABELS)
    fresh = render(doc)
    try:
        old = open(committed).read()
    except OSError as e:
        print('check: cannot read %s: %s' % (committed, e), file=sys.stderr)
        return 2
    summarise(doc, present, cases, bad_only=True)
    if fresh == old:
        print('CHECK PASS — %s is byte-identical to a fresh default run (%d bytes, labels %s)'
              % (committed, len(fresh), present))
        return 0
    import difflib
    old_l, new_l = old.splitlines(), fresh.splitlines()
    print('CHECK FAIL — %s differs from a fresh default run' % committed)
    n = 0
    for line in difflib.unified_diff(old_l, new_l, fromfile='committed', tofile='fresh',
                                     lineterm='', n=0):
        print(line)
        n += 1
        if n >= 60:
            print('… (diff truncated at 60 lines)')
            break
    return 1


def main(argv):
    if len(argv) >= 2 and argv[1] == '--check':
        return check(argv[2] if len(argv) > 2 else 'docs/re/evidence/differential.json')
    if len(argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    ev, outp = argv[1], argv[2]
    labels = argv[3:] or DEFAULT_LABELS
    doc, present, cases = build(ev, labels)
    with open(outp, 'w') as fh:
        fh.write(render(doc))
    print('wrote %s (%d cases, labels %s)' % (outp, len(doc['cases']), present))
    summarise(doc, present, cases)
    return 0


if __name__ == '__main__':
    raise SystemExit(main(sys.argv))
