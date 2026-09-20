import { DocsLayout } from 'fumadocs-ui/layouts/docs';
import { RootProvider } from 'fumadocs-ui/provider/next';
import type { ReactNode } from 'react';
import { source } from '@/lib/source';
import './global.css';
import { Inter } from 'next/font/google';
import { Analytics } from '@vercel/analytics/next';
import type { Metadata } from 'next';

export const metadata: Metadata = {
  metadataBase: new URL('https://docs.axiomcore.dev'),
  title: {
    default: 'AxiomCore documentation',
    template: '%s · AxiomCore',
  },
  description: 'Build, connect, test, and evolve software through typed, auditable contracts.',
  alternates: {
    canonical: '/',
  },
  openGraph: {
    type: 'website',
    siteName: 'AxiomCore documentation',
    title: 'AxiomCore documentation',
    description: 'Build, connect, test, and evolve software through typed, auditable contracts.',
    url: '/',
    images: [{ url: '/og/docs/image.png', width: 1200, height: 630, alt: 'AxiomCore documentation' }],
  },
  twitter: {
    card: 'summary_large_image',
    title: 'AxiomCore documentation',
    description: 'Build, connect, test, and evolve software through typed, auditable contracts.',
    images: ['/og/docs/image.png'],
  },
  robots: {
    index: true,
    follow: true,
  },
};

const inter = Inter({
  subsets: ['latin'],
});

const Logo = () => (
  <svg aria-hidden="true" focusable="false" width="24" height="24" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg" className="mr-2">
    <path d="M12 2L22 12L12 22L2 12L12 2Z" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round" />
    <path d="M12 6L18 12L12 18L6 12L12 6Z" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round" />
    <path d="M12 9L15 12L12 15L9 12L12 9Z" fill="currentColor" />
  </svg>
);

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className={inter.className} suppressHydrationWarning>
      <body className="flex flex-col min-h-screen">
        <RootProvider search={{ options: { type: 'static' } }}>
          <DocsLayout
            tree={source.pageTree}
            githubUrl="https://github.com/AxiomCore/AxiomCore"
            nav={{
              title: (
                <div className="flex items-center font-bold">
                  <Logo />
                  <span>AxiomCore</span>
                </div>
              ),
              url: 'https://axiomcore.dev',
            }}
          >
            {children}
          </DocsLayout>
        </RootProvider>
        {process.env.VERCEL ? <Analytics /> : null}
      </body>
    </html>
  );
}
