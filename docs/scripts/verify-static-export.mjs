import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { createCloudflareHeaders } from '../lib/deployment-policy.mjs';
import { legacyRedirects, legacyReaderRedirects } from '../lib/legacy-redirects.mjs';

import { toReaderMarkdown } from '../lib/content-model.mjs';

const docsRoot = resolve(import.meta.dirname, '..');
const outputRoot = join(docsRoot, 'out');

function requireFile(path) {
  if (!existsSync(path) || !statSync(path).isFile()) throw new Error(`Static export is missing ${path}`);
  return readFileSync(path, 'utf8');
}

function hasExportedPath(path) {
  const relative = path === '/' ? 'index.html' : path.slice(1);
  return [
    join(outputRoot, relative),
    join(outputRoot, `${relative}.html`),
    join(outputRoot, relative, 'index.html'),
  ].some((candidate) => existsSync(candidate));
}

requireFile(join(outputRoot, 'index.html'));
for (const path of [
  '/api/search',
  '/api/health',
  '/version.json',
  '/llms.txt',
  '/llms-full.txt',
  '/sitemap.xml',
  '/robots.txt',
  '/.well-known/security.txt',
  '/docs/reference/versioning-and-deprecation.mdx',
]) {
  if (!hasExportedPath(path)) throw new Error(`Static export is missing route output for ${path}`);
}

const headers = requireFile(join(outputRoot, '_headers'));
for (const required of [
  'Content-Security-Policy:',
  "default-src 'self'",
  'X-Frame-Options: DENY',
  '/api/health',
  'Content-Type: application/json; charset=utf-8',
  'Cache-Control: no-store',
  '/api/search',
]) {
  if (!headers.includes(required)) throw new Error(`Cloudflare _headers is missing ${required}`);
}
if (headers !== createCloudflareHeaders()) throw new Error('Cloudflare _headers differs from the deployment policy');

const redirects = requireFile(join(outputRoot, '_redirects'));
for (const redirect of legacyRedirects) {
  for (const source of [redirect.source, `${redirect.source}/`]) {
    const expected = `${source} ${redirect.destination}/ ${redirect.permanent ? 301 : 302}`;
    if (!redirects.split('\n').includes(expected)) throw new Error(`Missing redirect: ${expected}`);
  }
}
for (const redirect of legacyReaderRedirects) {
  const expected = `${redirect.source} ${redirect.destination} 301`;
  if (!redirects.split('\n').includes(expected)) throw new Error(`Missing reader redirect: ${expected}`);
}

const contentRoot = join(docsRoot, 'content', 'docs');
const files = readdirSync(contentRoot, { recursive: true }).filter((file) => file.endsWith('.mdx'));
const legacyAnchors = JSON.parse(readFileSync(join(docsRoot, 'lib', 'legacy-heading-anchors.json'), 'utf8'));
for (const [route, anchors] of Object.entries(legacyAnchors)) {
  const html = requireFile(join(outputRoot, route === '/' ? 'index.html' : route.slice(1) + '/index.html'));
  for (const anchor of anchors) {
    if (!html.includes(`id="${anchor}"`)) throw new Error(`Static compatibility anchor missing: ${route}#${anchor}`);
  }
}
const agents = JSON.parse(requireFile(join(outputRoot, 'agents.json')));
if (agents.pageCount !== files.length || agents.pages.length !== files.length) throw new Error('Static agent index is incomplete');
for (const file of files) {
  const slug = file.replace(/\.mdx$/, '').replace(/\/index$/, '');
  const canonical = `https://docs.axiomcore.dev/${slug === 'index' ? '' : slug + '/'}`;
  const expected = toReaderMarkdown(readFileSync(join(contentRoot, file), 'utf8'), { canonical });
  const reader = requireFile(join(outputRoot, 'docs', `${slug}.mdx`));
  if (reader !== expected) throw new Error(`Static reader lost source semantics: ${file}`);
  if (!agents.pages.some((page) => page.url === canonical)) throw new Error(`Agent index omits ${canonical}`);
}
console.log(`Cloudflare static export verified: routes, headers, ${legacyRedirects.length} human redirects, ${legacyReaderRedirects.length} reader redirects, and ${files.length} complete Markdown readers.`);
