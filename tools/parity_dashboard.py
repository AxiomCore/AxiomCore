"""Source inventory of dashboard HTTP actions. No network or third-party packages."""
from pathlib import Path
import json
import re

PROJECT = '/api/v1/projects/{project}'


def canonical(name):
    return re.sub(r'[^a-z0-9]', '', name.lower())


def arguments(text, start):
    """Split call arguments, respecting nested brackets and quoted JS strings."""
    depth, quote, escape, begin, out = 0, None, False, start, []
    for i in range(start, len(text)):
        c = text[i]
        if quote:
            if escape: escape = False
            elif c == '\\': escape = True
            elif c == quote: quote = None
            continue
        if c in "'\"`": quote = c
        elif c in '([{': depth += 1
        elif c in ')]}':
            if c == ')' and depth == 0:
                return out + [text[begin:i].strip()]
            depth -= 1
        elif c == ',' and depth == 0:
            out.append(text[begin:i].strip()); begin = i + 1
    raise ValueError('Unclosed API call')


def group_for(path):
    for token, group in [('auth/environments', 'Authentication administration'), ('/data/', 'Data editor and capture'), ('/branches', 'Branches'), ('/migrations', 'Migrations'), ('/resources', 'Resources and billing'), ('/resource-catalog', 'Resources and billing'), ('secret', 'Secrets'), ('configuration-environments', 'Configuration environments'), ('/workspace', 'Workspace'), ('/onboarding', 'Onboarding'), ('/auth', 'Account authentication'), ('/session', 'Account authentication'), ('/environments', 'Sandbox'), ('/contracts', 'Contracts and releases'), ('/releases', 'Contracts and releases')]:
        if token in path: return group
    return 'Projects and intelligence'


