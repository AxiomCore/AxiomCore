import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const script = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '..', 'tools', 'docs-vs-binary.py');

// A stand-in for `axiom` that prints clap-style help for a small command tree.
const fakeAxiom = `#!/bin/sh
case "$*" in
"--version") echo "axiom 9.9.9" ;;
"--help") printf 'Usage: axiom <COMMAND>\\n\\nCommands:\\n  auth  Administer authentication\\n  init  \\n  help  Print this message\\n\\nOptions:\\n  -h, --help  Print help\\n' ;;
"auth --help") printf 'Usage: axiom auth [OPTIONS] <COMMAND>\\n\\nCommands:\\n  users   List and ban users\\n  status  Show whether authentication is\\n          enabled and ready\\n  help    Print this message\\n\\nOptions:\\n      --environment <ENVIRONMENT>  Environment\\n' ;;
"auth users --help") printf 'Usage: axiom auth users <COMMAND>\\n\\nCommands:\\n  list  List users\\n  ban   Ban a user\\n\\nOptions:\\n  -h, --help  Print help\\n' ;;
*) printf 'Usage: axiom leaf\\n\\nOptions:\\n  -h, --help  Print help\\n' ;;
esac
`;

function fixture(pages) {
  const dir = mkdtempSync(path.join(tmpdir(), 'docs-vs-binary-'));
  const bin = path.join(dir, 'axiom');
  writeFileSync(bin, fakeAxiom);
  chmodSync(bin, 0o755);
  const docs = path.join(dir, 'docs');
  mkdirSync(docs);
  for (const [name, text] of Object.entries(pages)) writeFileSync(path.join(docs, name), text);
  return { dir, bin, docs };
}

function run(pages) {
  const { dir, bin, docs } = fixture(pages);
  try {
    const result = spawnSync('python3', ['-I', script, '--axiom', bin, '--docs', docs, '--json'], { encoding: 'utf8' });
    return { status: result.status, report: result.status === 2 ? null : JSON.parse(result.stdout), stderr: result.stderr };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test('reports commands that no code block or inline code shows', () => {
  const { status, report } = run({
    'a.mdx': [
      'Run `axiom auth --environment production users list` to list users.',
      'The axiom auth status command is only named in prose here.',
      'An unbalanced ` backtick must not make the next paragraph code.',
      '',
      'Prose again: axiom auth status` and more.',
      '```bash title="Terminal"',
      '# axiom auth users ban is only a comment',
      'axiom init \\',
      '  --module axiom-fastapi',
      'axiom nonsense here',
      '```',
    ].join('\n'),
  });
  assert.equal(status, 1);
  assert.equal(report.binary, 'axiom 9.9.9');
  // init, auth, auth users, auth users list, auth users ban, auth status; `help` is skipped.
  assert.equal(report.commands, 6);
  assert.deepEqual(report.undocumented, ['auth status', 'auth users ban']);
  assert.deepEqual(report.docs_words_not_in_binary, { nonsense: ['a.mdx'] });
});

test('exits 0 when every command is documented', () => {
  const { status, report } = run({
    'a.mdx': '```text title="Usage"\naxiom auth status\naxiom auth users ban <USER>\n```\n',
    'b.md': 'See `axiom auth users list` and `axiom init`.',
  });
  assert.equal(status, 0);
  assert.deepEqual(report.undocumented, []);
});

test('fails with status 2 when the binary cannot run', () => {
  const result = spawnSync('python3', ['-I', script, '--axiom', '/nonexistent/axiom'], { encoding: 'utf8' });
  assert.equal(result.status, 2);
  assert.match(result.stderr, /docs-vs-binary/);
});
