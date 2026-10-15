import { docs } from 'fumadocs-mdx:collections/server';
import { type InferPageType, loader } from 'fumadocs-core/source';
import { lucideIconsPlugin } from 'fumadocs-core/source/lucide-icons';
import { createElement } from 'react';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { toReaderMarkdown } from './content-model.mjs';
import {
  BookIcon,
  RocketIcon,
  CpuIcon,
  CodeIcon,
  ShieldCheckIcon,
  TerminalIcon,
  MapIcon
} from 'lucide-react';

const docsOrigin = 'https://docs.axiomcore.dev';

// See https://fumadocs.dev/docs/headless/source-api for more info
export const source = loader({
  baseUrl: '/',
  source: docs.toFumadocsSource(),
  plugins: [lucideIconsPlugin()],
  icon(key) {
    if (key === 'introduction') return createElement(BookIcon);
    if (key === 'getting-started') return createElement(RocketIcon);
    if (key === 'core-concepts') return createElement(CpuIcon);
    if (key === 'guides') return createElement(MapIcon);
    if (key === 'acore-language') return createElement(CodeIcon);
    if (key === 'client-integration') return createElement(TerminalIcon);
    if (key === 'cloud-security') return createElement(ShieldCheckIcon);
    if (key === 'reference') return createElement(BookIcon);
  },
});

export function getPageImage(page: InferPageType<typeof source>) {
  const segments = [...page.slugs, 'image.png'];

  return {
    segments,
    url: `/og/docs/${segments.join('/')}`,
  };
}

export async function getLLMText(page: InferPageType<typeof source>) {
  const root = join(process.cwd(), 'content', 'docs');
  const slug = page.slugs.join('/');
  const direct = join(root, `${slug || 'index'}.mdx`);
  const path = existsSync(direct) ? direct : join(root, slug, 'index.mdx');
  return toReaderMarkdown(readFileSync(path, 'utf8'), {
    canonical: `${docsOrigin}${page.url === '/' ? '/' : page.url.replace(/\/$/, '') + '/'}`,
    title: page.data.title,
    description: page.data.description,
  });
}
