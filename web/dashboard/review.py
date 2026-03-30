#!/usr/bin/env python3
"""CLI tool for recording spec refutation attempts, issues, and sub-evidence.

Popperian refutation model: specs are conjectures, confidence comes from
surviving attempts to break them. Each review is a refutation attempt with
type, severity, outcome, and git commit tracking.

Supports dot-notation for nested evidence:
  python3 dashboard/review.py R2-FNV --set testing.unit=true
  python3 dashboard/review.py R2-FNV --set testing.roundtrip=true

Also supports legacy flat keys for backward compat:
  python3 dashboard/review.py R2-FNV --set test_vectors true
"""

import argparse
import json
import os
import subprocess
import sys
from datetime import date

BASE = os.path.join(os.path.dirname(__file__), 'spec-meta')
REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..'))
REVIEWS_DIR = os.path.join(BASE, 'reviews')

# Valid refutation types and severity levels
REFUTATION_TYPES = [
    'peer_review', 'adversarial_vectors', 'dual_model_audit',
    'field_deployment', 'implementation_test', 'integration_test',
]
SEVERITY_LEVELS = ['casual', 'thorough', 'hostile', 'competing_models', 'real_world']

# Map legacy flat keys to new nested evidence paths
LEGACY_MAP = {
    'test_vectors': [('test_spec', 'spec_vectors'), ('test_spec', 'json_vectors')],
    'tests_passing': [('testing', 'unit'), ('testing', 'automated')],
    'implementation': [('built', 'complete',)],
    'integration_tests': [('integrated', 'component',)],
    'field_validation': [('field', 'lab',)],
}

# Default evidence structure
DEFAULT_EVIDENCE = {
    'reviewed': {'review1': False, 'review2': False, 'expert': False},
    'test_spec': {'spec_vectors': False, 'json_vectors': False, 'adversarial': False},
    'testing': {'unit': False, 'automated': False, 'roundtrip': False, 'human': False},
    'built': {'partial': False, 'complete': False, 'optimised': False},
    'integrated': {'component': False, 'cross_component': False, 'system': False},
    'field': {'lab': False, 'pilot': False, 'production': False},
}


def get_spec_commit(spec_path):
    """Get the current git commit hash for a spec file."""
    if not spec_path:
        return None
    try:
        result = subprocess.run(
            ['git', 'log', '-1', '--format=%H', '--', spec_path],
            cwd=REPO_ROOT,
            capture_output=True, text=True, timeout=5,
        )
        if result.returncode == 0 and result.stdout.strip():
            return result.stdout.strip()
    except (subprocess.TimeoutExpired, FileNotFoundError):
        pass
    return None


def load_reviews():
    """Load all per-spec review files into a single dict."""
    data = {}
    os.makedirs(REVIEWS_DIR, exist_ok=True)
    for fname in sorted(os.listdir(REVIEWS_DIR)):
        if fname.endswith('.json'):
            code = fname[:-5]  # strip .json
            with open(os.path.join(REVIEWS_DIR, fname)) as f:
                data[code] = json.load(f)
    return data


def save_reviews(data):
    """Save each spec's review data to its own file."""
    os.makedirs(REVIEWS_DIR, exist_ok=True)
    for code, entry in data.items():
        with open(os.path.join(REVIEWS_DIR, f'{code}.json'), 'w') as f:
            json.dump(entry, f, indent=2)


def ensure_evidence(entry):
    """Ensure entry has nested evidence structure."""
    if 'evidence' not in entry:
        entry['evidence'] = {}
    for col, signals in DEFAULT_EVIDENCE.items():
        if col not in entry['evidence']:
            entry['evidence'][col] = {}
        for sig, default in signals.items():
            if sig not in entry['evidence'][col]:
                entry['evidence'][col][sig] = default


