// Regenerates the tables of the Semantic service pages from the protocol
// definition and the service's capability manifest, so that enum lists,
// fields, statuses and limits are derived rather than copied by hand.
//
// Usage (from docs/):
//   node scripts/generate-semantic-service.mjs --protocol <dir> --binary <acore-semantic> [--check]
//
// <dir> holds the protocol definition: README.md, schemas/ and
// support-matrix.json. The binary is asked for its v1 manifest (the
// `capabilities` operation) and every status in support-matrix.json must agree
// with it. Generated regions sit between BEGIN/END GENERATED comments in the
// pages below. --check fails instead of writing when a region is out of date.
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import process from 'node:process';

const args = process.argv.slice(2);
const option = (name) => {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
};
const protocolDir = option('--protocol');
const binary = option('--binary');
const checkOnly = args.includes('--check');
if (!protocolDir || !binary) {
  console.error('usage: generate-semantic-service.mjs --protocol <dir> --binary <acore-semantic> [--check]');
  process.exit(2);
}

const docsRoot = resolve(import.meta.dirname, '..');
const pagesDir = join(docsRoot, 'content', 'docs', 'semantic-service');
const schemasDir = join(protocolDir, 'schemas');
const readJson = (path) => JSON.parse(readFileSync(path, 'utf8'));

// ---------------------------------------------------------------- manifest

