import { existsSync, readFileSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { createCloudflareHeaders } from '../lib/deployment-policy.mjs';
import { legacyRedirects } from '../lib/legacy-redirects.mjs';

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
  const expected = `${redirect.source} ${redirect.destination} ${redirect.permanent ? 301 : 302}`;
  if (!redirects.split('\n').includes(expected)) {
    throw new Error(`Cloudflare _redirects is missing ${expected}`);
  }
}

console.log('Cloudflare static export verified: routes, headers, and redirects are present.');