def scan_dashboard(root):
    root = Path(root)
    if (root / 'apps/dashboard').is_dir(): root = root / 'apps/dashboard'
    js = root / 'static/js'
    endpoints = json.loads((root / 'public/axiom.axiom').read_text())['endpoints']
    endpoint_map = {canonical(e['name']): e for e in endpoints}
    rows, errors = [], []
    def add(action, method, path, file, pos, evidence):
        path = path.split('?')[0].rstrip('/')
        text = file.read_text()
        rows.append(dict(action=action, method=method, path=path, source=str(file.relative_to(root)), line=text.count('\n', 0, pos)+1, group=group_for(path), evidence=evidence))
    files = sorted(js.glob('*.js')) + sorted((root / 'templates').rglob('*.tmpl'))
    # Every raw transport currently has a reviewed resolver below. A new raw
    # transport must be taught to this scanner rather than silently excluded.
    known_transports = {
        'project-secrets.js', 'project-resources.js', 'project-recovery.js',
        'project-migrations.js', 'project-database.js', 'project-auth.js',
        'project-branches.js', 'auth-admin-client.js', 'data-editor-client.js',
        # This file emits a fetch example for users; it does not execute it.
        'project-intelligence.js',
    }
    for file in files:
        if file.name.startswith(('atmx', 'axui')): continue
        text = file.read_text()
        fetches = list(re.finditer(r'\bfetch\s*\(', text))
        if fetches and (file.name not in known_transports or len(fetches) != 1):
            errors.append(f'{file.name}: unreviewed raw fetch transport')
        streams = list(re.finditer(r'new\s+(?:EventSource|XMLHttpRequest|WebSocket)\s*\(', text))
        if streams and (file.name != 'dashboard-state.js' or len(streams) != 1):
            errors.append(f'{file.name}: unreviewed streaming transport')
    # Operation names are resolved from the shipped contract, not guessed from
    # camel-case names. Generated SDK declarations alone do not establish use.
    for file in files:
        if file.name.startswith(('atmx', 'axui')): continue
        text = file.read_text()
        pattern = r'(?:queryOnce|mutate|watch|watchLive|watchTelemetry|cachedQuery)\(\s*[\'\"]([A-Za-z_]+)[\'\"]|axiom\.([A-Za-z_]+)'
        for m in re.finditer(pattern, text):
            name = m.group(1) or m.group(2)
            endpoint = endpoint_map.get(canonical(name))
            if endpoint:
                add(endpoint['name'], endpoint['method'], endpoint['path'], file, m.start(), m.group())
            elif m.group(1): errors.append(f'{file.name}: unknown SDK operation {name}')
    # Browser stream is a first-class read transport, outside the generated SDK.
    file = js / 'api-client.js'; text = file.read_text()
    for m in re.finditer(r'`\$\{apiBase\(\)\}(/api/v1/[^`]+)`', text):
        path = re.sub(r'\$\{[^}]+\}', '{id}', m.group(1))
        action = 'watch activity stream' if path.endswith('/activity/stream') else 'download contract release' if path.endswith('/pull') else 'download latest contract'
        add(action, 'GET', path, file, m.start(), m.group())
    # Direct SDK bindings in templates must resolve too. One existing legacy
    # partial references a removed operation; keep that defect visible.
    for file in files:
        text = file.read_text()
        for m in re.finditer(r'sdk\.axiom\.([A-Za-z_]+)\(', text):
            name = m.group(1)
            if canonical(name) in endpoint_map: continue
            if name == 'createOrg':
                add('legacy create organization (unresolved SDK)', 'UNRESOLVED', 'SDK:createOrg', file, m.start(), m.group())
            else: errors.append(f'{file.name}: unknown direct SDK operation {name}')
    # Typed clients expose explicit capability allowlists. Inventory all entries,
    # including reads: command parity is not limited to mutations.
    for name, prefix in [('auth-admin-client.js', PROJECT+'/auth/environments/{environment}/'), ('data-editor-client.js', PROJECT+'/data/environments/{environment}/')]:
        file = js / name; text = file.read_text()
        block = text.index('const ROUTES'); end = text.index(']);', block)
        for m in re.finditer(r"\['(GET|POST|PUT|PATCH|DELETE)', '([^']+)'", text[block:end]):
            method, suffix = m.groups()
            add(f'{method.lower()} {suffix}', method, prefix+suffix, file, block+m.start(), m.group())
    # Handwritten transport wrappers. Resolve their base paths and enumerate
    # finite action selectors, rather than treating an action as a path ID.
    for file in files:
        text = file.read_text(); name = file.name
        if name.startswith(('atmx', 'axui')): continue
        for m in re.finditer(r'this\.(request|platformRequest|sendStepped|send)\(', text):
            kind = m.group(1); args = arguments(text, m.end()); expr = args[0]
            # Shared forwarding/retry calls have their concrete callers below.
            if expr in ('path', 'action', 'c.action', 's.action'):
                continue
            if name == 'project-recovery.js' and kind == 'send': continue
            method = 'GET'
            if kind in ('sendStepped', 'send'):
                method = args[1].strip("'\"")
            elif len(args)>1:
                mm = re.search(r"(?:method\s*:\s*|this\.json\()(['\"])(GET|POST|PUT|PATCH|DELETE)\1", args[1])
                if mm: method = mm.group(2)
                elif not args[1].lstrip().startswith('{') or re.search(r'\bmethod\s*:|\.\.\.', args[1]):
                    errors.append(f'{name}: unresolved request options {args[1]}'); continue
            base = PROJECT + ('/resources' if name in ('project-resources.js','project-resources-placement.js','project-resources-byo.js','project-recovery.js') else '')
            scope = '/configuration-environments/{environment}'
            expr = re.sub(r'\$\{this\.base\(\)\}', base, expr)
            expr = re.sub(r'\$\{this\.scope\([^)]*\)\}', scope, expr)
            expr = re.sub(r'this\.base\(\)', repr(base), expr)
            expr = re.sub(r'this\.scope\([^)]*\)', repr(scope), expr)
            expr = re.sub(r'^scope\b', repr(scope), expr)
            expr = re.sub(r'\$\{(?!action\}|operation\})[^}]+\}', '{id}', expr)
            if expr.startswith('`'):
                path = expr[1:expr.index('`',1)]
            else:
                residual = re.sub(r"['\"][^'\"]*['\"]", '', expr).replace('+', '').strip()
                query_suffix = residual == 'q' and "const q = '?environment='" in text
                if residual and residual != 'action' and '?' not in expr and not query_suffix:
                    errors.append(f'{name}: unresolved path expression {args[0]}'); continue
                chunks = re.findall(r"['\"]([^'\"]*)['\"]", expr)
                path = ''.join(chunks)
                if expr.endswith('+action'): path += '${action}'
            if name in ('project-secrets.js','project-branches.js') or kind == 'platformRequest':
                path = PROJECT + path
            elif not path.startswith('/api/v1'): path = '/api/v1'+path
            if '${operation}' in path:
                labels = re.search(r'const labels = \{([^}]+)\}', text)
                if not labels: raise ValueError('Unknown secret operation selector')
                actions = re.findall(r'(\w+):', labels.group(1))
                paths = [path.replace('${operation}', a) for a in actions]
            elif '${action}' in path:
                if name == 'project-recovery.js':
                    allowed = re.search(r'const allowed = \{([^}]+)\}', text)
                    if not allowed: raise ValueError('Unknown recovery selector')
                    actions = re.findall(r'(\w+):', allowed.group(1)) + re.findall(r"this\.send\('([^']+)'", text)
                elif path.endswith('/resources/${action}'):
                    template = (root / 'templates/pages/project_resources.tmpl').read_text()
                    actions = re.findall(r"billing\('([^']+)'\)", template)
                else:
                    selector = re.search(r'actions\(resource\)\{([^\n]+)', text)
                    if not selector: raise ValueError('Unknown resource action selector')
                    actions = sorted(set(re.findall(r"'([a-z_]+)'", selector.group(1))) - {'unprovisioned','customer'})
                paths = [path.replace('${action}', a) for a in actions]
            else: paths = [path]
            if not path.startswith('/api/v1/') or method not in ('GET','POST','PUT','PATCH','DELETE'):
                errors.append(f'{name}: unresolved {m.group()}{args[0]}'); continue
            for path in paths:
                # Nearest named method provides a useful dashboard action label.
                before = text[:m.start()]
                names = list(re.finditer(r'(?:async\s+)?(\w+)\([^()]*\)\s*\{', before))
                action = names[-1].group(1) if names else method.lower()
                add(f'{name[:-3]}: {action} ({method} {path.split("/")[-1]})', method, path, file, m.start(), m.group()+args[0])
    if errors: raise ValueError('\n'.join(sorted(set(errors))))
    # Keep distinct dashboard call sites even when they share a transport route.
    unique = {(r['method'],r['path'],r['source'],r['line']):r for r in rows}
    return sorted(unique.values(), key=lambda r:(r['group'],r['path'],r['method'],r['source'],r['line']))

if __name__ == '__main__':
    import sys
    print(json.dumps(scan_dashboard(sys.argv[1]), indent=2))
