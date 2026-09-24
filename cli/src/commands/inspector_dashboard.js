const S = {
  e: null,
  r: null,
  status: null,
  live: null,
  changes: null,
  answer: null,
  view: 'overview',
  selected: null,
  kind: '',
  query: '',
  graph: { root: null, direction: 'downstream', depth: 1, edge: 'all' },
  sourceCache: new Map(),
};

const $ = selector => document.querySelector(selector);
const $$ = selector => [...document.querySelectorAll(selector)];
const h = value => String(value ?? '').replace(/[&<>"']/g, character => ({
  '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;',
}[character]));
const token = new URLSearchParams(location.search).get('token') || sessionStorage.getItem('axiom-token');
if (token) sessionStorage.setItem('axiom-token', token);

async function api(path, options = {}) {
  options.headers = { ...(options.headers || {}), 'x-axiom-inspector-token': token };
  const response = await fetch(path, options);
  if (!response.ok) throw new Error(`${response.status} ${await response.text()}`);
  return response.status === 204 ? null : response.json();
}

const kinds = {
  source: ['source-unit', 'frontend-module', 'implementation-binding'],
  ui: ['route', 'page', 'component', 'primitive', 'action', 'action-step', 'state', 'derived-value', 'effect', 'style-rule', 'style-variable', 'responsive-branch', 'accessibility', 'asset'],
  backend: ['backend-service', 'operation', 'data-model', 'field', 'relationship', 'projection', 'validation-rule', 'auth-policy', 'security-policy', 'cache-policy', 'retry-policy', 'stream', 'implementation-binding'],
  dependencies: ['contract', 'package', 'dependency-environment', 'third-party-package', 'extension', 'extension-export', 'license', 'advisory', 'build-script', 'release-policy', 'release', 'implementation-binding', 'artifact'],
  security: ['permission', 'auth-policy', 'security-policy', 'advisory', 'build-script', 'release-policy', 'finding', 'readiness-fact'],
};

function nodeById(id) { return S.e.nodes.find(node => node.id === id); }
function edgeById(id) { return S.e.edges.find(edge => edge.id === id); }
function label(id) { return nodeById(id)?.label || id; }
function title(name, subtitle) {
  return `<div class="eyebrow">Declared · Resolved · Observed</div><h1>${h(name)}</h1><p class="sub">${h(subtitle)}</p>`;
}
function badge(value, className = '') { return `<span class="badge ${className}">${h(value)}</span>`; }
function empty(message = 'No facts match the current filters.') { return `<div class="empty">${h(message)}</div>`; }
function count(kind) { return S.e.nodes.filter(node => Array.isArray(kind) ? kind.includes(node.kind) : node.kind === kind).length; }

function nodes(list = S.e.nodes) {
  const target = $('#target')?.value || '';
  const layer = $('#layer')?.value || '';
  const query = S.query.toLowerCase();
  return list.filter(node =>
    (!target || !node.targets.length || node.targets.includes(target)) &&
    (!layer || node.layer === layer) &&
    (!query || `${node.label} ${node.id} ${node.kind} ${JSON.stringify(node.attributes)}`.toLowerCase().includes(query))
  );
}

function sourceLabel(node) {
  const evidence = (node.evidence || []).find(item => item.path);
  return evidence ? `${evidence.path}${evidence.detail ? ` · ${evidence.detail}` : ''}` : 'No source span';
}

function inbound(id) { return S.e.edges.filter(edge => edge.to === id); }
function outbound(id) { return S.e.edges.filter(edge => edge.from === id); }
function neighbors(id) {
  const ids = new Set();
  S.e.edges.forEach(edge => {
    if (edge.from === id) ids.add(edge.to);
    if (edge.to === id) ids.add(edge.from);
  });
  return S.e.nodes.filter(node => ids.has(node.id));
}

function fact(node) {
  return `<article class="fact" tabindex="0" data-id="${h(node.id)}">
    <div><span class="kind">${h(node.kind)}</span><br><b>${h(node.label)}</b></div>
    <code>${h(node.id)}</code>
    <div>${badge(node.verification, node.verification === 'verified' ? '' : 'dim')}</div>
  </article>`;
}

function table(list) {
  if (!list.length) return empty();
  return `<table class="matrix"><thead><tr><th>Kind</th><th>Fact</th><th>Targets</th><th>Verification</th></tr></thead><tbody>
    ${list.map(node => `<tr tabindex="0" data-id="${h(node.id)}">
      <td>${h(node.kind)}</td>
      <td><b>${h(node.label)}</b><br><span class="source">${h(sourceLabel(node))}</span></td>
      <td>${h((node.targets || []).join(', ') || 'all')}</td>
      <td>${badge(node.verification, node.verification === 'verified' ? '' : 'dim')}</td>
    </tr>`).join('')}
  </tbody></table>`;
}

function detail(labelText, value) {
  return `<div class="detail"><small>${h(labelText)}</small><b title="${h(value)}">${h(value || '—')}</b></div>`;
}

function relationCount(id, edgeKinds, direction = 'both') {
  return S.e.edges.filter(edge => {
    const matches = edgeKinds.includes(edge.kind);
    if (direction === 'upstream') return matches && edge.to === id;
    if (direction === 'downstream') return matches && edge.from === id;
    return matches && (edge.from === id || edge.to === id);
  }).length;
}

