import { source } from '@/lib/source';
import { legacyRedirects } from '@/lib/legacy-redirects.mjs';

export const dynamic = 'force-static';

export function GET() {
  const origin = 'https://docs.axiomcore.dev';
  const pages = source.getPages().map((page) => ({
    title: page.data.title,
    description: page.data.description,
    type: page.data.pageType,
    url: `${origin}${page.url === '/' ? '/' : page.url.replace(/\/$/, '') + '/'}`,
    markdown: `${origin}/docs/${page.slugs.join('/') || 'index'}.mdx`,
    ...(page.data.testedWith ? { testedWith: page.data.testedWith } : {}),
  })).sort((a, b) => a.url.localeCompare(b.url));

  return Response.json({
    format: 'axiom-documentation-index/v1',
    canonicalOrigin: origin,
    index: `${origin}/llms.txt`,
    corpus: `${origin}/llms-full.txt`,
    startHere: [
      `${origin}/guides/using-ai-agents/`,
      `${origin}/introduction/support-matrix/`,
      `${origin}/getting-started/installation/`,
      `${origin}/getting-started/quickstart/`,
    ],
    interpretation: [
      'AxiomCore is the platform; Acore is the authoring language.',
      'Availability belongs to a specific capability and target. Coming soon is not runnable.',
      'Use page-level testedWith values as evidence for that procedure, not for the whole platform.',
      'Compilation, mocks, browser simulations, generated adapter tests, and real execution prove different things.',
      'Inspect project instructions, manifests, versions, and locks before proposing changes.',
    ],
    pageCount: pages.length,
    pages,
    redirects: legacyRedirects.map(({ source, destination }) => ({ from: source, to: destination })),
  });
}
