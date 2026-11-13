#!/usr/bin/env python3
"""Audit dashboard/CLI HTTP parity without credentials or network requests."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys


def route_key(method, path):
    # Parameter names differ between the Rust client and generated SDK.
    path = path.split('?')[0].rstrip('/')
    path = re.sub(r'\{[^{}]+\}', '{}', path)
    return method.upper(), path


# The only reasons the CLI may deliberately lack a command for a dashboard route. Each
# category names the exact routes it may cover, the dashboard operations (actions) allowed
# to call each of them and the dashboard groups they belong to. A route the dashboard reaches
# through any other action is part of another flow, so it stays a gap until a command maps it.
EXEMPTION_CATEGORIES = {
    'browser-session': {
        'description': 'dashboard sign-in routes of the browser session (email code, HttpOnly cookie); the CLI uses device login',
        'groups': {'Account authentication'},
        'routes': {
            ('POST', '/api/v1/auth/send'): {'SendOTP'},
            ('POST', '/api/v1/auth/verify'): {'VerifyOTP'},
            ('GET', '/api/v1/auth/session'): {'GetSession'},
            ('POST', '/api/v1/auth/session/refresh'): {'RefreshBrowserSession'},
            ('DELETE', '/api/v1/auth/session'): {'Logout'},
        },
    },
}


def validate_exemptions(exempt, dashboard, cli):
    """Checks every exemption and returns them keyed by `route_key`. An exemption must be
    `{'category': <allowlisted category>, 'reason': <non-blank text>}` on a route of that
    category, only used by the category's dashboard groups, and never on a route a CLI
    command maps (the mapping wins, so the exemption is stale or hides a partial). Any
    violation raises ValueError, which fails the checker (exit 2)."""
    mapped = {route_key(row['method'], row['path']) for row in cli}
    errors, result = [], {}
    for raw_key, entry in (exempt or {}).items():
        if not (isinstance(raw_key, tuple) and len(raw_key) == 2 and all(isinstance(part, str) for part in raw_key)):
            errors.append(f'{raw_key!r}: an exemption key must be (METHOD, path)')
            continue
        key = route_key(*raw_key)
        name = f'{key[0]} {key[1]}'
        if not isinstance(entry, dict):
            errors.append(f'{name}: an exemption must be {{"category": ..., "reason": ...}}, not {type(entry).__name__}')
            continue
        category, reason = entry.get('category'), entry.get('reason')
        spec = EXEMPTION_CATEGORIES.get(category) if isinstance(category, str) else None
        if not isinstance(reason, str) or not reason.strip():
            errors.append(f'{name}: the exemption has no reason')
        if spec is None:
            errors.append(f'{name}: category {category!r} is not an allowed exemption category ({", ".join(sorted(EXEMPTION_CATEGORIES))})')
        routes = {route_key(*route): actions for route, actions in spec['routes'].items()} if spec is not None else {}
        if spec is not None and key not in routes:
            errors.append(f'{name}: not a {category} route, so exempting it would hide a real gap')
        if key in mapped:
            errors.append(f'{name}: a CLI command maps this route; remove the exemption')
        if key in routes:
            callers = [row for row in dashboard if route_key(row['method'], row['path']) == key]
            outside = sorted({row['group'] for row in callers} - spec['groups'])
            if outside:
                errors.append(f'{name}: the dashboard uses it in {", ".join(outside)}, outside the {category} category, so exempting it would hide a real gap')
            for row in callers:
                if row.get('action') not in routes[key]:
                    errors.append(f"{name}: the dashboard calls it from {row.get('action')} ({row.get('source')}:{row.get('line')}), not a {category} operation, so exempting it would hide a real gap")
        if key in result:
            errors.append(f'{name}: exempted twice')
        result[key] = {'category': category, 'reason': reason.strip() if isinstance(reason, str) else ''}
    if errors:
        raise ValueError('invalid parity exemptions:\n  ' + '\n  '.join(errors))
    return result


def compare(dashboard, cli, exempt=None):
    """`exempt` maps a (METHOD, path) to `{'category', 'reason'}`: the reviewed reason the CLI
    deliberately has no command for it (see `validate_exemptions`, which runs first and
    raises ValueError on an invalid one). Such an endpoint is `exempt` rather than a gap."""
    exempt = validate_exemptions(exempt, dashboard, cli)
    index = {}
    for row in cli:
        index.setdefault(route_key(row['method'], row['path']), []).append(row)
    result = []
    for row in dashboard:
        key = route_key(row['method'], row['path'])
        matches = index.get(key, [])
        status = 'capability only' if row.get('capability_only') else 'unresolved' if row['method'] == 'UNRESOLVED' else 'partial' if matches and all(r.get('note') for r in matches) else 'mapped' if matches else 'exempt' if key in exempt else 'gap'
        extra = {'exempt_category': exempt[key]['category'], 'exempt_reason': exempt[key]['reason']} if status == 'exempt' else {}
        result.append({**row, 'commands': sorted({r['command'] for r in matches}),
                       'cli_evidence': matches, 'status': status, **extra})
    return sorted(result, key=lambda r: (r['group'], r['path'], r['method'], r['action'], r['source'], r['line']))


def revision(root):
    return subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip()


def cell(value):
    return str(value).replace('|', '\\|').replace('\n', ' ')


def grouped_gaps(rows):
    endpoints = {}
    for r in rows:
        if r['status'] not in ('gap', 'partial', 'unresolved'): continue
        key = route_key(r['method'], r['path'])
        if key not in endpoints: endpoints[key] = {**r, 'actions': set()}
        endpoints[key]['actions'].add(r['action'])
    return sorted(endpoints.values(), key=lambda r: (r['group'], r['status'], route_key(r['method'], r['path'])))


def gap_lines(rows):
    gaps = grouped_gaps(rows)
    lines = []
    for group in sorted({r['group'] for r in gaps}):
        members = [r for r in gaps if r['group'] == group]
        counts = {s: sum(r['status'] == s for r in members) for s in ('gap', 'partial', 'unresolved')}
        lines += [f"### {group} — {counts['gap']} gaps, {counts['partial']} partial" + (f", {counts['unresolved']} unresolved" if counts['unresolved'] else ''), '']
        for r in members:
            lines.append(f"- **{r['status']}** `{r['method']} {r['path']}`: {', '.join(sorted(r['actions']))}")
        lines.append('')
    return lines


def markdown(rows, frontend_rev, cli_rev):
    capabilities = [r for r in rows if r['status'] == 'capability only']
    rows = [r for r in rows if r['status'] != 'capability only']
    gaps = [r for r in rows if r['status'] not in ('mapped', 'exempt')]
    unique = lambda rs: len({route_key(r['method'], r['path']) for r in rs if r['method'] != 'UNRESOLVED'})
    lines = ['# Dashboard / CLI parity matrix', '',
             'W4-L7 part 1, D9 parity plus. Source audit only; no commands are added by this change.', '',
             f'Frontend revision: `{frontend_rev}`. AxiomCore revision: `{cli_rev}`.', '',
             f'{len(rows)} action records; {unique(rows)} HTTP endpoints; {unique(gaps)} endpoints with missing or partial CLI coverage.', '',
             f"{unique([r for r in rows if r['status'] == 'mapped'])} mapped; {unique([r for r in rows if r['status'] == 'partial'])} partial; {unique([r for r in rows if r['status'] == 'gap'])} no CLI call; {unique([r for r in rows if r['status'] == 'exempt'])} exempt (N/A for the CLI, with a reviewed reason). Capability-only rows are excluded from these totals.", '',
             'An action record is a referenced SDK operation, a used typed client route, or a direct request call site. '
             'Repeated call sites are retained. `mapped` proves a source-level command/HTTP route correspondence, '
             'not complete payload, permissions, output, or production behavior equivalence. '
             'Parameterized paths are matched by position; HTTP methods must match. '
             'Browser navigation, local UI state, and external provider pages are outside this HTTP matrix. '
             '''Generated SDK declarations alone do not prove a dashboard action. UNRESOLVED rows identify dashboard references missing from the shipped API contract; investigate those before specifying a CLI command.''', '',
             '## Reproduce', '',
             'From an AxiomCore checkout, with a sibling axiom-frontend checkout:', '',
             '```sh', 'python3 tools/parity-check.py --frontend ../axiom-frontend',
             'python3 tools/parity-check.py --frontend ../axiom-frontend --format json',
             'python3 tools/parity-check.py --frontend ../axiom-frontend --strict',
             'python3 -m unittest discover -s tools/tests -p "test_parity*.py"', '```', '',
             'Use `--format markdown --output PATH` to regenerate this file. Exit 0 means the audit ran; '
             '`--strict` returns 1 while gaps remain. Extraction failures return 2. '
             'The checker reads current source each time. CLI mappings are reviewed caller/dispatch anchors, not a Rust call-graph parser; update them when commands or their transport wrappers change. Review new dynamic request patterns before treating '
             'the inventory as exhaustive. Part 2 must add commands using the same APIs and test payload-level parity.', '',
             'The deployments service described by D9 is future shared dashboard/CLI work. '
             'A local server package command or managed sandbox does not establish hosted deployment parity.', '',
             '## Grouped gaps', '']
    lines += gap_lines(rows)
    lines += ['## Exempt (N/A for the CLI)', '']
    for key, r in sorted({route_key(r['method'], r['path']): r for r in rows if r['status'] == 'exempt'}.items()):
        lines.append(f"- `{r['method']} {r['path']}` ({r['group']}): {r['exempt_reason']}")
    lines.append('')
    lines += ['## Capability only (excluded from endpoint counts)', '']
    for r in capabilities:
        lines.append(f"- `{r['method']} {r['path']}` — {r['action']} (`{r['source']}:{r['line']}`)")
    lines.append('')
    lines += ['## Complete matrix', '', '| Group / dashboard action | HTTP endpoint | CLI command | Dashboard evidence |', '| --- | --- | --- | --- |']
    for r in rows:
        commands = ', '.join('`'+c+'`' for c in r['commands']) or ('**UNRESOLVED**' if r['status'] == 'unresolved' else '**GAP**')
        notes = sorted({c['note'] for c in r['cli_evidence'] if c.get('note')})
        if r['status'] == 'partial': commands = '**PARTIAL** ' + commands
        if r['status'] == 'exempt': commands = '**N/A** ' + r['exempt_reason']
        if notes:
            commands += ' (' + '; '.join(notes) + ')'
        lines.append('| ' + ' | '.join(map(cell, [r['group']+' / '+r['action'], f"`{r['method']} {r['path']}`", commands, f"`{r['source']}:{r['line']}`"])) + ' |')
    lines += ['', 'CLI source evidence, including route/client references, is available in the JSON output.', '']
    return '\n'.join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--core', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--frontend', type=Path, required=True)
    parser.add_argument('--format', choices=['text', 'json', 'markdown'], default='text')
    parser.add_argument('--output', type=Path)
    parser.add_argument('--group', help='only report one dashboard group (exact name)')
    parser.add_argument('--strict', action='store_true', help='exit 1 if any dashboard endpoint lacks a CLI mapping')
    args = parser.parse_args()
    try:
        from parity_dashboard import scan_dashboard
        from parity_cli import scan_cli, EXEMPT
        rows = compare(scan_dashboard(args.frontend), scan_cli(args.core), EXEMPT)
        if args.group:
            rows = [row for row in rows if row['group'] == args.group]
        if not rows:
            raise ValueError('No dashboard actions extracted')
        if args.format == 'json':
            output = json.dumps(rows, indent=2) + '\n'
        elif args.format == 'markdown':
            output = markdown(rows, revision(args.frontend), revision(args.core))
        else:
            output = '\n'.join(gap_lines(rows)) + f'\n{len(grouped_gaps(rows))} grouped endpoint gaps or partial\n'
        if args.output:
            args.output.write_text(output)
        else:
            print(output, end='')
        return int(args.strict and any(r['status'] in ('gap', 'partial', 'unresolved') for r in rows))
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        print(f'parity-check: {error}', file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