function cardBody(node) {
  const attributes = node.attributes || {};
  if (node.kind === 'operation') {
    const surface = attributes.surface || attributes.contractMetadata || {};
    const method = attributes.method || surface.method || surface.kind || 'operation';
    const path = attributes.path || surface.path || node.label;
    const request = surface.requestFields || [];
    const response = surface.responseFields || [];
    return {
      signature: `${method} ${path}`,
      details: [
        ['Kind', surface.operationKind || surface.kind || (attributes.backend ? 'backend' : 'frontend')],
        ['Exposure', attributes.exposure || (attributes.security?.public ? 'public' : 'contract-scoped')],
        ['Request fields', request.length ? request.join(', ') : 'none'],
        ['Response fields', response.length ? response.join(', ') : 'typed result'],
      ],
      note: `${relationCount(node.id, ['calls', 'resolves-to'], 'upstream')} callers · ${relationCount(node.id, ['calls', 'resolves-to', 'returns'], 'downstream')} downstream relationships`,
    };
  }
  if (node.kind === 'state') {
    return {
      signature: `${attributes.name || node.label}: ${attributes.type || 'inferred'}`,
      details: [
        ['Initial value', attributes.initializer || 'not exposed'],
        ['Readers', relationCount(node.id, ['reads'], 'upstream')],
        ['Writers', relationCount(node.id, ['writes'], 'upstream')],
        ['Targets', (node.targets || []).join(', ') || 'all'],
      ],
      note: 'State access is derived from compiler-owned reads, writes and effective authority.',
    };
  }
  if (node.kind === 'contract') {
    return {
      signature: `${attributes.projectId || node.label}@${attributes.projectVersion || 'local'}`,
      details: [
        ['Audience', attributes.audience || 'declared by contract'],
        ['Operations', relationCount(node.id, ['exposes'], 'downstream')],
        ['Artifact', attributes.artifact || 'resolved package'],
        ['Executable', attributes.executable ? 'yes' : 'no'],
      ],
      note: attributes.baseUrl ? `Development endpoint ${attributes.baseUrl}` : 'Resolved through verified contract evidence.',
    };
  }
  if (node.kind === 'extension') {
    const provenance = attributes.provenance || {};
    const packageIdentity = attributes.package || provenance.package || {};
    return {
      signature: `${packageIdentity.name || node.label}@${packageIdentity.version || 'local'}`,
      details: [
        ['Runtime', provenance.engine || 'sandboxed WASM'],
        ['SDK', provenance.sdk || 'verified extension SDK'],
        ['Permissions', relationCount(node.id, ['requests', 'grants', 'authorizes'])],
        ['Release', attributes.releaseEligible ? 'eligible' : 'development'],
      ],
      note: `${(node.targets || []).join(' · ') || 'all targets'} · ${attributes.sourceAvailability || 'source availability unknown'}`,
    };
  }
  if (node.kind === 'third-party-package') {
    return {
      signature: `${attributes.name || node.label}@${attributes.version || 'locked'}`,
      details: [
        ['Language', attributes.language || 'resolved ecosystem'],
        ['Registry', attributes.registry || 'locked registry'],
        ['Licenses', (attributes.licenses || []).join(', ') || 'not declared'],
        ['Introduced by', (attributes.introducedBy || []).join(', ') || 'transitive dependency'],
      ],
      note: `${attributes.byteLength || 0} bytes · sha256 ${String(attributes.artifactSha256 || '').slice(0, 16)}…`,
    };
  }
  if (node.kind === 'dependency-environment') {
    return {
      signature: `${attributes.language || 'guest'} ${attributes.runtimeVersion || ''}`.trim(),
      details: [
        ['Profile', attributes.profile || 'managed'],
        ['Engine', attributes.engine || 'pinned'],
        ['SDK', attributes.sdk || 'pinned'],
        ['Extensions', (attributes.extensions || []).join(', ') || 'none'],
      ],
      note: `${attributes.targetFamily || 'wasm32'} · environment ${String(attributes.identitySha256 || '').slice(0, 16)}…`,
    };
  }
  if (node.kind === 'release-policy') {
    const summary = attributes.summary || {};
    return {
      signature: attributes.compliant ? 'Policy passed' : 'Release blocked',
      details: [
        ['Packages', summary.packages || 0],
        ['Advisories', summary.advisories || 0],
        ['Scripts', summary.scripts || 0],
        ['Blockers', summary.blockers || 0],
      ],
      note: 'Evaluated only from canonical lock facts and the declared release policy.',
    };
  }
  if (['extension-export', 'implementation-binding', 'artifact', 'release'].includes(node.kind)) {
    return {
      signature: attributes.interfaceSha256 || attributes.moduleSha256 || attributes.package?.name || node.label,
      details: [
        ['Extension', attributes.ownerExtension || attributes.extension || 'resolved owner'],
        ['ABI', attributes.abi || 'verified package'],
        ['SDK / engine', attributes.sdk || attributes.engine || 'verified runtime'],
        ['Targets', (node.targets || []).join(', ') || 'all'],
      ],
      note: sourceLabel(node),
    };
  }
  return {
    signature: node.id,
    details: [
      ['Truth layer', node.layer],
      ['Verification', node.verification],
      ['Incoming', inbound(node.id).length],
      ['Outgoing', outbound(node.id).length],
    ],
    note: sourceLabel(node),
  };
}

function semanticCard(node) {
  const body = cardBody(node);
  return `<article class="card clickable semantic-card" tabindex="0" data-id="${h(node.id)}" data-node-kind="${h(node.kind)}">
    <div class="row between"><span class="kind">${h(node.kind)}</span>${badge(node.verification, node.verification === 'verified' ? '' : 'dim')}</div>
    <div><h3>${h(node.label)}</h3><div class="signature">${h(body.signature)}</div></div>
    <div class="details">${body.details.map(([key, value]) => detail(key, String(value))).join('')}</div>
    <p class="muted">${h(body.note)}</p>
    <div class="card-actions">
      <button data-trace="upstream" data-trace-id="${h(node.id)}">↑ Upstream</button>
      <button data-trace="downstream" data-trace-id="${h(node.id)}">↓ Downstream</button>
      <button data-why="${h(node.id)}">Why?</button>
    </div>
  </article>`;
}

function semanticSection(heading, list, emptyText) {
  return `<h2>${h(heading)}</h2>${list.length ? `<section class="semantic-grid">${list.map(semanticCard).join('')}</section>` : empty(emptyText)}`;
}

function runtimeBadge() {
  if (!S.r) return badge('not collected', 'dim');
  if (S.r.capture === 'disabled') return badge('disabled', 'warn');
  if (!S.r.retention.complete) return badge('partial', 'warn');
  return badge(S.r.capture);
}

function runtimeSummary() {
  if (!S.r) return `No runtime session is attached. Disconnected: ${S.e.targets.join(', ') || 'all targets'}.`;
  const disconnected = S.e.targets.filter(target => target !== S.r.target);
  return `${S.r.events.length} events · ${S.r.retention.droppedRecords} dropped · attached: ${S.r.target}${disconnected.length ? ` · disconnected: ${disconnected.join(', ')}` : ''}`;
}

function architectureNodes() {
  const accepted = new Set(['application', 'frontend-module', 'backend-service', 'page', 'operation', 'contract', 'extension']);
  return S.e.nodes.filter(node => accepted.has(node.kind));
}

function architectureRanks(list) {
  const rank = new Map();
  const byKind = {
    application: 0,
    'frontend-module': 1,
    'backend-service': 1,
    page: 2,
    contract: 2,
    extension: 2,
    operation: 3,
  };
  list.forEach(node => rank.set(node.id, byKind[node.kind] ?? 3));
  return rank;
}

function focusedModel(rootId, direction = 'both', depth = 3, edgeKind = 'all', maximum = 32) {
  const root = nodeById(rootId) || nodes()[0] || S.e.nodes[0];
  if (!root) return { root: null, nodes: [], edges: [], ranks: new Map() };
  const selected = new Set([root.id]);
  const ranks = new Map([[root.id, 0]]);
  const queue = [{ id: root.id, distance: 0, rank: 0 }];
  while (queue.length && selected.size < maximum) {
    const current = queue.shift();
    if (current.distance >= depth) continue;
    const candidates = [];
    S.e.edges.forEach(edge => {
      if (edgeKind !== 'all' && edge.kind !== edgeKind) return;
      if (direction !== 'upstream' && edge.from === current.id) candidates.push({ id: edge.to, rank: current.rank + 1 });
      if (direction !== 'downstream' && edge.to === current.id) candidates.push({ id: edge.from, rank: current.rank - 1 });
    });
    candidates.sort((left, right) => label(left.id).localeCompare(label(right.id)));
    for (const candidate of candidates) {
      if (selected.has(candidate.id) || selected.size >= maximum) continue;
      selected.add(candidate.id);
      ranks.set(candidate.id, candidate.rank);
      queue.push({ id: candidate.id, distance: current.distance + 1, rank: candidate.rank });
    }
  }
  const visibleNodes = S.e.nodes.filter(node => selected.has(node.id));
  const visibleEdges = S.e.edges.filter(edge => selected.has(edge.from) && selected.has(edge.to) && (edgeKind === 'all' || edge.kind === edgeKind));
  return { root, nodes: visibleNodes, edges: visibleEdges, ranks };
}

