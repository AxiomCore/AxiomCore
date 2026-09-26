import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, extname, join, relative, resolve, sep } from 'node:path';
import process from 'node:process';
import { legacyRedirects } from '../lib/legacy-redirects.mjs';

const docsRoot = resolve(import.meta.dirname, '..');
const repositoryRoot = resolve(docsRoot, '..');
const contentRoot = join(docsRoot, 'content', 'docs');
const failures = [];

if (!existsSync(join(docsRoot, 'pnpm-lock.yaml'))) {
  failures.push('docs/pnpm-lock.yaml: missing authoritative dependency lock');
}

function walk(directory, predicate = () => true) {
  const result = [];
  for (const name of readdirSync(directory)) {
    const path = join(directory, name);
    const stats = statSync(path);
    if (stats.isDirectory()) result.push(...walk(path, predicate));
    else if (predicate(path)) result.push(path);
  }
  return result;
}

function display(path) {
  return relative(repositoryRoot, path).split(sep).join('/');
}

function fail(path, message) {
  failures.push(`${display(path)}: ${message}`);
}

function routeFor(path) {
  let slug = relative(contentRoot, path).split(sep).join('/').replace(/\.mdx$/, '');
  if (slug === 'index') return '/';
  slug = slug.replace(/\/index$/, '');
  return `/${slug}`;
}

const pages = walk(contentRoot, (path) => extname(path) === '.mdx');
const routes = new Set(pages.map(routeFor));
const titles = new Map();
const descriptions = new Map();

const redirectSources = new Set();
for (const redirect of legacyRedirects) {
  const { source, destination, permanent } = redirect;
  if (!/^\/[A-Za-z0-9_./-]+$/.test(source)) {
    failures.push(`Legacy redirect has an invalid source: ${source}`);
  }
  if (!/^\/[A-Za-z0-9_./-]+$/.test(destination)) {
    failures.push(`Legacy redirect has an invalid destination: ${destination}`);
  }
  if (redirectSources.has(source)) failures.push(`Duplicate legacy redirect source: ${source}`);
  redirectSources.add(source);
  if (routes.has(source)) failures.push(`Legacy redirect source is still a current page: ${source}`);
  if (!routes.has(destination)) failures.push(`Legacy redirect destination is not a current page: ${destination}`);
  if (source === destination) failures.push(`Legacy redirect loops to itself: ${source}`);
  if (permanent !== true) failures.push(`Legacy redirect must explicitly be permanent: ${source}`);
}
for (const redirect of legacyRedirects) {
  if (redirectSources.has(redirect.destination)) {
    failures.push(`Legacy redirect chain is not allowed: ${redirect.source} -> ${redirect.destination}`);
  }
}

