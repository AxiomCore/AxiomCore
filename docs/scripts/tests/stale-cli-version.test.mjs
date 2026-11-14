import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const docs = process.env.DOCS_CONTENT_DIR || path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '..', 'content', 'docs');

function pages(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) return pages(full);
    return /\.mdx?$/.test(entry.name) ? [full] : [];
  });
}

// Statements that CLI 0.151.0 made untrue: it ships `axiom secrets`,
// `axiom project resources` and a non-interactive `project create`, and the
// current release is 0.151.0 with UI host 0.8.0.
const stale = [
  [/secrets[^.\n]{0,80}\(source preview\)/i, 'the secrets commands are released in 0.151.0'],
  [/title:\s*Secrets commands \(source preview\)/i, 'the secrets commands are released in 0.151.0'],
  [/(?:are|is) not in (?:the published )?CLI 0\.149\.1|0\.149\.1 does not include `axiom/i, 'these commands are in CLI 0.151.0'],
  [/Source preview; not in the published CLI/i, 'the command is in the published CLI 0.151.0'],
  [/In (?:CLI )?0\.149\.1,? (?:the command always needs|`project create` needs) an interactive terminal/i, 'project create runs without a terminal when --name and --slug are given'],
  [/This reference covers the released CLI \*\*0\.149\.1\*\*/, 'the CLI overview covers 0.151.0'],
  [/The Axiom CLI release is \*\*0\.149\.1\*\*/, 'the current release is 0.151.0'],
  [/VERSION=0\.149\.1|releases\/download\/cli-v0\.149\.1\//, 'install commands must name the current release'],
  [/host install --target web --version 0\.6\.12/, 'the current web UI host is 0.8.0'],
];

test('no page claims CLI 0.149.1 limits that 0.151.0 removed', () => {
  const found = [];
  for (const file of pages(docs)) {
    const lines = readFileSync(file, 'utf8').split('\n');
    lines.forEach((line, index) => {
      for (const [pattern, reason] of stale) {
        if (pattern.test(line)) found.push(`${path.relative(docs, file)}:${index + 1}: ${reason}: ${line.trim()}`);
      }
    });
  }
  assert.deepEqual(found, []);
});
