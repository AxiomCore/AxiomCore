import { getPageImage, source } from '@/lib/source';
import { DocsBody, DocsDescription, DocsPage, DocsTitle } from 'fumadocs-ui/layouts/docs/page';
import { notFound } from 'next/navigation';
import { getMDXComponents } from '@/mdx-components';
import type { Metadata } from 'next';
import { createRelativeLink } from 'fumadocs-ui/mdx';

export default async function Page(props: { params: Promise<{ slug?: string[] }> }) {
  const { slug } = await props.params;
  const page = source.getPage(slug);
  if (!page) notFound();

  const MDX = page.data.body;
  const canonical = `https://docs.axiomcore.dev${page.url === '/' ? '/' : page.url.replace(/\/$/, '') + '/'}`;
  const items = [{ '@type': 'ListItem', position: 1, name: 'Documentation', item: 'https://docs.axiomcore.dev/' }];
  if (page.url !== '/') items.push({ '@type': 'ListItem', position: 2, name: page.data.title, item: canonical });
  const schema = { '@context': 'https://schema.org', '@type': page.url === '/' ? 'WebSite' : 'BreadcrumbList', ...(page.url === '/' ? { name: 'AxiomCore documentation', url: canonical } : { itemListElement: items }) };

  return (
    <main className="contents">
      <script type="application/ld+json" dangerouslySetInnerHTML={{ __html: JSON.stringify(schema).replace(/</g, "\\u003c") }} />
      <DocsPage toc={page.data.toc} full={page.data.full}>
        <p className="docs-label docs-page-label">[ AxiomCore · Documentation ]</p>
        <DocsTitle>{page.data.title}</DocsTitle>
        <DocsDescription className="mb-0">{page.data.description}</DocsDescription>
        <DocsBody>
          <MDX
            components={getMDXComponents({
              a: createRelativeLink(source, page),
            })}
          />
        </DocsBody>
      </DocsPage>
    </main>
  );
}

export async function generateStaticParams() {
  return source.generateParams();
}

export async function generateMetadata(props: { params: Promise<{ slug?: string[] }> }): Promise<Metadata> {
  const { slug } = await props.params;
  const page = source.getPage(slug);
  if (!page) notFound();

  return {
    title: page.data.title,
    description: page.data.description,
    alternates: {
      canonical: page.url === "/" ? "/" : page.url.replace(/\/$/, "") + "/",
    },
    openGraph: {
      type: 'article',
      title: page.data.title,
      description: page.data.description,
      url: page.url === "/" ? "/" : page.url.replace(/\/$/, "") + "/",
      images: [{
        url: getPageImage(page).url,
        width: 1200,
        height: 630,
        alt: `${page.data.title} · AxiomCore documentation`,
      }],
    },
    twitter: {
      card: 'summary_large_image',
      title: page.data.title,
      description: page.data.description,
      images: [getPageImage(page).url],
    },
  };
}