for (const page of pages) {
  const text = readFileSync(page, 'utf8');
  const frontmatter = text.match(/^---\n([\s\S]*?)\n---(?:\n|$)/)?.[1];
  if (!frontmatter) {
    fail(page, 'missing YAML frontmatter');
    continue;
  }

  const title = frontmatter.match(/^title:\s*(.+)$/m)?.[1]?.trim();
  const description = frontmatter.match(/^description:\s*(.+)$/m)?.[1]?.trim();
  if (!title) fail(page, 'missing title frontmatter');
  if (!description) fail(page, 'missing description frontmatter');

  for (const [value, index, field] of [
    [title, titles, 'title'],
    [description, descriptions, 'description'],
  ]) {
    if (!value) continue;
    const previous = index.get(value);
    if (previous) fail(page, `duplicate ${field}; first used by ${display(previous)}`);
    else index.set(value, page);
  }

  const links = [
    ...text.matchAll(/\]\((\/[A-Za-z0-9_./#-]+)\)/g),
    ...text.matchAll(/href=["'](\/[A-Za-z0-9_./#-]+)["']/g),
  ].map((match) => match[1]);

  for (const link of links) {
    const pathname = link.split('#', 1)[0].replace(/\/$/, '') || '/';
    if (!routes.has(pathname)) fail(page, `internal link does not resolve: ${link}`);
  }
}

for (const metaPath of walk(contentRoot, (path) => path.endsWith(`${sep}meta.json`))) {
  let meta;
  try {
    meta = JSON.parse(readFileSync(metaPath, 'utf8'));
  } catch (error) {
    fail(metaPath, `invalid JSON: ${error.message}`);
    continue;
  }

  if (!Array.isArray(meta.pages)) {
    fail(metaPath, 'pages must be an array');
    continue;
  }

  const listed = meta.pages.filter((entry) => !entry.startsWith('---'));
  const duplicates = listed.filter((entry, index) => listed.indexOf(entry) !== index);
  for (const entry of new Set(duplicates)) fail(metaPath, `duplicate navigation entry: ${entry}`);

  const directory = dirname(metaPath);
  const expected = readdirSync(directory)
    .filter((name) => name !== 'meta.json' && !name.startsWith('.'))
    .filter((name) => {
      const path = join(directory, name);
      return (statSync(path).isFile() && name.endsWith('.mdx'))
        || (statSync(path).isDirectory() && existsSync(join(path, 'meta.json')));
    })
    .map((name) => name.replace(/\.mdx$/, ''));

  for (const entry of expected) {
    if (!listed.includes(entry)) fail(metaPath, `navigation omits ${entry}`);
  }
  for (const entry of listed) {
    if (!expected.includes(entry)) fail(metaPath, `navigation references missing entry ${entry}`);
  }
}

let publicFiles = [];
try {
  const output = execFileSync(
    'git',
    ['ls-files', '--cached', '--others', '--exclude-standard', '-z', '--', 'README.md', 'CONTRIBUTING.md', 'docs/content'],
    { cwd: repositoryRoot, encoding: 'utf8' },
  );
  publicFiles = output.split('\0').filter(Boolean).map((path) => join(repositoryRoot, path));
} catch (error) {
  failures.push(`Unable to enumerate public files with git: ${error.message}`);
}

const unsafePatterns = [
  [/(?:^|[^A-Za-z])\/Users\/[A-Za-z0-9._-]+/m, 'developer macOS home-directory path'],
  [/C:\\Users\\[A-Za-z0-9._-]+/i, 'developer Windows home-directory path'],
  [/-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----/, 'private key material'],
  [/\baxot_[A-Za-z0-9]+\b/, 'credential-shaped telemetry token'],
  [/\bsk_live_[A-Za-z0-9]+\b/, 'credential-shaped live secret'],
  [/\b(?:my_)?supersecret(?:_key)?\b/i, 'credential-shaped demo secret'],
];
const internalMilestone = /\b(?:phase|docs)\s+[0-9]+[A-Za-z]?\b/i;

for (const path of publicFiles) {
  let text;
  try {
    text = readFileSync(path, 'utf8');
  } catch {
    continue;
  }
  // Opaque compiled fixtures can contain toolchain build paths in symbol data.
  // Scan reviewable text and JSON artifacts; validate native binaries in their
  // owning release pipeline instead of interpreting them as public copy.
  if (text.includes('\0')) continue;
  for (const [pattern, label] of unsafePatterns) {
    if (pattern.test(text)) fail(path, `contains ${label}`);
  }
  if (internalMilestone.test(text)) fail(path, 'contains an internal numbered milestone name');
}

for (const manifest of publicFiles.filter((path) => path.endsWith('AxiomDeps.toml'))) {
  const directory = dirname(manifest);
  const text = readFileSync(manifest, 'utf8');
  for (const match of text.matchAll(/^source\s*=\s*"([^"]+)"/gm)) {
    const source = match[1];
    if (/^https?:\/\//.test(source)) continue;
    if (source.startsWith('/') || /^[A-Za-z]:\\/.test(source)) {
      fail(manifest, `contract source must be relative: ${source}`);
    } else if (!existsSync(resolve(directory, source))) {
      fail(manifest, `relative contract source does not exist: ${source}`);
    }
  }
}

const supportPath = join(contentRoot, 'introduction', 'support-matrix.mdx');
const maximumReviewAgeDays = 120;
const reviewLabel = readFileSync(supportPath, 'utf8').match(/last reviewed[^\n]*\n\*\*([A-Za-z]+ \d{1,2}, \d{4})\*\*/i)?.[1];
const reviewTime = reviewLabel ? Date.parse(`${reviewLabel} UTC`) : Number.NaN;
if (!Number.isFinite(reviewTime)) fail(supportPath, 'missing or invalid public support-matrix review date');
else {
  const today = new Date();
  const todayTime = Date.UTC(today.getUTCFullYear(), today.getUTCMonth(), today.getUTCDate());
  const reviewAgeDays = Math.floor((todayTime - reviewTime) / (24 * 60 * 60 * 1_000));
  // Permit the adjacent calendar day so contributors east of UTC do not fail
  // a UTC-based CI run during their local morning.
  if (reviewAgeDays < -1) fail(supportPath, `review date is more than one day in the future: ${reviewLabel}`);
  if (reviewAgeDays > maximumReviewAgeDays) {
    fail(supportPath, `support matrix review is ${reviewAgeDays} days old; maximum is ${maximumReviewAgeDays}`);
  }
}

if (failures.length > 0) {
  console.error(`Documentation policy failed with ${failures.length} issue(s):`);
  for (const failure of failures) console.error(`- ${failure}`);
  process.exit(1);
}

console.log(`Documentation policy passed: ${pages.length} pages, ${routes.size} routes, ${publicFiles.length} public files.`);