function architectureModel() {
  const visibleNodes = architectureNodes();
  const ids = new Set(visibleNodes.map(node => node.id));
  const visibleEdges = S.e.edges.filter(edge => ids.has(edge.from) && ids.has(edge.to));
  const root = visibleNodes.find(node => node.kind === 'application') || visibleNodes[0];
  return { root, nodes: visibleNodes, edges: visibleEdges, ranks: architectureRanks(visibleNodes) };
}

function truncate(value, maximum = 24) { return value.length > maximum ? `${value.slice(0, maximum - 1)}…` : value; }

function graphSvg(model, compact = false) {
  if (!model.root || !model.nodes.length) return empty('No connected semantic relationships match this focus.');
  const rawRanks = [...new Set(model.nodes.map(node => model.ranks.get(node.id) ?? 0))].sort((a, b) => a - b);
  const rankIndex = new Map(rawRanks.map((rank, index) => [rank, index]));
  const columns = rawRanks.map(rank => model.nodes.filter(node => (model.ranks.get(node.id) ?? 0) === rank).sort((a, b) => a.label.localeCompare(b.label)));
  const width = Math.max(compact ? 980 : 1040, columns.length * 270 + 80);
  const height = Math.max(compact ? 390 : 500, Math.max(...columns.map(column => column.length), 1) * 92 + 90);
  const positions = new Map();
  columns.forEach((column, columnIndex) => {
    const gap = height / (column.length + 1);
    column.forEach((node, rowIndex) => {
      positions.set(node.id, { x: 40 + columnIndex * ((width - 250) / Math.max(columns.length - 1, 1)), y: gap * (rowIndex + 1) - 30 });
    });
  });
  const nodeWidth = 210;
  const nodeHeight = 60;
  const edgeMarkup = model.edges.map(edge => {
    const from = positions.get(edge.from);
    const to = positions.get(edge.to);
    if (!from || !to) return '';
    const startX = from.x + nodeWidth;
    const startY = from.y + nodeHeight / 2;
    const endX = to.x;
    const endY = to.y + nodeHeight / 2;
    const midpoint = (startX + endX) / 2;
    const path = `M ${startX} ${startY} C ${midpoint} ${startY}, ${midpoint} ${endY}, ${endX} ${endY}`;
    const focus = edge.from === model.root.id || edge.to === model.root.id ? 'focus' : '';
    return `<path class="graph-edge ${focus}" d="${path}"></path><text class="graph-edge-label" x="${midpoint}" y="${(startY + endY) / 2 - 6}" text-anchor="middle">${h(edge.kind)}</text>`;
  }).join('');
  const nodeMarkup = model.nodes.map(node => {
    const position = positions.get(node.id);
    const rootClass = node.id === model.root.id ? 'root' : '';
    return `<g class="graph-node ${rootClass}" tabindex="0" role="button" aria-label="Inspect ${h(node.label)}" data-id="${h(node.id)}" transform="translate(${position.x} ${position.y})">
      <rect width="${nodeWidth}" height="${nodeHeight}"></rect>
      <text x="13" y="24">${h(truncate(node.label))}</text>
      <text class="node-kind" x="13" y="44">${h(node.kind)}</text>
    </g>`;
  }).join('');
  return `<div class="graph-viewport"><svg class="relation-graph" viewBox="0 0 ${width} ${height}" height="${height}" role="img" aria-label="Focused semantic relationship graph">
    <defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse"><path d="M 0 0 L 10 5 L 0 10 z" fill="#42645b"></path></marker></defs>
    ${edgeMarkup}${nodeMarkup}
  </svg></div><div class="graph-caption"><span>${model.nodes.length} facts · ${model.edges.length} typed relationships</span><span>Click a fact to inspect its evidence</span></div>`;
}

function renderOverview() {
  const findings = S.e.nodes.filter(node => node.kind === 'finding');
  const blockers = findings.filter(node => ['error', 'blocker'].includes(node.attributes?.severity)).length;
  const warnings = findings.filter(node => node.attributes?.severity === 'warning').length;
  const fresh = S.status?.source?.fresh !== false;
  const architecture = architectureModel();
  $('#main').innerHTML = title('Application overview', 'A deterministic map of what this application declares, resolves, and has observed.') +
    `<section class="grid">
      <div class="card metric"><small>Semantic facts</small><strong>${S.e.nodes.length}</strong><span class="muted">${S.e.edges.length} relationships</span></div>
      <div class="card metric"><small>Targets</small><strong>${S.e.targets.length}</strong><span class="muted">${h(S.e.targets.join(' · ') || 'not declared')}</span></div>
      <div class="card metric"><small>Extensions</small><strong>${count('extension')}</strong><span class="muted">${count('permission')} authority facts</span></div>
      <div class="card metric"><small>Readiness</small><strong>${blockers ? badge(`${blockers} blockers`, 'bad') : badge('No blockers')}</strong><span class="muted">${warnings} warnings</span></div>
    </section>
    <h2>Application architecture</h2>
    <section class="card architecture-map">
      <div class="architecture-head"><div><b>How this application is connected</b><p class="muted">Frontend, backend, contracts and imperative code through compiler-proven edges.</p></div><button class="quiet-button" data-focus-graph="${h(architecture.root?.id || '')}">Explore full graph →</button></div>
      ${graphSvg(architecture, true)}
    </section>
    <h2>Evidence health</h2>
    <section class="grid two">
      <div class="card"><div class="row between"><b>Static graph</b>${badge(fresh ? 'verified' : 'stale', fresh ? '' : 'warn')}</div><p class="muted">Revision <code>${h(S.e.graphRevision.slice(0, 16))}…</code></p><p>${fresh ? h(S.e.completeness.staticEvidence) : h(S.status.source.error || 'The previous valid graph is being shown.')}</p></div>
      <div class="card"><div class="row between"><b>Runtime attachment</b>${runtimeBadge()}</div><p class="muted">${h(runtimeSummary())}</p><p>${S.r && S.r.graphRevision !== S.e.graphRevision ? badge('Graph revision mismatch', 'bad') : 'Static and observed identities remain separate.'}</p></div>
    </section>
    <h2>Diagnostics</h2>${S.e.diagnostics.length ? S.e.diagnostics.map(diagnostic => `<div class="card diagnostic"><b>${h(diagnostic.code)}</b> ${h(diagnostic.message)}</div>`).join('') : empty('No Inspector diagnostics.')}`;
}

