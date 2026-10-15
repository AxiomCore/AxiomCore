import { parseDocument, walkNodes } from './content-model.mjs';
import { renderDocumentationDiagram } from './mermaid.mjs';

export const pageTypes = new Set(['overview', 'concept', 'guide', 'tutorial', 'reference']);
const languages = new Set(['acore', 'bash', 'sh', 'text', 'json', 'toml', 'sql', 'typescript', 'ts', 'tsx', 'javascript', 'js', 'jsx', 'html', 'css', 'python', 'go', 'rust', 'dart', 'yaml', 'yml', 'mermaid']);

export function documentationIssues(source, document = parseDocument(source)) {
  const issues = [];
  if (source.includes('\u2014')) issues.push('contains an em dash; rephrase the authored text or example');
  if (!pageTypes.has(document.metadata.pageType)) issues.push('pageType must be overview, concept, guide, tutorial, or reference');
  const publicLabels = source.replace(/<span id="[^"]+"\s*\/>/g, '');
  if (/\b(?:M[1-4]|F[0-9]|DX[0-9]+|D9)\b|\b(?:phase|milestone|workstream)\s+[0-9]+[A-Za-z]?\b/i.test(publicLabels)) {
    issues.push('contains an internal phase or milestone label');
  }
  if (/axiom-internal-docs|(?:acore|axiom-ui|axiom-runtime|axiom-lib)\/(?:src|tests)\/|frontend_application\.rs|\b[fmld]\d+-completion\b/i.test(source)) {
    issues.push('contains a private implementation or evidence reference');
  }
  if (document.duplicateAnchors.length) issues.push(`duplicate heading/compatibility anchors: ${document.duplicateAnchors.join(', ')}`);
  for (const block of document.codeBlocks) {
    const line = block.position?.start.line;
    if (!languages.has(block.lang)) issues.push(`code block at body line ${line} needs a supported language`);
    if (!/\btitle="[^"\n]+"/.test(block.meta ?? '')) issues.push(`code block at body line ${line} needs a descriptive title or filename`);
    if (block.lang === 'mermaid') {
      try { renderDocumentationDiagram(block.value); }
      catch (error) { issues.push(`Mermaid at body line ${line}: ${error.message}`); }
    }
  }
  walkNodes(document.tree, (node) => {
    if (node.name === 'CapabilityTable') issues.push('write capability tables as Markdown so text readers retain every row');
    if (node.type === 'image' && !node.alt?.trim()) issues.push('images require descriptive alt text');
  });
  return issues;
}

export function resolveDocumentationReference(reference, fromRoute) {
  if (/^(?:mailto:|tel:)/i.test(reference)) return null;
  const base = `https://docs.axiomcore.dev${fromRoute === '/' ? '/' : `${fromRoute.replace(/\/$/, '')}/`}`;
  const url = new URL(reference, base);
  if (url.origin !== 'https://docs.axiomcore.dev') return null;
  return { pathname: decodeURIComponent(url.pathname).replace(/\/$/, '') || '/', fragment: decodeURIComponent(url.hash.slice(1)) };
}
