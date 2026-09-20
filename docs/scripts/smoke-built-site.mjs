import { spawn } from 'node:child_process';
import { readFileSync } from 'node:fs';
import process from 'node:process';
import { legacyRedirects } from '../lib/legacy-redirects.mjs';

const docsRoot = new URL('../', import.meta.url).pathname;
const host = '127.0.0.1';
const port = 4310;
const origin = `http://${host}:${port}`;
const nextBin = new URL('../node_modules/next/dist/bin/next', import.meta.url).pathname;
const server = spawn(process.execPath, [nextBin, 'start', '-H', host, '-p', String(port)], {
  cwd: docsRoot,
  env: { ...process.env, NEXT_TELEMETRY_DISABLED: '1' },
  stdio: ['ignore', 'pipe', 'pipe'],
});

let serverOutput = '';
server.stdout.on('data', (chunk) => { serverOutput += chunk; });
server.stderr.on('data', (chunk) => { serverOutput += chunk; });

const routes = [
  ['/', 'text/html', 'Software built around contracts'],
  ['/guides', 'text/html', 'Adoption guides'],
  ['/reference/versioning-and-deprecation', 'text/html', 'Versioning and deprecation'],
  ['/reference/support-and-feedback', 'text/html', 'Support and feedback'],
  ['/reference/documentation-contributions', 'text/html', 'Documentation contributions'],
  ['/reference/documentation-site-privacy', 'text/html', 'Documentation site privacy'],
  ['/llms.txt', 'text/plain', 'https://docs.axiomcore.dev/reference/versioning-and-deprecation'],
  ['/llms-full.txt', 'text/plain', '# AxiomCore documentation corpus'],
  ['/docs/reference/versioning-and-deprecation.mdx', 'text/markdown', 'Source: https://docs.axiomcore.dev/reference/versioning-and-deprecation'],
  ['/sitemap.xml', 'application/xml', 'https://docs.axiomcore.dev/reference/support-and-feedback'],
  ['/robots.txt', 'text/plain', 'Sitemap: https://docs.axiomcore.dev/sitemap.xml'],
  ['/og/docs/reference/versioning-and-deprecation/image.png', 'image/png', null],
];

const requiredSecurityHeaders = new Map([
  ['content-security-policy', ["default-src 'self'", "object-src 'none'", "frame-ancestors 'none'"]],
  ['cross-origin-opener-policy', ['same-origin']],
  ['permissions-policy', ['camera=()', 'geolocation=()', 'microphone=()']],
  ['referrer-policy', ['strict-origin-when-cross-origin']],
  ['strict-transport-security', ['max-age=31536000']],
  ['x-content-type-options', ['nosniff']],
  ['x-frame-options', ['DENY']],
]);

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function inBatches(items, batchSize, operation) {
  for (let index = 0; index < items.length; index += batchSize) {
    await Promise.all(items.slice(index, index + batchSize).map(operation));
  }
}

function assertSameMembers(actual, expected, label) {
  const missing = [...expected].filter((value) => !actual.has(value));
  const unexpected = [...actual].filter((value) => !expected.has(value));
  if (missing.length || unexpected.length) {
    throw new Error(`${label} differs from the page corpus; missing=${missing.join(',') || 'none'}; unexpected=${unexpected.join(',') || 'none'}`);
  }
}

async function waitForServer() {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    if (server.exitCode !== null) throw new Error(`Next.js exited early (${server.exitCode}).\n${serverOutput}`);
    try {
      const response = await fetch(origin, { signal: AbortSignal.timeout(1_000) });
      if (response.ok) return;
    } catch {
      // The server has not bound the port yet.
    }
    await delay(100);
  }
  throw new Error(`Timed out waiting for ${origin}.\n${serverOutput}`);
}

