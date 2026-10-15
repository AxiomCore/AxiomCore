import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseDocument, toReaderMarkdown } from '../../lib/content-model.mjs';
import { documentationIssues, resolveDocumentationReference } from '../../lib/content-policy.mjs';
import { renderDocumentationDiagram } from '../../lib/mermaid.mjs';

const header = '---\ntitle: Example\ndescription: A small documentation fixture.\npageType: guide\n---\n\n';

test('headings, explicit anchors, and links are parsed outside code', () => {
  const document = parseDocument(header + '## Run `axiom`\n\n<span id="old-run" />\n\n[Next](../next#setup)\n\n```text title="Output"\n## Not a heading\n[Not a link](/missing)\n```');
  assert.deepEqual([...document.anchors], ['run-axiom', 'old-run']);
  assert.deepEqual(document.references.map((item) => item.url), ['../next#setup']);
});

test('relative links use the public trailing-slash URL; fragments stay intact', () => {
  assert.deepEqual(resolveDocumentationReference('../sqlite#file-lifetime', '/database/contracts'), { pathname: '/database/sqlite', fragment: 'file-lifetime' });
  assert.deepEqual(resolveDocumentationReference('#check', '/backend'), { pathname: '/backend', fragment: 'check' });
  assert.equal(resolveDocumentationReference('https://example.com/elsewhere', '/backend'), null);
});

test('editorial gate rejects internal labels, private links, missing captions, and em dashes', () => {
  for (const body of ['M3 implementation', 'phase 9', 'axiom-internal-docs/plan.md', 'one\u2014two', '```acore\nproject(id: "x", version: "1")\n```']) {
    assert.ok(documentationIssues(header + body).length, body);
  }
  assert.deepEqual(documentationIssues(header + 'Cloudflare D1, KV, and R2 require separate providers.'), []);
});

test('plain readers retain availability, cards, tables, commands, and diagram source', () => {
  const source = header + `import { Card } from 'example';

<StatusNote status="experimental" title="SQLite">Local files only.</StatusNote>

<Card title="Run it" href="/getting-started" description="A working app." />

| Option | Default |
| --- | --- |
| port | 8080 |

\`\`\`bash title="Terminal"
axiom --version
\`\`\`

\`\`\`mermaid title="Request flow"
flowchart TD
 A[Client] --> B[Server]
\`\`\`
`;
  const markdown = toReaderMarkdown(source, { canonical: 'https://docs.axiomcore.dev/example/' });
  for (const value of ['Experimental: SQLite', 'Local files only.', '[Run it](/getting-started)', '8080', 'axiom --version', 'flowchart TD', 'Page type: guide']) assert.ok(markdown.includes(value), value);
  assert.doesNotMatch(markdown, /import \{|<StatusNote|<Card/);
});

test('diagrams render without external fonts or active resources', () => {
  for (const source of ['flowchart TD\n A[Client] --> B[API]', 'sequenceDiagram\n Client->>API: Read\n API-->>Client: Result', 'erDiagram\n ATTEMPT ||--o| RESULT : produces']) {
    const { svg, width, height } = renderDocumentationDiagram(source);
    assert.match(svg, /<svg/);
    assert.ok(width > 0 && height > 0);
    assert.doesNotMatch(svg, /@import|googleapis|<script|<foreignObject|href=/i);
  }
  for (const source of ['flowchart TD\n A[<script>alert(1)</script>]', 'flowchart TD\n click A "https://example.com"', '%%{init: {}}%%\nflowchart TD\n A-->B']) assert.throws(() => renderDocumentationDiagram(source));
});

test('Acore highlighting distinguishes declarations, control flow, interpolation, and comments', async () => {
  const { getHighlighter } = await import('fumadocs-core/highlight');
  const { readFile } = await import('node:fs/promises');
  const ts = await import('typescript');
  const grammar = await readFile(new URL('../../lib/acore-language.ts', import.meta.url), 'utf8');
  const js = ts.transpileModule(grammar, { compilerOptions: { module: ts.ModuleKind.ESNext } }).outputText;
  const { acoreLanguage } = await import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);
  const highlighter = await getHighlighter('oniguruma', { langs: [acoreLanguage], themes: ['github-dark'] });
  const code = 'pure fn greet(value: String): String = $"Hello, {value}"\n// public excerpt\naction hello { managed { return {message: "hello"} } }';
  const { tokens } = highlighter.codeToTokens(code, { lang: 'acore', theme: 'github-dark' });
  const flat = tokens.flat();
  const color = (value) => flat.find((token) => token.content === value)?.color;
  for (const value of ['pure', 'fn', 'greet', 'String', 'action', 'managed', 'return']) assert.ok(color(value), value);
  assert.notEqual(color('pure'), color('greet'));
  assert.notEqual(color('return'), flat.find((token) => token.content.startsWith('//'))?.color);
  assert.ok(flat.some((token) => token.content.includes('Hello,')));
});

test('frontmatter parses quoted punctuation and rejects duplicate keys', () => {
  assert.equal(parseDocument('---\ntitle: "Exam: run"\ndescription: A test\npageType: guide\n---\n').metadata.title, 'Exam: run');
  assert.throws(() => parseDocument('---\ntitle: One\ntitle: Two\n---\n'));
});
