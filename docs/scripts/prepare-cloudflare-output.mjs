import { existsSync, mkdirSync, readdirSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { join, relative, resolve } from 'node:path';
import { createCloudflareHeaders } from '../lib/deployment-policy.mjs';
import { legacyRedirects } from '../lib/legacy-redirects.mjs';

const docsRoot = resolve(import.meta.dirname, '..');
const outputRoot = join(docsRoot, 'out');
const contentRoot = join(docsRoot, 'content', 'docs');

if (!existsSync(outputRoot)) {
  throw new Error('Static export output is missing. Run the Cloudflare static build first.');
}

const redirects = legacyRedirects
  .map(({ source, destination, permanent }) => source + ' ' + destination + ' ' + (permanent ? '301' : '302'))
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

function frontmatterValue(frontmatter, field) {
  const match = frontmatter.match(new RegExp('^' + field + ':\\s*(.+)\\s*$', 'm'));
  const value = match?.[1]?.trim();

  return value?.replace(/^['"]|['"]$/g, '');
}

function createReaderAsset(sourcePath) {
  const relativePath = relative(contentRoot, sourcePath);
  const routePath = relativePath.replace(/\.mdx$/, '').replace(/\/index$/, '');
  if (!routePath) return null;

  const source = readFileSync(sourcePath, 'utf8');
  const frontmatterMatch = source.match(/^---\s*\n([\s\S]*?)\n---\s*\n?/);
  const frontmatter = frontmatterMatch?.[1] ?? '';
  const body = source.slice(frontmatterMatch?.[0].length ?? 0).trim();
  const title = frontmatterValue(frontmatter, 'title') ?? routePath;
  const description = frontmatterValue(frontmatter, 'description');
  const destination = join(outputRoot, 'docs', routePath + '.mdx');
  const sourceUrl = 'https://docs.axiomcore.dev/docs/' + routePath + '.mdx';

  mkdirSync(resolve(destination, '..'), { recursive: true });
  writeFileSync(
    destination,
    [
      '# ' + title,
      '',
      'Source: ' + sourceUrl,
      ...(description ? ['', 'Description: ' + description] : []),
      '',
      body,
      '',
    ].join('\n'),
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