function importantGraphRoots() {
  const accepted = new Set(['application', 'page', 'action', 'state', 'operation', 'contract', 'extension', 'extension-export', 'dependency-environment', 'third-party-package', 'release', 'data-model', 'field', 'permission', 'finding']);
  return nodes(S.e.nodes.filter(node => accepted.has(node.kind))).sort((a, b) => a.label.localeCompare(b.label));
}

function renderGraph() {
  const roots = importantGraphRoots();
  if (!S.graph.root || !nodeById(S.graph.root)) S.graph.root = roots.find(node => node.kind === 'application')?.id || roots[0]?.id;
  const model = focusedModel(S.graph.root, S.graph.direction, S.graph.depth, S.graph.edge);
  const edgeKinds = [...new Set(S.e.edges.map(edge => edge.kind))].sort();
  $('#main').innerHTML = title('Relationship graph', 'Focus one semantic fact, then follow only the relationships that answer your question.') +
    `<section class="card graph-workbench">
      <div class="graph-toolbar">
        <label>Focus fact<select id="graph-root">${roots.map(node => `<option value="${h(node.id)}" ${node.id === S.graph.root ? 'selected' : ''}>${h(node.kind)} · ${h(node.label)}</option>`).join('')}</select></label>
        <div><div class="kind">Direction</div><div class="segmented">${['upstream', 'both', 'downstream'].map(direction => `<button data-graph-direction="${direction}" class="${S.graph.direction === direction ? 'active' : ''}">${direction}</button>`).join('')}</div></div>
        <label>Depth<select id="graph-depth">${[1, 2, 3, 4, 5].map(depth => `<option ${S.graph.depth === depth ? 'selected' : ''}>${depth}</option>`).join('')}</select></label>
      </div>
      <div class="pillbar inset-pillbar">${['all', ...edgeKinds].map(kind => `<button data-graph-edge="${h(kind)}" class="${S.graph.edge === kind ? 'active' : ''}">${h(kind)}</button>`).join('')}</div>
      ${graphSvg(model)}
    </section>
    <h2>Focused relationships</h2>${edgeTable(model.edges)}`;
  $('#graph-root').onchange = event => { S.graph.root = event.target.value; render(); };
  $('#graph-depth').onchange = event => { S.graph.depth = Number(event.target.value); render(); };
  $$('[data-graph-direction]').forEach(button => button.onclick = () => { S.graph.direction = button.dataset.graphDirection; render(); });
  $$('[data-graph-edge]').forEach(button => button.onclick = () => { S.graph.edge = button.dataset.graphEdge; render(); });
}

function edgeTable(edges) {
  if (!edges.length) return empty('No typed relationships match this focus.');
  return `<table class="matrix"><thead><tr><th>From</th><th>Relationship</th><th>To</th></tr></thead><tbody>${edges.map(edge => `<tr><td><button class="quiet-button" data-id="${h(edge.from)}">${h(label(edge.from))}</button></td><td>${badge(edge.kind, 'dim')}</td><td><button class="quiet-button" data-id="${h(edge.to)}">${h(label(edge.to))}</button></td></tr>`).join('')}</tbody></table>`;
}

function renderSource() {
  const sourceNodes = nodes(S.e.nodes.filter(node => kinds.source.includes(node.kind)));
  const units = sourceNodes.filter(node => node.kind === 'source-unit' || node.kind === 'frontend-module');
  const bindings = sourceNodes.filter(node => !units.includes(node));
  $('#main').innerHTML = title('Source explorer', 'Open compiler-owned source evidence with exact spans, semantic context and editor handoff.') +
    semanticSection('Source units', units, 'No source units match the current filters.') +
    `<h2>Implementation bindings</h2>${table(bindings)}`;
}

function renderUi() {
  const visible = nodes(S.e.nodes.filter(node => kinds.ui.includes(node.kind)));
  const states = visible.filter(node => node.kind === 'state');
  const actions = visible.filter(node => node.kind === 'action');
  const presentation = visible.filter(node => !['state', 'action'].includes(node.kind));
  $('#main').innerHTML = title('UI, state & presentation', 'Semantic UI, state access, actions, accessibility, styles and responsive target behavior.') +
    semanticSection('State', states, 'No declared state matches the current filters.') +
    semanticSection('Actions', actions, 'No actions match the current filters.') +
    `<h2>Presentation facts</h2>${table(S.kind ? presentation.filter(node => node.kind === S.kind) : presentation)}` + kindFilter(presentation);
}

function renderBackend() {
  const visible = nodes(S.e.nodes.filter(node => kinds.backend.includes(node.kind)));
  const operations = visible.filter(node => node.kind === 'operation');
  const remaining = visible.filter(node => node.kind !== 'operation');
  $('#main').innerHTML = title('Backend', 'Operations as callable contracts, plus domain data, policy, cache, validation, streams and implementation bindings.') +
    semanticSection('Operations', operations, 'No backend operations match the current filters.') +
    `<h2>Domain and policy evidence</h2>${table(S.kind ? remaining.filter(node => node.kind === S.kind) : remaining)}` + kindFilter(remaining);
}

function kindFilter(list) {
  const values = [...new Set(list.map(node => node.kind))];
  if (!values.length) return '';
  return `<div class="pillbar">${values.map(kind => `<button class="${S.kind === kind ? 'active' : ''}" data-filter-kind="${h(kind)}">${h(kind)} <b>${list.filter(node => node.kind === kind).length}</b></button>`).join('')}</div>`;
}

function capabilityMatrix() {
  const permissions = nodes(S.e.nodes.filter(node => node.kind === 'permission'));
  if (!permissions.length) return empty();
  return `<table class="matrix"><thead><tr><th>Stage</th><th>Capability</th><th>Access</th><th>Targets</th></tr></thead><tbody>${permissions.map(node => {
    const permission = node.attributes.permission || {};
    return `<tr tabindex="0" data-id="${h(node.id)}"><td>${badge(node.attributes.stage || 'declared', node.attributes.stage === 'effective' ? '' : 'dim')}</td><td><b>${h(permission.kind || node.label)}</b><br><span class="source">${h([permission.scope, permission.path].filter(Boolean).join('.'))}</span></td><td>${h(permission.access || 'group')}</td><td>${h((node.targets || []).join(', ') || 'all')}</td></tr>`;
  }).join('')}</tbody></table>`;
}

function renderExtensions() {
  const extensions = nodes(S.e.nodes.filter(node => node.kind === 'extension'));
  $('#main').innerHTML = title('Extensions', 'Sandboxed imperative code as verified packages with explicit authority, provenance and target coverage.') +
    semanticSection('Executable extensions', extensions, 'No executable extension matches the current filters.') +
    `<h2>Capability surface</h2>${capabilityMatrix()}`;
}

