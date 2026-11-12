// Dependency-free build: strip the TypeScript of main.ts and copy the installed
// @axiomcore/auth-web module next to it. Output: dist/ (served as static files).
import { stripTypeScriptTypes } from 'node:module';
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const sdk = require.resolve('@axiomcore/auth-web');
mkdirSync('dist/auth/callback', { recursive: true });
const js = stripTypeScriptTypes(readFileSync('main.ts', 'utf8'), { mode: 'strip' }).replace("from '@axiomcore/auth-web'", "from './auth-web.js'");
writeFileSync('dist/main.js', js);
copyFileSync(sdk, 'dist/auth-web.js');
const api = process.env.API_URL ?? 'http://127.0.0.1:8080';
// The page may connect to itself, the issuer and the configured API origin only.
const html = readFileSync('index.html', 'utf8').replace('http://127.0.0.1:8080', new URL(api).origin);
writeFileSync('dist/index.html', html);
writeFileSync('dist/auth/callback/index.html', html);
writeFileSync('dist/config.json', JSON.stringify({ issuer: process.env.AXIOM_ISSUER ?? '', audience: process.env.AXIOM_AUDIENCE ?? '', api }) + '\n');
console.log('built dist/ with @axiomcore/auth-web from', sdk.replace(process.cwd(), '.'));
