// Copies the semantic service's JSON Schemas and agent skill from the Acore
// compiler source into public/, so the site serves them as static files:
//
//   /semantic-service/schemas/<file>     protocol/semantic/schemas/, same layout
//   /semantic-service/schemas/index.json generated list of every schema file
//   /semantic-service/skill/<file>       skills/acore-semantic/ (SKILL.md, references/*.md)
//
// Maintainer tool: the public pages never point at it.
//
//   node scripts/sync-semantic-service-files.mjs --source <acore checkout> [--check]
//
// The copies are byte-identical to the source except for the lines listed in
// FIXES, which replace maintainer references (internal plans, repository
// paths, source symbols) with public wording. Every fix must match exactly the
// stated number of times, so a source change that touches a fixed line fails
// the run until the fix is reviewed. The copies are then scanned for internal
// names, every JSON file must parse, and every $ref must resolve among the
// hosted schema files. The agent skill page's copy of SKILL.md (a generated
// region) is rewritten from the hosted file. --check writes nothing and fails
// when public/ or that region differs from what this script would write.
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, posix, relative, resolve, sep } from 'node:path';
import process from 'node:process';

const args = process.argv.slice(2);
const sourceIndex = args.indexOf('--source');
const source = sourceIndex >= 0 ? args[sourceIndex + 1] : undefined;
const checkOnly = args.includes('--check');
if (!source) {
  console.error('usage: sync-semantic-service-files.mjs --source <acore checkout> [--check]');
  process.exit(2);
}

const docsRoot = resolve(import.meta.dirname, '..');
const publicRoot = join(docsRoot, 'public', 'semantic-service');
const siteOrigin = 'https://docs.axiomcore.dev';
const SETS = [
  { name: 'schemas', from: join(source, 'protocol', 'semantic', 'schemas'), include: (path) => path.endsWith('.json') },
  { name: 'skill', from: join(source, 'skills', 'acore-semantic'), include: (path) => path === 'SKILL.md' || /^references\/[^/]+\.md$/.test(path) },
];

// [hosted path, text in the source, public replacement, expected matches]
// Line fixes applied to a hosted copy: [hosted path, from, to, count]. Empty:
// the hosted files are byte-identical to the source.
const FIXES = [];

// Names a public reader cannot use: maintainer plans and tasks, repository
// paths, source symbols, local paths. Matched against every hosted file.
const INTERNAL = /\b(?:SVC|SEM)-\d|\bD-\d\d\b|\bU\d\b|PLAN decision|\bPhases? \d|studio|\blanes?\b|axiom-internal|\/Users\/|\/Volumes\/|\btests\/|\bsrc\/|\bcrates\/|\.rs\b|semantic_api|README\.md|SUPPORT\.md|acore repository|protocol\/semantic|fixture|worktree|semantic\/integration/i;
// A commit-hash-shaped token (letters and digits, 7 to 40 hex characters) that
// is not a shortened value followed by an ellipsis.
const HASH = /(?<![0-9a-zA-Z:_-])(?=[0-9a-f]*[a-f])(?=[0-9a-f]*[0-9])[0-9a-f]{7,40}(?![0-9a-zA-Z.…])/;

function walk(directory, base = directory) {
  return readdirSync(directory).sort().flatMap((name) => {
    const path = join(directory, name);
    return statSync(path).isDirectory() ? walk(path, base) : [relative(base, path).split(sep).join('/')];
  });
}

const failures = [];
const expected = new Map(); // hosted path (relative to public/semantic-service) -> Buffer

for (const set of SETS) {
  if (!existsSync(set.from)) {
    console.error(`missing source directory ${set.from}`);
    process.exit(2);
  }
  for (const path of walk(set.from).filter(set.include)) {
    expected.set(`${set.name}/${path}`, readFileSync(join(set.from, path)));
  }
}

for (const [path, from, to, count] of FIXES) {
  const bytes = expected.get(path);
  if (!bytes) { failures.push(`fix for missing file ${path}`); continue; }
  const text = bytes.toString('utf8');
  const found = text.split(from).length - 1;
  if (found !== count) { failures.push(`${path}: fix expects ${count} match(es) of ${JSON.stringify(from)}, found ${found}; review the fix`); continue; }
  expected.set(path, Buffer.from(text.split(from).join(to), 'utf8'));
}

