import { DocsLayout } from "fumadocs-ui/layouts/docs";
import { RootProvider } from "fumadocs-ui/provider/next";
import type { ReactNode } from "react";
import { source } from "@/lib/source";
import "./global.css";
import { Brand } from "@/components/brand";
import { NavigationAccessibility } from "@/components/docs/navigation-accessibility";
import { SiteAnalytics } from "@/components/site-analytics";
import "@/lib/brand/styles/analytics.css";
import type { Metadata } from "next";

export const metadata: Metadata = {
  metadataBase: new URL("https://docs.axiomcore.dev"),
  title: {
    default: "AxiomCore documentation",
    template: "%s · AxiomCore",
  },
  description:
    "Build, connect, test, and evolve software through typed, auditable contracts.",
  alternates: {
    canonical: "/",
  },
  openGraph: {
    type: "website",
    siteName: "AxiomCore documentation",
    title: "AxiomCore documentation",
    description:
      "Build, connect, test, and evolve software through typed, auditable contracts.",
    url: "/",
    images: [
      {
        url: "/og/docs/image.png",
        width: 1200,
        height: 630,
        alt: "AxiomCore documentation",
      },
    ],
  },
  twitter: {
    card: "summary_large_image",
    title: "AxiomCore documentation",
    description:
      "Build, connect, test, and evolve software through typed, auditable contracts.",
    images: ["/og/docs/image.png"],
  },
  robots: {
    index: true,
    follow: true,
  },
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className="dark" suppressHydrationWarning>
      <body className="flex flex-col min-h-screen">
        <RootProvider
          theme={{ forcedTheme: "dark", enableSystem: false }}
          search={{ options: { type: "static" } }}
        >
          <DocsLayout
            tree={source.pageTree}
            githubUrl="https://github.com/AxiomCore/AxiomCore"
            themeSwitch={{ enabled: false }}
            sidebar={{
              banner: (
                <p className="docs-label docs-sidebar-label">
                  [ Documentation ]
                </p>
              ),
              footer: (
                <nav
                  aria-label="AxiomCore products"
                  className="docs-product-links"
                >
                  <a href="https://playground.axiomcore.dev">
                    Playground <span aria-hidden="true">↗</span>
                  </a>
                  <a
                    href="https://app.axiomcore.dev"
                    className="docs-start-link"
                  >
                    Get started <span aria-hidden="true">↗</span>
                  </a>
                </nav>
              ),
            }}
            nav={{
              title: <Brand />,
              url: "https://axiomcore.dev",
            }}
          >
            <NavigationAccessibility />
            {children}
          </DocsLayout>
        </RootProvider>
        <SiteAnalytics />
      </body>
    </html>
  );
}