function renderDependencies() {
  const visible = nodes(S.e.nodes.filter(node => kinds.dependencies.includes(node.kind)));
  const contracts = visible.filter(node => node.kind === 'contract');
  const packages = visible.filter(node => node.kind === 'package');
  const thirdParty = visible.filter(node => node.kind === 'third-party-package');
  const environments = visible.filter(node => node.kind === 'dependency-environment');
  const releases = visible.filter(node => ['release', 'implementation-binding', 'artifact'].includes(node.kind));
  const metadata = visible.filter(node => ['license', 'advisory', 'build-script', 'release-policy'].includes(node.kind));
  $('#main').innerHTML = title('Dependency closure', 'Every contract, executable module, language package, environment and release remains connected to authority and provenance.') +
    `<section class="grid three"><div class="card metric"><small>Contracts</small><strong>${count('contract')}</strong></div><div class="card metric"><small>Third-party packages</small><strong>${count('third-party-package')}</strong></div><div class="card metric"><small>Executable extensions</small><strong>${count('extension')}</strong></div></section>` +
    semanticSection('Pinned build environments', environments, 'No authored dependency environments were resolved.') +
    semanticSection('Third-party code', thirdParty, 'No third-party language packages were locked.') +
    semanticSection('Executable provenance', releases, 'No verified release artifacts were discovered.') +
    `<h2>Contracts and Axiom packages</h2>${table([...contracts, ...packages])}` +
    `<h2>Supply-chain metadata</h2>${table(metadata)}`;
}

function renderSecurity() {
  const findings = nodes(S.e.nodes.filter(node => node.kind === 'finding'));
  const permissions = nodes(S.e.nodes.filter(node => node.kind === 'permission'));
  const operations = nodes(S.e.nodes.filter(node => node.kind === 'operation' && node.attributes?.exposure));
  const supplyChain = nodes(S.e.nodes.filter(node => ['release-policy', 'advisory', 'build-script'].includes(node.kind)));
  $('#main').innerHTML = title('Security & authority', 'Requested, granted and effective authority with explicit evidence—never an opaque score.') +
    `<section class="grid three"><div class="card metric"><small>Capability facts</small><strong>${permissions.length}</strong></div><div class="card metric"><small>Exposed operations</small><strong>${operations.length}</strong></div><div class="card metric"><small>Findings</small><strong>${findings.length}</strong></div></section>
    <h2>Capability matrix</h2>${capabilityMatrix()}
    ${semanticSection('Backend exposure', operations, 'No exposed operations match the current filters.')}
    ${semanticSection('Supply-chain policy', supplyChain, 'No third-party release policy or supply-chain concerns were declared.')}
    <h2>Findings</h2>${table(findings)}`;
}