// Schema index and $ref resolution.
const schemaPaths = [...expected.keys()].filter((path) => path.startsWith('schemas/')).map((path) => path.slice('schemas/'.length));
const schemas = new Map();
for (const path of schemaPaths) {
  try { schemas.set(path, JSON.parse(expected.get(`schemas/${path}`).toString('utf8'))); }
  catch (error) { failures.push(`schemas/${path}: invalid JSON: ${error.message}`); }
}
const idBase = new Set();
let refCount = 0;
function checkRefs(node, path) {
  if (Array.isArray(node)) { node.forEach((item) => checkRefs(item, path)); return; }
  if (!node || typeof node !== 'object') return;
  for (const [key, value] of Object.entries(node)) {
    if (key === '$ref' && typeof value === 'string') {
      refCount += 1;
      const [filePart, fragment = ''] = value.split('#');
      if (/^[a-z]+:/i.test(filePart)) { failures.push(`schemas/${path}: absolute $ref ${value}`); continue; }
      const target = filePart ? posix.normalize(posix.join(posix.dirname(path), filePart)) : path;
      let resolved = schemas.get(target);
      if (resolved === undefined) { failures.push(`schemas/${path}: $ref ${value} names a file that is not hosted`); continue; }
      for (const raw of fragment ? fragment.slice(1).split('/') : []) {
        const segment = raw.replace(/~1/g, '/').replace(/~0/g, '~');
        resolved = resolved && typeof resolved === 'object' ? resolved[segment] : undefined;
      }
      if (resolved === undefined) failures.push(`schemas/${path}: $ref ${value} does not resolve`);
    } else {
      checkRefs(value, path);
    }
  }
}
for (const [path, schema] of schemas) {
  checkRefs(schema, path);
  const id = schema.$id;
  if (typeof id !== 'string' || !id.endsWith(`/${path}`)) failures.push(`schemas/${path}: $id ${id} does not end with its path`);
  else idBase.add(id.slice(0, -path.length));
}
if (idBase.size !== 1) failures.push(`schemas: expected one $id base, found ${[...idBase].join(', ')}`);

const index = {
  description: 'JSON Schema (draft 2020-12) files of the axiom-agent-semantic/v1 protocol, as served by acore-semantic. Each $id is an identifier, not a download location; these files are the published copies. References between files are relative and resolve against the $id, so load every file into your validator first, or map the $id base to url_base.',
  protocol: 'axiom-agent-semantic/v1',
  id_base: [...idBase][0],
  url_base: `${siteOrigin}/semantic-service/schemas/`,
  documentation: `${siteOrigin}/semantic-service/protocol`,
  files: schemaPaths.filter((path) => schemas.has(path)).map((path) => ({
    path,
    url: `${siteOrigin}/semantic-service/schemas/${path}`,
    id: schemas.get(path).$id,
    title: schemas.get(path).title,
  })),
};
expected.set('schemas/index.json', Buffer.from(`${JSON.stringify(index, null, 2)}\n`, 'utf8'));

for (const [path, bytes] of expected) {
  const lines = bytes.toString('utf8').split('\n');
  lines.forEach((line, number) => {
    const hit = line.match(INTERNAL) ?? line.match(HASH);
    if (hit) failures.push(`${path}:${number + 1}: internal name ${JSON.stringify(hit[0])}`);
  });
}

if (failures.length) {
  console.error(`semantic service files: ${failures.length} problem(s)`);
  for (const failure of failures) console.error(`- ${failure}`);
  process.exit(1);
}

const present = existsSync(publicRoot) ? walk(publicRoot) : [];
const differences = [
  ...[...expected].filter(([path, bytes]) => !existsSync(join(publicRoot, path)) || !readFileSync(join(publicRoot, path)).equals(bytes)).map(([path]) => `differs or missing: ${path}`),
  ...present.filter((path) => !expected.has(path)).map((path) => `not in the source: ${path}`),
];

// The agent skill page reproduces the hosted SKILL.md in a generated region.
const skillPage = join(docsRoot, 'content', 'docs', 'semantic-service', 'agent-skill.mdx');
const pageText = readFileSync(skillPage, 'utf8');
const region = /(\{\/\* BEGIN GENERATED skill \*\/\}\n)[\s\S]*?(\{\/\* END GENERATED skill \*\/\})/;
if (!region.test(pageText)) { console.error('agent-skill.mdx has no generated skill region'); process.exit(1); }
const skillText = expected.get('skill/SKILL.md').toString('utf8').replace(/\n$/, '');
if (skillText.includes('```')) { console.error('SKILL.md contains a code fence; the page block needs another fence'); process.exit(1); }
const pageExpected = pageText.replace(region, (_, begin, end) => `${begin}\n\`\`\`text title="SKILL.md"\n${skillText}\n\`\`\`\n\n${end}`);
if (pageExpected !== pageText) differences.push('differs: content/docs/semantic-service/agent-skill.mdx (skill region)');

const summary = `${schemaPaths.length} schema files (${refCount} $refs resolved) plus index.json, ${expected.size - schemaPaths.length - 1} skill files, ${FIXES.length} line fixes`;
if (checkOnly) {
  if (differences.length) {
    console.error(`semantic service files are out of date (${differences.length}):`);
    for (const difference of differences) console.error(`- ${difference}`);
    process.exit(1);
  }
  console.log(`semantic service files up to date: ${summary}`);
} else {
  rmSync(publicRoot, { recursive: true, force: true });
  for (const [path, bytes] of expected) {
    mkdirSync(dirname(join(publicRoot, path)), { recursive: true });
    writeFileSync(join(publicRoot, path), bytes);
  }
  writeFileSync(skillPage, pageExpected);
  console.log(`wrote public/semantic-service: ${summary}`);
}