try {
  await waitForServer();
  for (const [path, expectedType, expectedText] of routes) {
    const response = await fetch(`${origin}${path}`, { signal: AbortSignal.timeout(10_000) });
    if (response.status !== 200) throw new Error(`${path} returned HTTP ${response.status}`);
    const type = response.headers.get('content-type') ?? '';
    if (!type.includes(expectedType)) throw new Error(`${path} returned ${type}, expected ${expectedType}`);
    if (expectedText) {
      const body = await response.text();
      if (!body.includes(expectedText)) throw new Error(`${path} omitted expected text: ${expectedText}`);
    } else {
      const bytes = (await response.arrayBuffer()).byteLength;
      if (bytes < 1_000) throw new Error(`${path} returned an unexpectedly small image (${bytes} bytes)`);
    }
    console.log(`HTTP 200 ${path}`);
  }

  const home = await fetch(origin);
  for (const [header, expectedParts] of requiredSecurityHeaders) {
    const value = home.headers.get(header) ?? '';
    for (const expected of expectedParts) {
      if (!value.includes(expected)) throw new Error(`Missing ${header} policy: ${expected}`);
    }
  }
  if (home.headers.has('x-powered-by')) throw new Error('The production response exposes X-Powered-By');
  const homeHtml = await home.text();
  for (const expected of [
    '<html lang="en"',
    '<link rel="canonical" href="https://docs.axiomcore.dev"',
    'property="og:title"',
    'name="description"',
  ]) {
    if (!homeHtml.includes(expected)) throw new Error(`Home HTML omitted SEO/accessibility marker: ${expected}`);
  }

  const reference = await fetch(`${origin}/reference/versioning-and-deprecation`);
  const referenceHtml = await reference.text();
  if ((referenceHtml.match(/<h1[ >]/g) ?? []).length !== 1) {
    throw new Error('Versioning page must render exactly one h1');
  }
  if (!referenceHtml.includes('<th')) throw new Error('Versioning table did not render table headers');
  if (referenceHtml.includes('name="robots" content="noindex')) throw new Error('Public reference page is noindex');

  const missing = await fetch(`${origin}/definitely-not-a-documentation-route`);
  if (missing.status !== 404) throw new Error(`Unknown route returned HTTP ${missing.status}, expected 404`);
  if (!(await missing.text()).includes('Documentation page not found')) throw new Error('Custom 404 content was not rendered');

  const health = await fetch(`${origin}/api/health`);
  if (health.status !== 200) throw new Error(`Health route returned HTTP ${health.status}`);
  if (!health.headers.get('cache-control')?.includes('no-store')) throw new Error('Health route must be no-store');
  const healthBody = await health.json();
  if (healthBody.schema_version !== 1 || healthBody.status !== 'ok' || healthBody.service !== 'axiomcore-docs') {
    throw new Error('Health response does not match schema version 1');
  }

  const version = await fetch(`${origin}/version.json`);
  const versionBody = await version.json();
  if (versionBody.schema_version !== 1 || versionBody.canonical_origin !== 'https://docs.axiomcore.dev') {
    throw new Error('Version response does not match schema version 1');
  }

  const security = await fetch(`${origin}/.well-known/security.txt`);
  const securityBody = await security.text();
  if (security.status !== 200 || !securityBody.includes('Canonical: https://docs.axiomcore.dev/.well-known/security.txt')) {
    throw new Error('security.txt is missing its canonical production URL');
  }
  const expires = securityBody.match(/^Expires:\s*(.+)$/m)?.[1];
  if (!expires || Date.parse(expires) < Date.now() + 30 * 24 * 60 * 60 * 1_000) {
    throw new Error('security.txt must remain valid for at least 30 days');
  }

  const releaseManifest = JSON.parse(readFileSync(new URL('../artifacts/docs-release-manifest.json', import.meta.url), 'utf8'));
  if (releaseManifest.schema_version !== 1 || releaseManifest.documented_page_count < 1) {
    throw new Error('Documentation release manifest is missing or invalid');
  }
  if (!releaseManifest.public_output_proofs?.every((proof) => /^[a-f0-9]{64}$/.test(proof.sha256))) {
    throw new Error('Documentation release manifest contains an invalid output proof');
  }

  const sitemapText = await (await fetch(`${origin}/sitemap.xml`)).text();
  const sitemapUrls = new Set([...sitemapText.matchAll(/<loc>(https:\/\/docs\.axiomcore\.dev[^<]+)<\/loc>/g)].map((match) => match[1]));
  if (sitemapUrls.size !== releaseManifest.documented_page_count) {
    throw new Error(`Sitemap contains ${sitemapUrls.size} pages; expected ${releaseManifest.documented_page_count}`);
  }

  const llmsText = await (await fetch(`${origin}/llms.txt`)).text();
  const llmsUrls = new Set([...llmsText.matchAll(/\]\((https:\/\/docs\.axiomcore\.dev[^)]+)\):/g)].map((match) => match[1]));
  assertSameMembers(llmsUrls, sitemapUrls, 'llms.txt URL inventory');

  const canonicalOrigin = 'https://docs.axiomcore.dev';
  const pagePaths = [...sitemapUrls].map((url) => new URL(url).pathname).sort();
  await inBatches(pagePaths, 8, async (path) => {
    const canonical = `${canonicalOrigin}${path === '/' ? '' : path}`;
    const response = await fetch(`${origin}${path}`, { signal: AbortSignal.timeout(10_000) });
    if (response.status !== 200) throw new Error(`Corpus page ${path} returned HTTP ${response.status}`);
    if (!response.headers.get('content-type')?.includes('text/html')) {
      throw new Error(`Corpus page ${path} did not return HTML`);
    }
    const html = await response.text();
    if (!html.includes(`<link rel="canonical" href="${canonical}"`)) {
      throw new Error(`Corpus page ${path} omitted canonical ${canonical}`);
    }
    if ((html.match(/<h1[ >]/g) ?? []).length !== 1) {
      throw new Error(`Corpus page ${path} must render exactly one h1`);
    }
    if (html.includes('name="robots" content="noindex')) {
      throw new Error(`Corpus page ${path} is unexpectedly noindex`);
    }

    const imagePath = path === '/' ? '/og/docs/image.png' : `/og/docs${path}/image.png`;
    const image = await fetch(`${origin}${imagePath}`, { signal: AbortSignal.timeout(10_000) });
    if (image.status !== 200 || !image.headers.get('content-type')?.includes('image/png')) {
      throw new Error(`Open Graph image ${imagePath} is unavailable`);
    }
    if ((await image.arrayBuffer()).byteLength < 1_000) {
      throw new Error(`Open Graph image ${imagePath} is unexpectedly small`);
    }

    if (path !== '/') {
      const readerPath = `/docs${path}.mdx`;
      const reader = await fetch(`${origin}${readerPath}`, { signal: AbortSignal.timeout(10_000) });
      if (reader.status !== 200 || !reader.headers.get('content-type')?.includes('text/markdown')) {
        throw new Error(`Machine-reader route ${readerPath} is unavailable`);
      }
      if (!(await reader.text()).includes(`Source: ${canonical}`)) {
        throw new Error(`Machine-reader route ${readerPath} has the wrong canonical source`);
      }
    }
  });

  for (const redirect of legacyRedirects) {
    const response = await fetch(`${origin}${redirect.source}`, {
      redirect: 'manual',
      signal: AbortSignal.timeout(10_000),
    });
    if (response.status !== 308) {
      throw new Error(`Legacy route ${redirect.source} returned HTTP ${response.status}, expected 308`);
    }
    const location = response.headers.get('location');
    if (!location || new URL(location, origin).pathname !== redirect.destination) {
      throw new Error(`Legacy route ${redirect.source} did not redirect to ${redirect.destination}`);
    }
  }

  console.log(`Security, SEO, accessibility, health, 404, and release-manifest audit passed.`);
  console.log(`Complete corpus audit passed: ${pagePaths.length} pages, ${pagePaths.length} social images, ${pagePaths.length - 1} processed MDX routes.`);
  console.log(`Discovery parity and ${legacyRedirects.length} legacy redirect audit passed.`);
  console.log(`Built-site smoke passed: ${routes.length} representative routes.`);
} finally {
  server.kill('SIGTERM');
  await Promise.race([
    new Promise((resolve) => server.once('exit', resolve)),
    delay(2_000).then(() => server.kill('SIGKILL')),
  ]);
}