function renderRuntime() {
  const runtime = S.r;
  $('#main').innerHTML = title('Observed runtime', 'Portable, redacted events remain distinct from declared and resolved evidence.') + (!runtime ?
    empty('Runtime capture has not been collected. Start with `laxiom inspect record --target TARGET`.') :
    `<section class="grid"><div class="card metric"><small>Capture</small><strong>${runtimeBadge()}</strong></div><div class="card metric"><small>Retained</small><strong>${runtime.retention.retainedRecords}</strong></div><div class="card metric"><small>Dropped</small><strong>${runtime.retention.droppedRecords}</strong></div><div class="card metric"><small>Target</small><strong>${h(runtime.target)}</strong></div></section>
    ${runtime.capture === 'disabled' ? '<div class="card diagnostic"><b>Capture disabled</b><p>Absence of events is not evidence that no activity occurred.</p></div>' : ''}
    <h2>Event timeline</h2><div class="card timeline">${runtime.events.length ? runtime.events.map(event => `<div class="event ${h(event.kind)}"><time>#${event.sequence} · ${h(event.source)} · ${h(event.traceId || 'no trace')}</time><br><b>${h(event.kind)}</b> ${badge(event.outcome, event.kind === 'denial' || event.kind === 'error' ? 'bad' : 'dim')}<p class="muted">${h(JSON.stringify(event.attributes))}</p></div>`).join('') : empty(runtime.capture === 'disabled' ? 'Capture was disabled; no observation claim can be made.' : 'Capture is enabled, but no runtime event has been observed.')}</div>
    <h2>Retention</h2><div class="card"><div class="row between"><b>${runtime.retention.complete ? 'Complete evidence' : 'Partial evidence'}</b><span>${runtime.retention.retainedRecords}/${runtime.retention.maximumRecords || 'unbounded'}</span></div><progress class="retention-progress" max="100" value="${Math.min(100, runtime.retention.retainedRecords / (runtime.retention.maximumRecords || runtime.retention.retainedRecords || 1) * 100)}"></progress><p class="muted">Values remain redacted; hashes, shapes, paths, decisions and identities are retained.</p></div>`);
}

function renderLive() {
  const live = S.live || { targets: [], selectedEvidence: [] };
  $('#main').innerHTML = title('Live semantic inspection', 'Alt-click on Web or long-press on iOS/Android to select the same compiler-owned semantic identity.') + (live.targets.length ? live.targets.map(target => {
    const selection = target.selection;
    const evidence = live.selectedEvidence.find(value => value.target === target.handshake.target);
    const node = evidence?.node;
    const stateHistory = target.stateHistory || [];
    const state = selection?.traceId ? stateHistory.filter(value => value.traceId === selection.traceId) : stateHistory.slice(-8);
    const causal = selection?.traceId ? (target.causalHistory || []).filter(value => value.traceId === selection.traceId) : (target.causalHistory || []).slice(-10);
    return `<section class="card card-gap"><div class="row between"><div><span class="eyebrow">${h(target.handshake.target)}</span><h2>${h(target.handshake.host)} ${h(target.handshake.hostVersion)}</h2></div>${target.exactGraph ? badge('exact graph') : badge('graph mismatch', 'bad')}</div>
      ${!target.exactGraph ? '<div class="card diagnostic"><b>Correlation refused</b><p>This target is running a different graph revision. Reload it before interpreting runtime causes.</p></div>' : ''}
      ${selection ? `<div class="grid two"><div><h2>Selected element</h2><p><b>${h(node?.label || selection.semanticId)}</b></p><p class="source">${h(selection.semanticId)}</p><p>${badge(node?.kind || 'rendered element')} ${badge(selection.reason, 'dim')}</p><div class="row"><button class="primary-button" data-live-command="highlight" data-target="${h(target.handshake.target)}" data-session="${h(target.handshake.sessionId)}" data-semantic="${h(selection.semanticId)}">Highlight</button><button class="quiet-button" data-id="${h(selection.semanticId)}">Inspect evidence</button></div></div><div><h2>Why current?</h2><p>Disabled: <b>${h(selection.presentation.disabled)}</b> · Visible: <b>${h(selection.presentation.visible)}</b> · Value: <b>${h(selection.presentation.valueShape || 'redacted')}</b></p><p>State dependencies: ${(selection.stateDependencies || []).map(value => badge(label(value), 'dim')).join(' ') || 'none declared'}</p><p>Causal trace: <span class="source">${h(selection.traceId || 'no matching observed trace')}</span></p></div></div>
      <h2>Causal chain</h2>${causal.length ? `<div class="timeline">${causal.map(value => `<div class="event"><time>#${value.sequence} · ${h(value.traceId)}</time><br><b>${h(value.kind)}</b> ${h(label(value.semanticId))} ${badge(value.outcome, 'dim')}</div>`).join('')}</div>` : empty('No matching user event → action → operation/extension → patch → rerender chain was observed.')}
      <h2>State revision history</h2>${state.length ? `<div class="timeline">${state.map(value => `<div class="event ${value.decision}"><time>#${value.revisionBefore} → #${value.revisionAfter}</time><br><b>${h(label(value.stateSemanticId))}</b> ${badge(value.decision, value.decision === 'applied' ? '' : 'warn')}<p class="muted">Writer ${h(label(value.writerSemanticId))} · authorized ${h(value.authorized)} · validated ${h(value.validated)} · paths ${(value.changedPaths || []).map(h).join(', ')}</p></div>`).join('')}</div>` : empty('No matching state revision evidence has been retained.')}` : empty('Target connected. Select an element in the running application.')}</section>`;
  }).join('') : empty('No development target is connected. Start Inspector first, then run the frontend target.'));
  $$('[data-live-command]').forEach(button => button.onclick = async () => {
    const target = live.targets.find(value => value.handshake.target === button.dataset.target && value.handshake.sessionId === button.dataset.session);
    await api('/api/v1/live/commands', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ target: button.dataset.target, sessionId: button.dataset.session, graphRevision: target.handshake.graphRevision, kind: button.dataset.liveCommand, semanticId: button.dataset.semantic }) });
    toast('Highlight command sent');
  });
}

function renderChanges() {
  const report = S.changes || { summary: { total: 0, breaking: 0, approvalRequired: 0, warnings: 0, informational: 0 }, changes: [], runtimeChanges: [] };
  const all = [...(report.changes || []), ...(report.runtimeChanges || [])];
  $('#main').innerHTML = title('Semantic changes', 'Behavioral change classified by axiom-change-policy/v1—not merely source lines.') +
    `<section class="grid"><div class="card metric"><small>Breaking</small><strong>${report.summary.breaking}</strong></div><div class="card metric"><small>Approval required</small><strong>${report.summary.approvalRequired}</strong></div><div class="card metric"><small>Warnings</small><strong>${report.summary.warnings}</strong></div><div class="card metric"><small>Informational</small><strong>${report.summary.informational}</strong></div></section>
    <h2>Behavioral diff</h2>${all.length ? `<table class="matrix"><thead><tr><th>Impact</th><th>Domain</th><th>Change</th><th>Evidence</th></tr></thead><tbody>${all.map(change => `<tr><td>${badge(change.impact, change.impact === 'breaking' ? 'bad' : change.impact === 'approval-required' || change.impact === 'warning' ? 'warn' : 'dim')}</td><td>${h(change.domain)}</td><td><b>${h(change.label)}</b><br><span class="muted">${h(change.reason)}</span><br><span class="source">${h(change.semanticId)}</span></td><td>${(change.evidence || []).map(value => h(value.path || value.kind)).join('<br>') || 'canonical graph'}</td></tr>`).join('')}</tbody></table>` : empty('No baseline snapshot is available, or no semantic behavior changed. Create snapshots to enable this view.')}
    <h2>Policy</h2><div class="card"><b>${h(report.policy)}</b><p class="muted">CI can fail independently on breaking, approval-required, permission-increase or warning classes. Timing-only runtime differences are excluded.</p></div>`;
}

function humanAnswer(answer) {
  if (answer.status === 'unsupported') return answer.summary;
  const kind = answer.request?.intent?.kind;
  const paths = answer.paths?.length || 0;
  if (kind === 'execution') return `${answer.summary} The visual trace below follows compiler-proven execution edges from the selected UI fact. ${paths ? `${paths} bounded path${paths === 1 ? '' : 's'} support this explanation.` : 'No connected execution path was returned.'}`;
  if (kind === 'writers') return `${answer.summary} The visual traces below separate the source fact from its requested, granted and target-effective authority evidence.`;
  if (kind === 'network-access') return `${answer.summary} Requested, granted and target-effective authority remain separate in the trace.`;
  if (kind === 'why') return `${answer.summary} Incoming relationships explain what establishes this fact; outgoing relationships show what it authorizes or affects.`;
  if (kind === 'changes') return `${answer.summary} These are semantic changes over canonical identities, not a text-only diff.`;
  return answer.summary;
}

function answerTitle(answer) {
  if (answer.status === 'unsupported') return 'I cannot answer that deterministically yet';
  const intent = answer.request?.intent || {};
  if (intent.kind === 'writers') return `Who can modify ${intent.selector}?`;
  if (intent.kind === 'network-access') return `Why can ${intent.selector} access the network?`;
  if (intent.kind === 'execution') return `What executes from ${intent.selector}?`;
  if (intent.kind === 'why') return `Why is ${intent.selector} connected?`;
  if (intent.kind === 'changes') return 'What changed semantically?';
  return 'Deterministic explanation';
}

function answerTrace(path) {
  const steps = path.nodeIds || [];
  return `<div class="trace-scroll"><div class="trace-strip">${steps.map((id, index) => {
    const node = nodeById(id) || (S.answer?.facts || []).find(fact => fact.id === id) || { id, label: id, kind: 'fact' };
    const edge = index < (path.edgeIds || []).length ? edgeById(path.edgeIds[index]) : null;
    return `<div class="trace-step"><div class="trace-node" tabindex="0" data-id="${h(id)}"><span class="kind">${h(node.kind)}</span><b>${h(node.label)}</b></div>${edge ? `<div class="trace-arrow">${h(edge.kind)}</div>` : ''}</div>`;
  }).join('')}</div></div>`;
}

function renderAsk() {
  const answer = S.answer;
  const jevEnabled = Boolean(S.status?.planners?.jevRemote);
  const planning = answer?.planning;
  $('#main').innerHTML = title('Ask Axiom', 'Ask a bounded question and receive a deterministic explanation backed by a visual semantic trace.') +
    `<form id="question-form" class="question-form"><input id="question" maxlength="512" required placeholder="What code can mess with the checkout total?" aria-label="Question about this application"><label class="planner-select">Planner<select id="question-planner"><option value="deterministic">Local deterministic</option>${jevEnabled ? '<option value="jev">Jev · remote</option>' : ''}</select></label><button>Ask</button></form>
    <p class="muted">Supported evidence queries: writers, network access, execution, why, and semantic changes. ${jevEnabled ? 'Jev may interpret paraphrases, but it only selects a bounded request; Axiom still validates and answers it locally.' : 'Remote Jev planning is disabled for this session.'}</p>` +
    (answer ? `<section class="answer-hero"><div class="row between"><span class="eyebrow">${planning ? 'Jev-planned · deterministic evidence' : 'Deterministic answer'}</span>${badge(answer.status, answer.status === 'unsupported' ? 'warn' : '')}</div><h2>${h(answerTitle(answer))}</h2><p>${h(humanAnswer(answer))}</p><div class="row"><span class="source">Graph ${h(answer.graphRevision.slice(0, 16))}…</span><span>${answer.facts?.length || 0} facts · ${answer.paths?.length || 0} paths</span></div></section>
      ${planning ? `<section class="planning-card"><div class="row between"><div><span class="eyebrow">Probabilistic planning receipt</span><h3>Jev through Vercel AI Gateway</h3></div>${badge(planning.decision?.accepted ? 'accepted' : planning.deterministicFallback ? 'local fallback' : 'abstained', planning.decision?.accepted ? '' : 'warn')}</div><div class="details"><div class="detail"><small>Intent</small><b>${h(planning.decision?.intent || 'unknown')} · ${h(Math.round((planning.decision?.intentProbability || 0) * 100))}%</b></div><div class="detail"><small>Candidate</small><b>${h(planning.decision?.selector || 'none')}</b></div><div class="detail"><small>Selector source</small><b>${h(planning.decision?.selectorSource || 'jev')}</b></div><div class="detail"><small>Candidate probability</small><b>${planning.decision?.selectorProbability == null ? 'exact match' : `${h(Math.round(planning.decision.selectorProbability * 100))}%`}</b></div><div class="detail"><small>Shortlist</small><b>${h(planning.candidateCount)} semantic facts</b></div></div><p class="muted">${h(planning.decision?.reason)} The question and bounded labels were sent remotely; source, attributes, edges, and the evidence graph remained local.</p><p class="source">Audit receipt: ${h(planning.receiptPath)}</p></section>` : ''}
      <h2>Visual trace</h2>${answer.paths?.length ? `<div class="trace-list">${answer.paths.slice(0, 8).map(answerTrace).join('')}</div>` : empty('No deterministic relationship path was returned for this question.')}
      <h2>Supporting semantic facts</h2><section class="semantic-grid">${(answer.facts || []).slice(0, 12).map(answerFact => {
        const node = nodeById(answerFact.id);
        return node ? semanticCard(node) : `<article class="card"><span class="kind">${h(answerFact.kind)}</span><h3>${h(answerFact.label)}</h3><p class="source">${h(answerFact.id)}</p></article>`;
      }).join('')}</section>
      <section class="grid two top-gap"><div class="card"><h3>Completeness</h3><p>${(answer.completeness?.uncertainty || []).map(value => badge(value, 'dim')).join(' ') || badge('complete')}</p></div><div class="card"><h3>Boundaries</h3><ul>${(answer.limits || []).map(value => `<li>${h(value)}</li>`).join('')}</ul></div></section>
      ${answer.validatedQuery ? `<details><summary>Show validated query</summary><pre>${h(JSON.stringify(answer.validatedQuery, null, 2))}</pre></details>` : ''}` : '');
  const form = $('#question-form');
  form.onsubmit = async event => {
    event.preventDefault();
    const button = form.querySelector('button');
    const question = $('#question').value;
    const planner = $('#question-planner').value;
    button.disabled = true;
    button.textContent = 'Resolving…';
    try {
      S.answer = await api('/api/v1/questions', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ question, planner }) });
      renderAsk();
      bindInteractions();
    } catch (error) {
      toast(error.message);
      button.disabled = false;
      button.textContent = 'Ask';
    }
  };
}

function relationshipMarkup(edges, direction) {
  if (!edges.length) return empty(`No ${direction} relationships are declared.`);
  return `<div class="relationship-group">${edges.slice(0, 30).map(edge => {
    const id = direction === 'upstream' ? edge.from : edge.to;
    return `<div class="relationship" tabindex="0" data-id="${h(id)}"><div><b>${h(label(id))}</b><br><span class="kind">${h(nodeById(id)?.kind || 'fact')}</span></div><small>${direction === 'upstream' ? `${h(edge.kind)} → this` : `this → ${h(edge.kind)}`}</small></div>`;
  }).join('')}</div>`;
}

function whyText(node) {
  const incoming = inbound(node.id);
  const outgoing = outbound(node.id);
  if (node.kind === 'permission') {
    const stage = node.attributes?.stage || 'declared';
    return `This ${stage} permission exists because ${incoming.length || 'no'} upstream authority relationship${incoming.length === 1 ? '' : 's'} establish it. Its ${outgoing.length || 'no'} downstream relationship${outgoing.length === 1 ? '' : 's'} show the state, resource or operation it can affect.`;
  }
  return `${incoming.length} upstream relationship${incoming.length === 1 ? '' : 's'} establish or use this fact. ${outgoing.length} downstream relationship${outgoing.length === 1 ? '' : 's'} show what it declares, calls, authorizes or affects.`;
}

function sourcePreviewMarkup(preview) {
  const editor = `vscode://file/${encodeURI(preview.absolutePath)}:${preview.startLine}:${preview.startColumn}`;
  const verified = preview.digestMatches === null ? badge('digest not supplied', 'dim') : preview.digestMatches ? badge('source verified') : badge('source changed', 'warn');
  return `<div class="source-view"><div class="source-toolbar"><div><div class="path">${h(preview.path)}:${preview.startLine}:${preview.startColumn}</div>${verified}</div><div class="row"><button data-copy-path="${h(preview.absolutePath)}">Copy path</button><a href="${h(editor)}">Open in Editor ↗</a></div></div><pre class="code-lines">${preview.lines.map(line => `<div class="code-line ${line.highlighted ? 'highlight' : ''}"><span class="line-number">${line.number}</span><code>${h(line.text) || ' '}</code></div>`).join('')}</pre></div>`;
}

function renderDrawer(node, preview = null, previewError = null) {
  if (S.selected !== node.id) return;
  const evidence = node.evidence || [];
  $('#drawer').innerHTML = `<button class="drawer-close" aria-label="Close">×</button>
    <span class="kind">${h(node.kind)}</span><h2>${h(node.label)}</h2><p class="source">${h(node.id)}</p>
    <div class="trace-actions"><button class="primary" data-trace="upstream" data-trace-id="${h(node.id)}">↑ Trace upstream</button><button class="primary" data-trace="downstream" data-trace-id="${h(node.id)}">↓ Trace downstream</button><button data-trace="both" data-trace-id="${h(node.id)}">↕ Trace both</button></div>
    <div class="kv"><span>Truth layer</span><b>${h(node.layer)}</b></div><div class="kv"><span>Verification</span><b>${h(node.verification)}</b></div><div class="kv"><span>Targets</span><b>${h((node.targets || []).join(', ') || 'all')}</b></div>
    <h2>Why?</h2><div class="why-panel"><p>${h(whyText(node))}</p><p class="muted">Every relationship below is backed by the same canonical graph revision.</p></div>
    <h2>Source evidence</h2>${preview ? sourcePreviewMarkup(preview) : previewError ? `<div class="source-state">${h(previewError)}</div>` : evidence.some(item => item.path) ? '<div class="source-state">Loading verified source lines…</div>' : empty('No local source evidence was supplied by this producer.')}
    ${evidence.length ? `<details><summary>All evidence references (${evidence.length})</summary>${evidence.map(item => `<div class="card evidence-card"><b>${h(item.kind)}</b><p class="source">${h(item.path || 'content-addressed artifact')} ${h(item.detail || '')}</p>${item.sha256 ? `<p class="muted mono">sha256 ${h(item.sha256)}</p>` : ''}</div>`).join('')}</details>` : ''}
    <h2>Upstream</h2>${relationshipMarkup(inbound(node.id), 'upstream')}
    <h2>Downstream</h2>${relationshipMarkup(outbound(node.id), 'downstream')}
    <details><summary>Canonical raw evidence</summary><pre>${h(JSON.stringify(node, null, 2))}</pre></details>`;
  $('#drawer').classList.add('open');
  $('#drawer').setAttribute('aria-hidden', 'false');
  $('#drawer .drawer-close').onclick = closeDrawer;
  bindInteractions();
}

async function openNode(id) {
  const node = nodeById(id);
  if (!node) return;
  S.selected = id;
  renderDrawer(node);
  const evidenceIndex = (node.evidence || []).findIndex(item => item.path);
  if (evidenceIndex >= 0) {
    const cacheKey = `${id}:${evidenceIndex}`;
    try {
      let preview = S.sourceCache.get(cacheKey);
      if (!preview) {
        preview = await api(`/api/v1/source?nodeId=${encodeURIComponent(id)}&evidenceIndex=${evidenceIndex}`);
        S.sourceCache.set(cacheKey, preview);
      }
      renderDrawer(node, preview);
    } catch (error) {
      renderDrawer(node, null, `Source preview unavailable: ${error.message}`);
    }
  }
  const url = new URL(location);
  url.searchParams.set('node', id);
  history.replaceState(null, '', url);
}

function closeDrawer() {
  S.selected = null;
  $('#drawer').classList.remove('open');
  $('#drawer').setAttribute('aria-hidden', 'true');
  const url = new URL(location);
  url.searchParams.delete('node');
  history.replaceState(null, '', url);
}

function focusGraph(id, direction = 'both') {
  S.graph.root = id;
  S.graph.direction = direction;
  S.graph.depth = 4;
  S.graph.edge = 'all';
  location.hash = 'graph';
  if (S.view === 'graph') render();
}

function bindInteractions() {
  $$('[data-id]').forEach(element => {
    element.onclick = event => {
      if (event.target !== element && event.target.closest('button,a')) return;
      openNode(element.dataset.id);
    };
    element.onkeydown = event => {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        openNode(element.dataset.id);
      }
    };
  });
  $$('[data-trace]').forEach(button => button.onclick = event => {
    event.stopPropagation();
    focusGraph(button.dataset.traceId, button.dataset.trace);
  });
  $$('[data-why]').forEach(button => button.onclick = event => {
    event.stopPropagation();
    openNode(button.dataset.why);
  });
  $$('[data-focus-graph]').forEach(button => button.onclick = () => focusGraph(button.dataset.focusGraph, 'downstream'));
  $$('[data-filter-kind]').forEach(button => button.onclick = () => { S.kind = S.kind === button.dataset.filterKind ? '' : button.dataset.filterKind; render(); });
  $$('[data-copy-path]').forEach(button => button.onclick = async () => {
    await navigator.clipboard.writeText(button.dataset.copyPath);
    toast('Source path copied');
  });
}

