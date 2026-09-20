import { execFileSync } from 'node:child_process';
import { existsSync, renameSync } from 'node:fs';
import { join, resolve } from 'node:path';
import process from 'node:process';

const docsRoot = resolve(import.meta.dirname, '..');
const repositoryRoot = resolve(docsRoot, '..');
const nextBin = join(docsRoot, 'node_modules', 'next', 'dist', 'bin', 'next');
const readerRoute = join(docsRoot, 'app', 'llms.mdx', 'docs', '[...slug]', 'route.ts');
const disabledReaderRoute = readerRoute + '.static-disabled';

if (!existsSync(nextBin)) {
  throw new Error('Next.js is not installed. Run pnpm install --frozen-lockfile first.');
}

function gitRevision() {
  try {
    return execFileSync('git', ['rev-parse', 'HEAD'], { cwd: repositoryRoot, encoding: 'utf8' }).trim();
  } catch {
    return undefined;
  }
}

const environment = {
  ...process.env,
  AXIOM_DOCS_STATIC_EXPORT: '1',
  NEXT_TELEMETRY_DISABLED: '1',
  CF_PAGES_COMMIT_SHA: process.env.CF_PAGES_COMMIT_SHA ?? process.env.GITHUB_SHA ?? gitRevision(),
};

if (!existsSync(readerRoute) || existsSync(disabledReaderRoute)) {
  throw new Error('The server-only machine-reader route is in an unexpected state.');
}

// Next static export requires every route-handler path to be enumerable. The
// reader route has a valid server representation but its nested paths collide
// with section paths on a filesystem. Exclude it only while exporting and
// materialize the equivalent public .mdx assets in the next step.
renameSync(readerRoute, disabledReaderRoute);
try {
  execFileSync(process.execPath, [nextBin, 'build'], {
    cwd: docsRoot,
    env: environment,
    stdio: 'inherit',
  });
} finally {
  renameSync(disabledReaderRoute, readerRoute);
}

execFileSync(process.execPath, [join(docsRoot, 'scripts', 'prepare-cloudflare-output.mjs')], {
  cwd: docsRoot,
  env: environment,
  stdio: 'inherit',
});
execFileSync(process.execPath, [join(docsRoot, 'scripts', 'verify-static-export.mjs')], {
  cwd: docsRoot,
  env: environment,
  stdio: 'inherit',
});
