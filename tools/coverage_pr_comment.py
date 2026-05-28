#!/usr/bin/env python3
"""tools/coverage_pr_comment.py — Generate a PR coverage comment from LCOV data.

Usage:
    python3 tools/coverage_pr_comment.py <current_lcov> <baseline_lcov> <base_ref> \
        [--fail-under=80] [--ratchet-tolerance=0.5] [--output=FILE]

Writes a markdown comment to --output (default: stdout).
Exits non-zero if diff coverage < --fail-under OR total drops > --ratchet-tolerance pp.
"""

import sys
import os
import re
import subprocess
from pathlib import Path

def parse_lcov(path: str) -> dict:
    files: dict = {}
    cur: str | None = None
    with open(path) as f:
        for raw in f:
            line = raw.strip()
            if line.startswith('SF:'):
                cur = line[3:]
                files[cur] = {'lh': 0, 'lf': 0, 'lines': {}}
            elif cur:
                if line.startswith('DA:'):
                    parts = line[3:].split(',')
                    lineno = int(parts[0])
                    count  = int(parts[1].split('=')[0])
                    files[cur]['lines'][lineno] = count
                elif line.startswith('LH:'):
                    files[cur]['lh'] = int(line[3:])
                elif line.startswith('LF:'):
                    files[cur]['lf'] = int(line[3:])
                elif line == 'end_of_record':
                    cur = None
    return files

def merge_lcov(partial: dict, baseline: dict) -> dict:
    merged = dict(baseline)
    merged.update(partial)
    return merged

def get_changed_lines(base_ref: str) -> dict:
    try:
        diff = subprocess.check_output(
            ['git', 'diff', '--unified=0', f'{base_ref}...HEAD'],
            text=True, stderr=subprocess.DEVNULL,
        )
    except subprocess.CalledProcessError:
        return {}
    changed: dict = {}
    cur_file: str | None = None
    for line in diff.splitlines():
        if line.startswith('+++ b/'):
            cur_file = line[6:]
            changed.setdefault(cur_file, set())
        elif line.startswith('@@ ') and cur_file is not None:
            m = re.search(r'\+(\d+)(?:,(\d+))?', line)
            if m:
                start = int(m.group(1))
                count = int(m.group(2)) if m.group(2) is not None else 1
                for i in range(start, start + count):
                    changed[cur_file].add(i)
    return changed

_SOURCE_EXTS = {'.rs'}
_EXCL_OFF = frozenset({'// coverage:off', '# coverage:off'})
_EXCL_ON  = frozenset({'// coverage:on',  '# coverage:on'})

def get_excluded_lines(fname: str) -> set:
    try:
        lines = Path(fname).read_text().splitlines()
    except (FileNotFoundError, OSError):
        return set()
    excluded: set = set()
    off = False
    for i, line in enumerate(lines, 1):
        token = line.strip().split('—')[0].strip()
        if token in _EXCL_OFF:
            off = True
        if off:
            excluded.add(i)
        if token in _EXCL_ON:
            off = False
    return excluded

def _icon(pct: float) -> str:
    if pct >= 80: return '🟢'
    if pct >= 60: return '🟡'
    return '🔴'

def _delta_str(delta: float) -> str:
    if delta > 0:  return f'📈 +{delta:.2f}pp'
    if delta < 0:  return f'📉 {delta:.2f}pp'
    return '➡️ 0.00pp'

