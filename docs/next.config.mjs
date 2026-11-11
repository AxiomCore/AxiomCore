import { createMDX } from 'fumadocs-mdx/next';
import { createSecurityHeaders } from './lib/deployment-policy.mjs';
import { legacyRedirects, legacyReaderRedirects } from './lib/legacy-redirects.mjs';

const withMDX = createMDX();

const isProduction = process.env.NODE_ENV === 'production';
const isStaticExport = process.env.AXIOM_DOCS_STATIC_EXPORT === '1';

/** @type {import('next').NextConfig} */
const config = {
  reactStrictMode: true,
  poweredByHeader: false,
  // The site is published as a static export, which has no /_next/image
  // optimizer. Serve the exported image files directly so they load.
  images: { unoptimized: true },
  ...(isStaticExport ? { output: 'export', trailingSlash: true } : {}),
  ...(!isStaticExport ? {
    async headers() {
      return [{ source: '/:path*', headers: createSecurityHeaders({ isProduction }) }];
    },
    async redirects() {
      // Next.js normalizes redirect entries in place during its build.
      return [...legacyRedirects, ...legacyReaderRedirects].map((redirect) => ({ ...redirect }));
    },
    async rewrites() {
      return [
        {
          source: '/docs/:path*.mdx',
          destination: '/llms.mdx/docs/:path*',
        },
      ];
    },
  } : {}),
};

export default withMDX(config);
