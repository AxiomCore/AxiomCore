import type { MetadataRoute } from 'next';
import { source } from '@/lib/source';

export const dynamic = 'force-static';

const origin = 'https://docs.axiomcore.dev';

export default function sitemap(): MetadataRoute.Sitemap {
  return source.getPages().map((page) => ({
    url: `${origin}${page.url}`,
    changeFrequency: page.url === '/' ? 'weekly' : 'monthly',
    priority: page.url === '/' ? 1 : 0.7,
  }));
}
