import { copyFileSync, mkdirSync } from 'node:fs';
mkdirSync('public', { recursive: true });
copyFileSync('node_modules/atmx-web/dist/axiom_runtime.wasm', 'public/axiom_runtime.wasm');