def _build_diff_summary(entry):
    """Build a short summary of the current evidence state for this review.

    Compares against the evidence snapshot stored in the previous review
    (if any) and returns a human-readable diff string.
    """
    evidence = entry.get('evidence', {})
    # Flatten current evidence into a set of "col.signal" keys that are True
    current = set()
    for col, signals in evidence.items():
        for sig, val in signals.items():
            if val:
                current.add(f"{col}.{sig}")

    # Find previous review's snapshot (if any)
    previous = set()
    for rev in reversed(entry.get('reviews', [])):
        if 'evidence_snapshot' in rev:
            previous = set(rev['evidence_snapshot'])
            break

    # Store snapshot for future diffs
    entry.setdefault('_next_snapshot', sorted(current))

    added = sorted(current - previous)
    removed = sorted(previous - current)

    parts = []
    if added:
        parts.append('+' + ', +'.join(added))
    if removed:
        parts.append('-' + ', -'.join(removed))
    if not parts:
        filled = sum(1 for col in evidence.values() for v in col.values() if v)
        total = sum(len(col) for col in evidence.values())
        parts.append(f"{filled}/{total} evidence signals filled")

    return '; '.join(parts)


def rebuild():
    dashboard_dir = os.path.dirname(__file__)
    repo_root = os.path.join(dashboard_dir, '..')
    subprocess.run([sys.executable, os.path.join(dashboard_dir, 'calculate.py')], cwd=repo_root)
    subprocess.run([sys.executable, os.path.join(dashboard_dir, 'generate.py')], cwd=repo_root)
    # Table injection disabled — README now links to live dashboard instead
    # subprocess.run([sys.executable, os.path.join(dashboard_dir, 'generate_readme.py')], cwd=repo_root)


def parse_set_arg(raw):
    """Parse --set argument. Supports both:
      --set testing.unit=true       (new dot notation, single arg)
      --set test_vectors true       (legacy, two args)
      --set testing.unit true       (dot notation, two args)
    """
    if '=' in raw[0]:
        key, val = raw[0].split('=', 1)
    elif len(raw) >= 2:
        key = raw[0]
        val = raw[1]
    else:
        return None, None, None

    bool_val = val.lower() in ('true', '1', 'yes')

    if '.' in key:
        col, sig = key.split('.', 1)
        return col, sig, bool_val
    elif key in LEGACY_MAP:
        return key, None, bool_val  # legacy
    else:
        # Try as flat key for backward compat
        return key, None, bool_val


