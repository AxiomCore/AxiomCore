import { getLLMText, source } from '@/lib/source';
import { notFound } from 'next/navigation';

export const dynamic = 'force-static';
export const dynamicParams = false;

export async function GET(_req: Request, { params }: RouteContext<'/llms.mdx/docs/[...slug]'>) {
  const { slug } = await params;
  const page = source.getPage(slug);
  if (!page) notFound();

  return new Response(await getLLMText(page), {
    headers: {
      'Content-Type': 'text/markdown',
    },
  });
}

export function generateStaticParams() {
  // Cloudflare's static build emits the public reader assets directly from the
  // authored MDX files. A filesystem cannot represent both
  // /docs/<section>.mdx and /docs/<section>/<page>.mdx as route-handler
  // output, so leave this server route empty in that build only.
  if (process.env.AXIOM_DOCS_STATIC_EXPORT === '1') {
    return [];
  }

  return source.generateParams().filter((params) => params.slug && params.slug.length > 0);
}
