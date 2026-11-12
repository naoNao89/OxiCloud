#!/usr/bin/env python3
"""diff-labels.py — compare two OxiCloud RE corpus label sets field by field.

    python3 docs/re/scripts/diff-labels.py <evidence-dir> [label_a label_b]
    # defaults: A A-loaded, looking for A*.jsonl and A-loaded*.jsonl

Why this exists: the harness records a lot per step, but only some of it is
reproducible. This script splits every field into DETERMINISTIC (must match
between two runs of the same binary, and is therefore what a cross-build diff
must be built on) and NONDETERMINISTIC (varies between two identical runs and
must be excluded, or every diff is noise). It is the check behind the claim in
docs/re/runtime-evidence.md that the delivered `A` and `A-loaded` corpora agree
on everything the questions depend on, and it is the same split stated in the
harness header (lib-re.sh) for the agent doing the two-build differential.

Exit status: 0 when the two sets agree on every DETERMINISTIC field,
1 when they do not (a real difference worth investigating), 2 on bad input.
"""
import json
import os
import re
import sys

# Fields that MUST agree between two runs of the same binary under the same
# posture. A difference here is a finding, not noise.
# `response` is projected field by field: x_request_id lives there but is a
# fresh UUID per request, so taking the whole object would drown every diff.
DET_RESPONSE = ('status', 'content_type', 'content_encoding',
                'content_length', 'retry_after')
DET_STEP = (
    'name', 'request', 'body_file', 'body_len', 'body_file_bytes',
    'body_decoded_len', 'body_decoded_source', 'handler_ran',
    'app_sql_statement_count', 'app_sql_statement_summaries',
    'resolved_client_ip', 'app_log_lines',
)
DET_CASE = ('case_id', 'label', 'note', 'config', 'state_users_owners')


# Lines the app emits that carry a value belonging to the HOST, not the build:
# the setup-create path logs the free space it saw when deciding whether to apply
# the default quota, so the number tracks the sandbox's disk, not the binary.
# Two runs of the same binary therefore differ here and nowhere else, which is
# exactly the kind of noise a cross-build diff must not stop on.
HOST_VARYING = (
    # (1) the free space the setup-create path observed when deciding whether to
    #     apply the default quota — a property of the sandbox's disk
    (re.compile(r'Available disk space \(\d+ bytes\)'),
     'Available disk space (N bytes)'),
    # (2) ids the run MINTS (user, personal drive, root folder, calendar, address
    #     book). They appear inside audit/user_lifecycle log lines, so a fresh
    #     UUID per run would otherwise show up as a diff on every creating case.
    (re.compile(r'[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}'),
     '<uuid>'),
)


def norm_line(l):
    for pat, repl in HOST_VARYING:
        l = pat.sub(repl, l)
    return l


def die(msg, code=2):
    print(msg, file=sys.stderr)
    raise SystemExit(code)


def load(path):
    with open(path, 'r', errors='replace') as fh:
        return [json.loads(l) for l in fh if l.strip()]


def pick(ev, labels):
    """Group *.jsonl by label. A file belongs to the LONGEST label that matches
    it, so `A-loaded-xff.jsonl` goes to `A-loaded` and never to `A`."""
    groups = {L: [] for L in labels}
    for f in sorted(os.listdir(ev)):
        if not f.endswith('.jsonl'):
            continue
        best = None
        for L in labels:
            if f == L + '.jsonl' or f.startswith(L + '-'):
                if best is None or len(L) > len(best):
                    best = L
        if best:
            groups[best].append(os.path.join(ev, f))
    return groups


