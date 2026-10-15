import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { createCloudflareHeaders } from '../lib/deployment-policy.mjs';
import { legacyRedirects, legacyReaderRedirects } from '../lib/legacy-redirects.mjs';
import { parseDocument, toReaderMarkdown } from '../lib/content-model.mjs';

const docsRoot = resolve(import.meta.dirname, '..');
const outputRoot = join(docsRoot, 'out');
const contentRoot = join(docsRoot, 'content', 'docs');

if (!existsSync(outputRoot)) {
  throw new Error('Static export output is missing. Run the Cloudflare static build first.');
}

const redirects = legacyRedirects
  .flatMap(({ source, destination, permanent }) => [source, `${source}/`].map((from) => `${from} ${destination}/ ${permanent ? '301' : '302'}`))
  .concat(legacyReaderRedirects.map(({ source, destination }) => `${source} ${destination} 301`))
  .join('\n');

mkdirSync(outputRoot, { recursive: true });
writeFileSync(join(outputRoot, '_headers'), createCloudflareHeaders(), 'utf8');
writeFileSync(join(outputRoot, '_redirects'), redirects + '\n', 'utf8');

function walk(directory) {
  const files = [];
  for (const entry of readdirSync(directory)) {
    const path = join(directory, entry);
    if (statSync(path).isDirectory()) files.push(...walk(path));
    else if (entry.endsWith('.mdx')) files.push(path);
  }
  return files;
}

function createReaderAsset(sourcePath) {
  const relativePath = relative(contentRoot, sourcePath);
  const routePath = relativePath.replace(/\.mdx$/, '').replace(/\/index$/, '');
  if (!routePath) return null;

  const source = readFileSync(sourcePath, 'utf8');
  const { title, description } = parseDocument(source).metadata;
  const destination = join(outputRoot, 'docs', routePath + '.mdx');
  const sourceUrl = 'https://docs.axiomcore.dev/' + (routePath === 'index' ? '' : routePath + '/');

  mkdirSync(resolve(destination, '..'), { recursive: true });
  writeFileSync(
    destination,
    toReaderMarkdown(source, { canonical: sourceUrl, title, description }),
  );

  return destination;
}

if (!existsSync(contentRoot)) {
  throw new Error('Authored documentation content is missing.');
}

const readerAssets = walk(contentRoot).map(createReaderAsset).filter(Boolean);
if (!readerAssets.length) {
  throw new Error('No static machine-reader assets were generated.');
}

console.log(
  'Prepared Cloudflare Pages assets: ' +
    legacyRedirects.length +
    ' redirect(s), ' +
    readerAssets.length +
    ' machine-reader file(s), shared security headers.',
);