def main() -> None:
    import argparse
    p = argparse.ArgumentParser()
    p.add_argument('current_lcov')
    p.add_argument('baseline_lcov')
    p.add_argument('base_ref')
    p.add_argument('--fail-under',        type=float, default=80.0)
    p.add_argument('--ratchet-tolerance', type=float, default=0.5)
    p.add_argument('--output',            default=None)
    p.add_argument('--merge-baseline',    action='store_true')
    args = p.parse_args()

    current  = parse_lcov(args.current_lcov)
    baseline = parse_lcov(args.baseline_lcov) if os.path.exists(args.baseline_lcov) else {}

    if args.merge_baseline and baseline:
        current = merge_lcov(current, baseline)
    changed_lines = get_changed_lines(args.base_ref)

    cur_lh = sum(v['lh'] for v in current.values())
    cur_lf = sum(v['lf'] for v in current.values())
    cur_pct = cur_lh * 100.0 / cur_lf if cur_lf else 0.0

    bas_lh = sum(v['lh'] for v in baseline.values())
    bas_lf = sum(v['lf'] for v in baseline.values())
    bas_pct = bas_lh * 100.0 / bas_lf if bas_lf else 0.0
    total_delta = cur_pct - bas_pct

    diff_hit = diff_total = 0
    file_rows: list = []

    for fname, added_lines in changed_lines.items():
        if not added_lines:
            continue
        if Path(fname).suffix not in _SOURCE_EXTS:
            continue
        if fname not in current:
            continue
        lcov_lines = current[fname]['lines']
        file_lh    = current[fname]['lh']
        file_lf    = current[fname]['lf']
        excluded     = get_excluded_lines(fname)
        instrumented = {ln for ln in added_lines if ln in lcov_lines and ln not in excluded}
        hit = sum(1 for ln in instrumented if lcov_lines[ln] > 0)
        diff_hit   += hit
        diff_total += len(instrumented)
        if file_lf > 0:
            file_pct  = file_lh * 100.0 / file_lf
            bas_entry = baseline.get(fname, {})
            bas_fpct  = (bas_entry.get('lh', 0) * 100.0 / bas_entry['lf']
                         if bas_entry.get('lf') else None)
            diff_fpct = hit * 100.0 / len(instrumented) if instrumented else None
            if file_pct < 100.0:
                file_rows.append((fname, file_pct, bas_fpct, diff_fpct, file_lh, file_lf))

    diff_pct = diff_hit * 100.0 / diff_total if diff_total else None

    violations: list[str] = []
    if diff_pct is not None and diff_pct < args.fail_under:
        violations.append(f'Diff coverage {diff_pct:.1f}% < threshold {args.fail_under:.0f}%')
    ratchet_drop = bas_pct - cur_pct
    if baseline and ratchet_drop > args.ratchet_tolerance:
        violations.append(
            f'Total coverage dropped {ratchet_drop:.2f}pp '
            f'({bas_pct:.2f}% → {cur_pct:.2f}%); tolerance {args.ratchet_tolerance}pp'
        )

    out: list[str] = ['<!-- coverage-report -->', '## Coverage', '']
    out.append('| | Coverage | vs Baseline |')
    out.append('|---|---|---|')
    out.append(
        f'| **Total** | {_icon(cur_pct)} {cur_pct:.2f}% ({cur_lh}/{cur_lf}) '
        f'| {_delta_str(total_delta)} |'
    )
    if diff_pct is not None:
        out.append(
            f'| **Changed lines** | {_icon(diff_pct)} {diff_pct:.1f}% '
            f'({diff_hit}/{diff_total}) | — |'
        )
    else:
        out.append('| **Changed lines** | — no instrumented lines changed — | — |')

    if violations:
        out.append('')
        for v in violations:
            out.append(f'> ❌ {v}')

    if file_rows:
        file_rows.sort(key=lambda r: r[1])
        out.append('')
        out.append('<details>')
        out.append('<summary>Per-file breakdown (changed files with &lt;100% coverage)</summary>')
        out.append('')
        out.append('| File | Total | Baseline | Diff |')
        out.append('|---|---|---|---|')
        for fname, fpct, bas_fpct, fdiff, flh, flf in file_rows:
            short    = '/'.join(fname.split('/')[-2:])
            bas_str  = f'{bas_fpct:.1f}%' if bas_fpct is not None else '—'
            diff_str = f'{fdiff:.0f}%'    if fdiff is not None     else '—'
            out.append(
                f'| `{short}` | {_icon(fpct)} {fpct:.1f}% ({flh}/{flf}) '
                f'| {bas_str} | {diff_str} |'
            )
        out.append('')
        out.append('</details>')

    comment = '\n'.join(out) + '\n'
    if args.output:
        Path(args.output).write_text(comment)
    else:
        print(comment, end='')

    if violations:
        for v in violations: print(f'FAIL: {v}', file=sys.stderr)
        sys.exit(1)

if __name__ == '__main__':
    main()