function render() {
  const next = (location.hash || '#overview').slice(1);
  if (next !== S.view) {
    S.kind = '';
    if (S.selected) closeDrawer();
  }
  S.view = next;
  $$('#nav a').forEach(anchor => anchor.classList.toggle('active', anchor.dataset.view === S.view));
  const renderer = {
    overview: renderOverview,
    graph: renderGraph,
    source: renderSource,
    ui: renderUi,
    backend: renderBackend,
    extensions: renderExtensions,
    dependencies: renderDependencies,
    security: renderSecurity,
    runtime: renderRuntime,
    live: renderLive,
    changes: renderChanges,
    ask: renderAsk,
  }[S.view] || renderOverview;
  renderer();
  bindInteractions();
  $('#main').focus({ preventScroll: true });
}

function toast(message) {
  const element = $('#toast');
  element.textContent = message;
  element.classList.add('show');
  setTimeout(() => element.classList.remove('show'), 1800);
}

async function boot() {
  try {
    [S.e, S.r, S.status, S.live, S.changes] = await Promise.all([
      api('/api/v1/evidence'), api('/api/v1/runtime'), api('/api/v1/status'), api('/api/v1/live'), api('/api/v1/changes'),
    ]);
    $('#revision').textContent = `${S.e.graphRevision.slice(0, 16)}…`;
    S.e.targets.forEach(target => $('#target').insertAdjacentHTML('beforeend', `<option>${h(target)}</option>`));
    $('#loading').classList.add('hidden');
    render();
    const id = new URL(location).searchParams.get('node');
    if (id) openNode(id);
    const events = new EventSource(`/api/v1/events?token=${encodeURIComponent(token)}`);
    events.onmessage = async event => {
      const update = JSON.parse(event.data);
      if (update.type === 'evidence-updated') {
        [S.e, S.status, S.changes] = await Promise.all([api('/api/v1/evidence'), api('/api/v1/status'), api('/api/v1/changes')]);
        S.sourceCache.clear();
        toast('Evidence updated');
        render();
      }
      if (update.type === 'runtime-updated') { S.r = await api('/api/v1/runtime'); toast('Runtime updated'); render(); }
      if (update.type.startsWith('live-')) { S.live = await api('/api/v1/live'); render(); }
      if (update.type === 'diagnostic') { S.status = await api('/api/v1/status'); toast(`Source is stale: ${update.message}`); render(); }
    };
    events.onerror = () => { $('#connection').textContent = '● Reconnecting'; $('#connection').style.color = 'var(--amber)'; };
  } catch (error) {
    $('#loading').innerHTML = `<div class="card"><h2>Inspector could not load</h2><p>${h(error.message)}</p></div>`;
  }
}

addEventListener('hashchange', render);
$('#target').onchange = render;
$('#layer').onchange = render;
$('#search').oninput = event => { S.query = event.target.value; render(); };
addEventListener('keydown', event => {
  if (event.key === '/' && !['INPUT', 'SELECT'].includes(document.activeElement.tagName)) { event.preventDefault(); $('#search').focus(); }
  if (event.key === 'Escape') closeDrawer();
  if ((event.key === 'j' || event.key === 'ArrowDown') && !['INPUT', 'SELECT'].includes(document.activeElement.tagName)) {
    const elements = $$('[data-id]');
    const index = elements.indexOf(document.activeElement);
    elements[Math.min(elements.length - 1, index + 1)]?.focus();
  }
  if ((event.key === 'k' || event.key === 'ArrowUp') && !['INPUT', 'SELECT'].includes(document.activeElement.tagName)) {
    const elements = $$('[data-id]');
    const index = elements.indexOf(document.activeElement);
    elements[Math.max(0, index - 1)]?.focus();
  }
});

boot();