def main():
    parser = argparse.ArgumentParser(description='Record spec refutation attempts, issues, and evidence')
    parser.add_argument('spec', help='Spec code (e.g. R2-WIRE)')
    parser.add_argument('--reviewer', help='Reviewer name')
    parser.add_argument('--status', choices=['approved', 'needs-changes', 'rejected'], help='Review status')
    parser.add_argument('--note', help='Review note')
    parser.add_argument('--version', help='Version reviewed (default: from dependencies.json)')
    parser.add_argument('--type', choices=REFUTATION_TYPES, default='peer_review',
                        help='Refutation type (default: peer_review)')
    parser.add_argument('--severity', choices=SEVERITY_LEVELS, default='casual',
                        help='Severity level (default: casual)')
    parser.add_argument('--outcome', choices=['survived', 'falsified'],
                        help='Outcome (default: inferred from --status)')
    parser.add_argument('--falsification', help='What was found if falsified')
    parser.add_argument('--issue', help='Add an open issue')
    parser.add_argument('--resolve-issue', type=int, help='Resolve issue by index')
    parser.add_argument('--set', nargs='+', metavar='KEY=VALUE',
                        help='Set evidence (dot notation: testing.unit=true, or legacy: test_vectors true)')
    parser.add_argument('--show', action='store_true', help='Show current evidence state')

    args = parser.parse_args()
    reviews = load_reviews()
    code = args.spec.upper()

    if code not in reviews:
        reviews[code] = {
            'reviews': [],
            'evidence': {},
            'issues': [],
            # Keep legacy flat keys for reading compat
            'test_vectors': False,
            'tests_passing': False,
            'implementation': False,
            'integration_tests': False,
            'field_validation': False,
        }

    entry = reviews[code]
    ensure_evidence(entry)

    if args.show:
        print(f"\n{code} evidence:")
        for col, signals in entry['evidence'].items():
            filled = sum(1 for v in signals.values() if v)
            total = len(signals)
            print(f"  {col}: {filled}/{total}")
            for sig, val in signals.items():
                mark = '✓' if val else '·'
                print(f"    {mark} {sig}")
        sys.exit(0)

    if args.reviewer and args.status:
        ver = args.version
        spec_path = None
        if not ver:
            deps_file = os.path.join(BASE, 'dependencies.json')
            if os.path.exists(deps_file):
                with open(deps_file) as f:
                    deps = json.load(f)
                ver = deps.get(code, {}).get('version', '?')
                spec_path = deps.get(code, {}).get('path', '')
            else:
                ver = '?'
        else:
            deps_file = os.path.join(BASE, 'dependencies.json')
            if os.path.exists(deps_file):
                with open(deps_file) as f:
                    deps = json.load(f)
                spec_path = deps.get(code, {}).get('path', '')

        # Get current git commit for the spec
        spec_commit = get_spec_commit(spec_path) if spec_path else None

        # Determine outcome
        outcome = args.outcome
        if not outcome:
            outcome = 'survived' if args.status == 'approved' else 'falsified'

        # Build diff summary: snapshot evidence state before this review
        diff_summary = _build_diff_summary(entry)

        review = {
            'reviewer': args.reviewer,
            'date': date.today().isoformat(),
            'status': args.status,
            'version': ver,
            'type': args.type,
            'severity': args.severity,
            'outcome': outcome,
            'spec_commit': spec_commit,
        }
        if args.note:
            review['note'] = args.note
        if args.falsification:
            review['falsification'] = args.falsification
        if diff_summary:
            review['diff_summary'] = diff_summary
        # Store evidence snapshot for future diff comparisons
        if '_next_snapshot' in entry:
            review['evidence_snapshot'] = entry.pop('_next_snapshot')
        entry['reviews'].append(review)
        outcome_icon = '🛡' if outcome == 'survived' else '💥'
        print(f"{outcome_icon} Refutation attempt: {args.reviewer} → {outcome} ({args.type}/{args.severity}) v{ver}")
        if spec_commit:
            print(f"  Spec commit: {spec_commit[:12]}")

    if args.issue:
        entry['issues'].append({'text': args.issue, 'resolved': False})
        print(f"Added issue #{len(entry['issues'])-1}: {args.issue}")

    if args.resolve_issue is not None:
        idx = args.resolve_issue
        if 0 <= idx < len(entry['issues']):
            entry['issues'][idx]['resolved'] = True
            print(f"Resolved issue #{idx}")
        else:
            print(f"Issue #{idx} not found", file=sys.stderr)
            sys.exit(1)

    if args.set:
        col, sig, bool_val = parse_set_arg(args.set)
        if col is None:
            print(f"Invalid --set format. Use: --set column.signal=true", file=sys.stderr)
            sys.exit(1)

        if sig is not None:
            # Dot notation: set specific sub-evidence
            if col in entry['evidence'] and sig in entry['evidence'][col]:
                entry['evidence'][col][sig] = bool_val
                print(f"Set {col}.{sig} = {bool_val}")
            else:
                print(f"Unknown evidence path: {col}.{sig}", file=sys.stderr)
                print(f"Valid columns: {list(DEFAULT_EVIDENCE.keys())}")
                if col in DEFAULT_EVIDENCE:
                    print(f"Valid signals for {col}: {list(DEFAULT_EVIDENCE[col].keys())}")
                sys.exit(1)
        elif col in LEGACY_MAP:
            # Legacy flat key: map to nested
            for target_col, target_sig in LEGACY_MAP[col]:
                entry['evidence'][target_col][target_sig] = bool_val
                print(f"Set {target_col}.{target_sig} = {bool_val} (via legacy key {col})")
            # Also set flat key for compat
            entry[col] = bool_val
        elif col in ('test_vectors', 'tests_passing', 'implementation', 'integration_tests', 'field_validation'):
            entry[col] = bool_val
            print(f"Set {col} = {bool_val} (flat key)")
        else:
            print(f"Unknown key: {col}", file=sys.stderr)
            sys.exit(1)

    save_reviews(reviews)
    rebuild()


if __name__ == '__main__':
    main()