def project(paths):
    """case_id -> deterministic projection, plus the nondeterministic values
    kept aside for reporting."""
    proj, nondet = {}, {}
    for p in paths:
        for r in load(p):
            st = r['state_after']
            proj.setdefault(r['case_id'], {})[p] = {
                'steps': [dict({k: (s[k] if k != 'app_log_lines'
                                     else [norm_line(x) for x in s[k]])
                                for k in DET_STEP},
                               response={k: s['response'][k] for k in DET_RESPONSE})
                          for s in r['steps']],
                'state_before': {k: r['state_before'][k]
                                 for k in ('users', 'owners', 'si_rows', 'si_value')},
                'state_after': {k: st[k]
                                for k in ('users', 'owners', 'si_rows', 'si_value')},
                'step_count': len(r['steps']),
            }
            nondet.setdefault(r['case_id'], {})[p] = {
                'si_updated_by_before': r['state_before']['si_updated_by'],
                'si_updated_by_after': st['si_updated_by'],
                'x_request_ids': [s['response']['x_request_id'] for s in r['steps']],
                'db_now': r['db_now'],
                'pg_window_sha256': [s['pg_log_window'].get('sha256', '')
                                     for s in r['steps']],
            }
    return proj, nondet


def main(argv):
    if len(argv) < 2:
        die(__doc__)
    ev = argv[1]
    la, lb = (argv[2], argv[3]) if len(argv) > 3 else ('A', 'A-loaded')
    groups = pick(ev, (la, lb))
    fa, fb = groups[la], groups[lb]
    if not fa or not fb:
        die('no files for labels %s / %s under %s' % (la, lb, ev))

    pa, na = project(fa)
    pb, nb = project(fb)
    print('A = %s (%s)' % (la, ', '.join(os.path.basename(x) for x in fa)))
    print('B = %s (%s)' % (lb, ', '.join(os.path.basename(x) for x in fb)))
    print()

    same = raceonly = real = 0
    detail = []
    for cid in sorted(set(pa) | set(pb)):
        a = next(iter(pa.get(cid, {}).values()), None)
        b = next(iter(pb.get(cid, {}).values()), None)
        if a == b:
            same += 1
            continue
        if a is None or b is None:
            real += 1
            print('MISSING %-32s present in only one set' % cid)
            continue
        ms_a = sorted(s['response']['status'] for s in a['steps'])
        ms_b = sorted(s['response']['status'] for s in b['steps'])
        if ms_a != ms_b:
            real += 1
            print('STATUS-MULTISET-DIFFERS %s' % cid)
            print('   A=%s' % ms_a)
            print('   B=%s' % ms_b)
            detail.append((cid, a, b))
            continue
        raceonly += 1
        # same outcomes, different arrangement
        print('same-outcomes %-32s statuses %s' % (cid, ms_a))
        if a['steps'] != b['steps']:
            detail.append((cid, a, b))

    for cid, a, b in detail:
        print()
        print('--- differences within %s (same status multiset)' % cid)
        if a['state_before'] != b['state_before'] or a['state_after'] != b['state_after']:
            print('   state  A=%s/%s  B=%s/%s' % (a['state_before'], a['state_after'],
                                                  b['state_before'], b['state_after']))
        for i, (sa, sb) in enumerate(zip(a['steps'], b['steps'])):
            for k in list(DET_STEP) + ['response']:
                if sa[k] != sb[k]:
                    va, vb = str(sa[k]), str(sb[k])
                    if len(va) > 100:
                        va, vb = va[:100] + '…', vb[:100] + '…'
                    print('   step %2d %-24s A=%s' % (i + 1, k, va))
                    print('   %5s %-24s B=%s' % ('', '', vb))
    print()
    print('cases identical (field by field)      : %d' % same)
    print('cases with same status multiset only  : %d' % raceonly)
    print('cases whose status multiset differs   : %d' % real)
    print()
    print('Reading: the second line is the class a cross-build diff must treat as')
    print('an outcome (compare the multiset and the state counters, not the step')
    print('index or the per-step log window — simultaneous steps bleed into each')
    print('other’s windows and the loser of a claim race may issue 1 or 2 queries).')
    print('Only the third line is a finding.')
    return 1 if real else 0


if __name__ == '__main__':
    raise SystemExit(main(sys.argv))