function v1Manifest() {
  const root = mkdtempSync(join(tmpdir(), 'semantic-docs-'));
  try {
    const request = JSON.stringify({ protocol: 'axiom-agent-semantic/v1', operation: 'capabilities', params: {} });
    const output = execFileSync(binary, ['--root', root], { input: request, encoding: 'utf8' });
    const response = JSON.parse(output);
    if (response.error) throw new Error(`capabilities failed: ${response.error.code}`);
    return response.result.manifest;
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
}

// ---------------------------------------------------------------- text hygiene

// Schema and matrix prose is written for maintainers. Keep only sentences that
// make sense to a public reader, and drop maintainer cross-references.
const internal = /pinned project|unit-tested|project-management|shopping-app|dashboard|studio\/v1|studio|\bSVC-|\bPhases?\b|approves the name|PLAN decision|D-05|follow-up|protocol gap|README|semantic_|tests\/|trials?\b|\bacore's\b|U1\b/i;

function publicText(text) {
  if (!text) return '';
  let value = String(text).replace(/\bSVC-\d+(?:\.\d+)?(?: follow-up)?:\s*/g, '');
  value = value.charAt(0).toUpperCase() + value.slice(1);
  // Remove parentheticals that only cite internal records.
  for (let pass = 0; pass < 3; pass += 1) {
    value = value.replace(/\s*\(([^()]*)\)/g, (match, inner) => (internal.test(inner) ? '' : match));
  }
  const sentences = value.split(/(?<=[.;])\s+(?=[A-Z`])/)
    .map((sentence) => sentence.split(/;\s+(?![^(]*\))/).filter((clause) => !internal.test(clause)).join('; '))
    .filter((sentence) => sentence && !internal.test(sentence))
    .map((sentence) => sentence.replace(/;$/, '.')).map((sentence) => (/[.:]$/.test(sentence) ? sentence : `${sentence}.`));
  return sentences.join(' ').replace(/\s+/g, ' ').replace(/;\./g, '.').trim();
}

// Escape MDX-significant characters outside inline code.
const cell = (text) => String(text ?? '')
  .split('`')
  .map((part, index) => (index % 2 ? part : part.replace(/[<>{}]/g, (char) => `&#${char.charCodeAt(0)};`)))
  .join('`')
  .replace(/\|/g, '\\|')
  .replace(/\n+/g, ' ')
  .trim();
const code = (value) => `\`${value}\``;

// ---------------------------------------------------------------- schemas

const schemaCache = new Map();
function loadSchema(path) {
  if (!schemaCache.has(path)) schemaCache.set(path, readJson(path));
  return schemaCache.get(path);
}

function resolveRef(ref, fromFile) {
  const [file, pointer = ''] = ref.split('#');
  const path = file ? resolve(dirname(fromFile), file) : fromFile;
  let node = loadSchema(path);
  for (const part of pointer.split('/').filter(Boolean)) node = node[part];
  return { schema: node, file: path, name: pointer.split('/').pop() };
}

function describeType(schema, file, depth = 0) {
  if (!schema) return 'any';
  if (schema.$ref) {
    const target = resolveRef(schema.$ref, file);
    const inner = target.schema;
    if (!inner.type && !inner.enum && inner.const === undefined) {
      const variants = inner.oneOf ?? inner.anyOf;
      return variants ? variants.map((variant) => describeType(variant, target.file, depth + 1)).join(' or ') : 'any JSON value';
    }
    if (inner.enum || inner.const !== undefined || ['string', 'integer', 'number', 'boolean'].includes(inner.type) || depth > 1) {
      return describeType(inner, target.file, depth + 1);
    }
    return `${inner.type === 'array' ? describeType(inner, target.file, depth + 1) : 'object'} (${code(target.name)})`;
  }
  if (schema.const !== undefined) return code(JSON.stringify(schema.const));
  if (schema.enum) return schema.enum.map((value) => code(value)).join(', ');
  const variants = schema.oneOf ?? schema.anyOf;
  if (variants) return variants.map((variant) => describeType(variant, file, depth + 1)).join(' or ');
  const type = Array.isArray(schema.type) ? schema.type.join(' or ') : schema.type;
  const bounds = [];
  if (type === 'array') {
    if (schema.minItems !== undefined || schema.maxItems !== undefined) bounds.push(`${schema.minItems ?? 0} to ${schema.maxItems ?? 'any'} items`);
    return `array of ${describeType(schema.items, file, depth + 1)}${bounds.length ? `, ${bounds.join(', ')}` : ''}`;
  }
  if (schema.minimum !== undefined || schema.maximum !== undefined) bounds.push(`${schema.minimum ?? ''}..${schema.maximum ?? ''}`);
  if (schema.maxLength !== undefined) bounds.push(`at most ${schema.maxLength} characters`);
  return `${type ?? 'object'}${bounds.length ? ` (${bounds.join(', ')})` : ''}`;
}

function fieldDescription(schema, file) {
  if (schema.const !== undefined || (schema.$ref && resolveRef(schema.$ref, file).schema.const !== undefined)) return '';
  let text = schema.description;
  if (!text && schema.$ref) text = resolveRef(schema.$ref, file).schema.description;
  return publicText(text);
}

function fieldTable(schema, file, emptyText) {
  const properties = schema.properties ?? {};
  const required = new Set(schema.required ?? []);
  const names = Object.keys(properties);
  if (!names.length) return emptyText;
  const rows = names.map((name) => {
    const property = properties[name];
    return `| ${code(name)} | ${required.has(name) ? 'yes' : 'no'} | ${cell(describeType(property, file))} | ${cell(fieldDescription(property, file))} |`;
  });
  return ['| Field | Required | Type | Meaning |', '| --- | --- | --- | --- |', ...rows].join('\n');
}

// ---------------------------------------------------------------- envelope

const envelopePath = join(schemasDir, 'envelope.request.json');
const envelope = loadSchema(envelopePath);
const operationsEnum = resolveRef(envelope.properties.operation.$ref, envelopePath).schema.enum;
const envelopeFields = ['workspace', 'snapshot', 'base_snapshot', 'request_id'];

function envelopeRules(operation) {
  const rules = Object.fromEntries(envelopeFields.map((field) => [field, 'optional']));
  for (const rule of envelope.allOf ?? []) {
    if (!rule.if?.properties?.operation?.enum?.includes(operation)) continue;
    for (const field of rule.then.required ?? []) rules[field] = 'required';
    for (const clause of rule.then.not?.anyOf ?? []) for (const field of clause.required ?? []) rules[field] = 'not allowed';
    if (rule.then.oneOf) rules.snapshot = 'required unless `params.candidate` is given';
  }
  return rules;
}

function envelopeTable() {
  const rows = operationsEnum.map((operation) => {
    const rules = envelopeRules(operation);
    return `| ${code(operation)} | ${envelopeFields.map((field) => cell(rules[field])).join(' | ')} |`;
  });
  return [`| Operation | ${envelopeFields.map(code).join(' | ')} |`, `| --- | ${envelopeFields.map(() => '---').join(' | ')} |`, ...rows].join('\n');
}

function envelopeFieldTable() {
  return fieldTable(envelope, envelopePath, '');
}

// ---------------------------------------------------------------- operations

const operationSchema = (operation, side) => {
  const path = join(schemasDir, 'operations', `${operation}.${side}.json`);
  return { schema: loadSchema(path), path };
};

// Object-valued result fields (such as the apply receipt) get their own table.
function nestedTables(schema, file) {
  const tables = [];
  for (const [name, property] of Object.entries(schema.properties ?? {})) {
    let target = { schema: property, file };
    if (property.$ref) target = resolveRef(property.$ref, file);
    if (target.schema.type === 'object' && target.schema.properties && Object.keys(target.schema.properties).length > 1) {
      tables.push('', `\`${name}\` fields:`, '', fieldTable(target.schema, target.file, ''));
    }
  }
  return tables;
}

function operationSection(operation) {
  const request = operationSchema(operation, 'request');
  const result = operationSchema(operation, 'result');
  const errors = result.schema['x-errors'] ?? [];
  return [
    `**Params** (\`schemas/operations/${operation}.request.json\`)`,
    '',
    fieldTable(request.schema, request.path, 'No params: send `"params": {}`.'),
    '',
    `**Result** (\`schemas/operations/${operation}.result.json\`)`,
    '',
    fieldTable(result.schema, result.path, 'An empty object.'),
    ...nestedTables(result.schema, result.path),
    '',
    `**Error codes:** ${errors.map(code).join(', ')}.`,
  ].join('\n');
}

function errorCodeList() {
  const path = join(schemasDir, 'error.json');
  const schema = loadSchema(path);
  return [
    '| Field | Required | Type | Meaning |',
    '| --- | --- | --- | --- |',
    ...fieldTable(schema, path, '').split('\n').slice(2),
  ].join('\n');
}

function errorsByOperation() {
  const codes = loadSchema(join(schemasDir, 'error.json')).$defs.code.enum;
  const header = `| Code | ${'Operations that can return it'} |`;
  const rows = codes.map((errorCode) => {
    const operations = operationsEnum.filter((operation) => (operationSchema(operation, 'result').schema['x-errors'] ?? []).includes(errorCode));
    return `| ${code(errorCode)} | ${operations.length === operationsEnum.length ? 'every operation' : operations.map(code).join(', ')} |`;
  });
  return [header, '| --- | --- |', ...rows].join('\n');
}

function editOperations() {
  const path = join(schemasDir, 'edits.json');
  const variants = loadSchema(path).$defs.editOperation.oneOf;
  return variants.map((variant) => {
    const target = resolveRef(variant.$ref, path);
    const { kind, op } = target.schema.properties;
    const label = op?.const ? `${code(kind.const)}, ${code(`op: ${op.const}`)}` : code(kind.const);
    const description = publicText(target.schema.description);
    return [`**${label}**${description ? `: ${cell(description)}` : ''}`, '', fieldTable(target.schema, target.file, '')].join('\n');
  }).join('\n\n');
}

function semanticStates() {
  const states = loadSchema(join(schemasDir, 'common.json')).$defs.semanticState;
  const meaning = Object.fromEntries([...states.description.matchAll(/([a-z_]+) \(([^)]*)\)/g)].map((match) => [match[1], match[2]]));
  return states.enum.map((state) => `- ${code(state)}${meaning[state] ? `: ${meaning[state]}` : ''}`).join('\n');
}

function limitsTable(manifest) {
  const { defaults, maximums } = manifest.limits;
  const rows = Object.keys(maximums).map((name) => `| ${code(name)} | ${defaults[name] ?? ''} | ${maximums[name]} |`);
  const input = manifest.limits.input ?? {};
  const inputRows = Object.entries({ request_max_bytes: manifest.limits.request_max_bytes, timeout_ms: manifest.limits.timeout_ms, group_compile_budget_ms: manifest.limits.group_compile_budget_ms, ...input })
    .filter(([, value]) => value !== undefined)
    .map(([name, value]) => `| ${code(name)} | ${value} |`);
  return [
    '| Request limit | Default | Maximum |', '| --- | --- | --- |', ...rows,
    '',
    '| Service bound | Value |', '| --- | --- |', ...inputRows,
  ].join('\n');
}

// ---------------------------------------------------------------- support matrix

function statusTable(entries, manifestEntries, key) {
  const served = new Map(manifestEntries.map((entry) => [entry[key], entry.status]));
  const rows = entries.map((entry) => {
    const limits = entry.limits.map(publicText).filter(Boolean).join(' ');
    const missing = entry.missing.map(publicText).filter(Boolean).join(' ');
    return `| ${code(entry.name)} | ${served.get(entry.name)} | ${cell(limits)} | ${cell(missing)} |`;
  });
  return ['| Name | Status | Limits | Missing (partial only) |', '| --- | --- | --- | --- |', ...rows].join('\n');
}

function guaranteeList(names) {
  return names.map((name) => `- ${code(name)}`).join('\n');
}

function compareMatrix(matrix, manifest) {
  const problems = [];
  const compare = (label, entries, manifestEntries, key) => {
    const served = new Map(manifestEntries.map((entry) => [entry[key], entry.status]));
    for (const entry of entries) {
      if (served.get(entry.name) !== entry.status) problems.push(`${label} ${entry.name}: matrix ${entry.status}, manifest ${served.get(entry.name)}`);
    }
    for (const name of served.keys()) if (!entries.some((entry) => entry.name === name)) problems.push(`${label} ${name}: in the manifest, not in the matrix`);
  };
  compare('operation', matrix.operations, manifest.operations, 'name');
  compare('edit kind', matrix.edit_kinds, manifest.edit_kinds, 'kind');
  compare('check', matrix.checks, manifest.checks, 'check');
  const sameList = (label, a, b) => { if (JSON.stringify(a) !== JSON.stringify(b)) problems.push(`${label} differ`); };
  sameList('publication guarantees', matrix.publication_guarantees.map((entry) => entry.name), manifest.publication[0].guarantees);
  sameList('external-write guarantees', matrix.external_write_guarantees.map((entry) => entry.name), manifest.external_writes.guarantees);
  sameList('error codes', loadSchema(join(schemasDir, 'error.json')).$defs.code.enum, manifest.error_codes);
  sameList('operations', operationsEnum, manifest.operations.map((entry) => entry.name));
  return problems;
}

// ---------------------------------------------------------------- regions

function replaceRegions(file, regions) {
  const path = join(pagesDir, file);
  const before = readFileSync(path, 'utf8');
  let after = before;
  for (const [id, body] of Object.entries(regions)) {
    const escaped = id.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const pattern = new RegExp(`(\\{/\\* BEGIN GENERATED ${escaped} \\*/\\})[\\s\\S]*?(\\{/\\* END GENERATED ${escaped} \\*/\\})`);
    if (!pattern.test(after)) throw new Error(`${file}: missing generated region ${id}`);
    after = after.replace(pattern, (match, start, end) => `${start}\n\n${body}\n\n${end}`);
  }
  if (after === before) return false;
  if (!checkOnly) writeFileSync(path, after);
  return true;
}

const manifest = v1Manifest();
const matrix = readJson(join(protocolDir, 'support-matrix.json'));
const problems = compareMatrix(matrix, manifest);
if (problems.length) {
  console.error('support-matrix.json and the binary manifest disagree:');
  for (const problem of problems) console.error(`- ${problem}`);
  process.exit(1);
}

const operationFiles = readdirSync(join(schemasDir, 'operations')).filter((name) => name.endsWith('.request.json')).map((name) => name.replace('.request.json', ''));
if (operationFiles.sort().join() !== [...operationsEnum].sort().join()) {
  console.error('operation schemas and the operation enum disagree');
  process.exit(1);
}

const protocolRegions = {
  'envelope-fields': envelopeFieldTable(),
  'envelope-rules': envelopeTable(),
  'error-object': errorCodeList(),
  'errors-by-operation': errorsByOperation(),
  limits: limitsTable(manifest),
  'edit-operations': editOperations(),
  'semantic-states': semanticStates(),
};
for (const operation of operationsEnum) protocolRegions[`operation:${operation}`] = operationSection(operation);

const count = (entries) => {
  const counts = {};
  for (const entry of entries) counts[entry.status] = (counts[entry.status] ?? 0) + 1;
  return Object.entries(counts).map(([status, value]) => `${value} ${status}`).join(', ');
};

const matrixRegions = {
  summary: [
    `- Protocol: ${code(manifest.protocol.name)}. Service version: ${code(manifest.service.version)}.`,
    `- Profiles: ${manifest.profiles.map((profile) => `${code(profile.name)} (${profile.status}; targets ${profile.targets.map(code).join(', ')})`).join(', ')}.`,
    `- Operations: ${count(matrix.operations)}. Edit families: ${count(matrix.edit_kinds)}. Checks: ${count(matrix.checks)}.`,
    `- Publication: ${code(matrix.publication.profile)}, ${matrix.publication.status}. Not provided: ${matrix.publication.missing.map((item) => item.charAt(0).toLowerCase() + item.slice(1)).join('; ')}.`,
  ].join('\n'),
  operations: statusTable(matrix.operations, manifest.operations, 'name'),
  'edit-kinds': statusTable(matrix.edit_kinds, manifest.edit_kinds, 'kind'),
  checks: statusTable(matrix.checks, manifest.checks, 'check'),
  publication: guaranteeList(manifest.publication[0].guarantees),
  'external-writes': guaranteeList(manifest.external_writes.guarantees),
  unsupported: matrix.unsupported.map((item) => `- ${publicText(item) || item.replace(/\s*\(D-05[^)]*\)/, '')}`).join('\n'),
};

// The hand-written error guidance must cover exactly the closed code list.
const protocolPage = readFileSync(join(pagesDir, 'protocol.mdx'), 'utf8');
const guidance = protocolPage.split('### Error codes and what to do')[1]?.split('\n\n')[1] ?? '';
const covered = [...guidance.matchAll(/^\| `([a-z_]+)` \|/gm)].map((match) => match[1]);
const codes = loadSchema(join(schemasDir, 'error.json')).$defs.code.enum;
if ([...covered].sort().join() !== [...codes].sort().join()) {
  console.error(`Error guidance table covers ${covered.join(', ')}; error.json has ${codes.join(', ')}`);
  process.exit(1);
}

const changed = [
  replaceRegions('protocol.mdx', protocolRegions) && 'protocol.mdx',
  replaceRegions('support-matrix.mdx', matrixRegions) && 'support-matrix.mdx',
].filter(Boolean);

if (checkOnly && changed.length) {
  console.error(`Generated regions are out of date in: ${changed.join(', ')}`);
  process.exit(1);
}
console.log(`${checkOnly ? 'Checked' : 'Generated'} semantic service tables (manifest ${manifest.service.version}, ${operationsEnum.length} operations)${changed.length && !checkOnly ? `; updated ${changed.join(', ')}` : ''}.`);
