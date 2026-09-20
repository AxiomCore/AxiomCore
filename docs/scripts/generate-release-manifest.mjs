import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';
import process from 'node:process';

const docsRoot = resolve(import.meta.dirname, '..');
const repositoryRoot = resolve(docsRoot, '..');
const nextRoot = join(docsRoot, '.next');
const outputPath = join(docsRoot, 'artifacts', 'docs-release-manifest.json');

function sha256(bytes) {
  return createHash('sha256').update(bytes).digest('hex');
}

function gitFiles(...pathspecs) {
  const output = execFileSync(
    'git',
    ['ls-files', '--cached', '--others', '--exclude-standard', '-z', '--', ...pathspecs],
    { cwd: repositoryRoot, encoding: 'utf8' },
  );
  return output.split('\0').filter(Boolean).filter((path) => existsSync(join(repositoryRoot, path))).sort();
}

function hashFiles(paths) {
  const hash = createHash('sha256');
  for (const path of paths) {
    hash.update(path);
    hash.update('\0');
    hash.update(readFileSync(join(repositoryRoot, path)));
    hash.update('\0');
  }
  return hash.digest('hex');
}

function sourceRevision() {
  for (const candidate of [process.env.VERCEL_GIT_COMMIT_SHA, process.env.GITHUB_SHA]) {
    if (candidate && /^[a-f0-9]{7,40}$/i.test(candidate)) return candidate;
  }
  return execFileSync('git', ['rev-parse', 'HEAD'], { cwd: repositoryRoot, encoding: 'utf8' }).trim();
}

if (!existsSync(join(nextRoot, 'prerender-manifest.json'))) {
  throw new Error('Missing .next production output. Run `pnpm build` before generating the release manifest.');
}

const siteInputs = gitFiles(
  'docs/app',
  'docs/components',
  'docs/content',
  'docs/lib',
  'docs/scripts',
  'docs/*.mjs',
  'docs/*.json',
  'docs/*.yaml',
  'docs/*.tsx',
  'docs/*.ts',
);
const verificationInputs = gitFiles('README.md', 'CONTRIBUTING.md', 'docs', 'examples');
const pages = gitFiles('docs/content/docs').filter((path) => path.endsWith('.mdx'));
const prerenderManifest = JSON.parse(readFileSync(join(nextRoot, 'prerender-manifest.json'), 'utf8'));
const appRoutes = JSON.parse(readFileSync(join(nextRoot, 'app-path-routes-manifest.json'), 'utf8'));
const packageJson = JSON.parse(readFileSync(join(docsRoot, 'package.json'), 'utf8'));

const publicOutputs = [
  'server/app/llms.txt.body',
  'server/app/llms-full.txt.body',
  'server/app/sitemap.xml.body',
  'server/app/robots.txt.body',
].map((path) => {
  const absolute = join(nextRoot, path);
  if (!existsSync(absolute)) throw new Error(`Expected production output is missing: .next/${path}`);
  const bytes = readFileSync(absolute);
  return { path: `.next/${path}`, bytes: bytes.byteLength, sha256: sha256(bytes) };
});

const dirty = execFileSync('git', ['status', '--porcelain', '--', 'README.md', 'CONTRIBUTING.md', 'docs', 'examples'], {
  cwd: repositoryRoot,
  encoding: 'utf8',
}).trim().length > 0;

const manifest = {
  schema_version: 1,
  service: 'axiomcore-docs',
  canonical_origin: 'https://docs.axiomcore.dev',
  source_revision: sourceRevision(),
  source_dirty: dirty,
  site_source_sha256: hashFiles(siteInputs),
  verification_input_sha256: hashFiles(verificationInputs),
  documented_page_count: pages.length,
  prerendered_route_count: Object.keys(prerenderManifest.routes).length,
  application_route_patterns: Object.values(appRoutes).sort(),
  public_output_proofs: publicOutputs,
  build_runtime: {
    node: process.version,
    package_manager: packageJson.packageManager,
    next: packageJson.dependencies.next,
  },
};

mkdirSync(dirname(outputPath), { recursive: true });
writeFileSync(outputPath, `${JSON.stringify(manifest, null, 2)}\n`);
console.log(`Documentation release manifest: ${relative(repositoryRoot, outputPath).split(sep).join('/')}`);
console.log(`Source proof: ${manifest.site_source_sha256}`);
console.log(`Pages: ${manifest.documented_page_count}; prerendered routes: ${manifest.prerendered_route_count}`);
