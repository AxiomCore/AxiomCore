// Node-only: reads the authored pages and renders the same reader Markdown that
// the static export publishes under /docs/<page>.mdx, so search results and the
// text the model reads always match the published site.
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { parseDocument, toReaderMarkdown } from '../content-model.mjs';

function walk(directory) {
  return readdirSync(directory).flatMap((entry) => {
    const path = join(directory, entry);
    if (statSync(path).isDirectory()) return walk(path);
    return entry.endsWith('.mdx') ? [path] : [];
  });
}

/**
 * Every page, in a stable order.
 * @param {string} docsRoot
 * @returns {import('./docs-index.mjs').CorpusPage[]}
 */
export function loadDocsCorpus(docsRoot) {
  const contentRoot = join(docsRoot, 'content', 'docs');
  return walk(contentRoot)
    .sort()
    .map((file) => {
      const routePath = relative(contentRoot, file).replace(/\.mdx$/, '').replace(/(^|\/)index$/, '');
      const url = routePath ? `/${routePath}` : '/';
      const source = readFileSync(file, 'utf8');
      const { title, description } = parseDocument(source).metadata;
      const canonical = `https://docs.axiomcore.dev${url === '/' ? '/' : `${url}/`}`;
      return { url, title, description, markdown: toReaderMarkdown(source, { canonical, title, description }) };
    });
}
