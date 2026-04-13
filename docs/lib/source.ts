import { docs } from 'fumadocs-mdx:collections/server';
import { type InferPageType, loader } from 'fumadocs-core/source';
import { lucideIconsPlugin } from 'fumadocs-core/source/lucide-icons';
import { createElement } from 'react';
import {
  BookIcon,
  RocketIcon,
  CpuIcon,
  CodeIcon,
  ShieldCheckIcon,
  TerminalIcon,
  MapIcon
} from 'lucide-react';

// See https://fumadocs.dev/docs/headless/source-api for more info
export const source = loader({
  baseUrl: '/',
  source: docs.toFumadocsSource(),
  plugins: [lucideIconsPlugin()],
  icon(key) {
    if (key === 'introduction') return createElement(BookIcon);
    if (key === 'getting-started') return createElement(RocketIcon);
    if (key === 'core-concepts') return createElement(CpuIcon);
    if (key === 'acore-language') return createElement(CodeIcon);
    if (key === 'client-integration') return createElement(TerminalIcon);
    if (key === 'cloud-security') return createElement(ShieldCheckIcon);
    if (key === 'reference') return createElement(BookIcon);
    if (key === 'roadmap') return createElement(MapIcon);
  },
});

export function getPageImage(page: InferPageType<typeof source>) {
  const segments = [...page.slugs, 'image.png'];

  return {
    segments,
    url: `/og/${segments.join('/')}`,
  };
}

export async function getLLMText(page: InferPageType<typeof source>) {
  const processed = await page.data.getText('processed');

  return `# ${page.data.title}

${processed}`;
}
