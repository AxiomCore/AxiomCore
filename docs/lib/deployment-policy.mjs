export function createContentSecurityPolicy({ isProduction }) {
  const ga = /^G-[A-Z0-9]+$/.test(process.env.NEXT_PUBLIC_GA4_MEASUREMENT_ID || 'G-MLGBL9YG66');
  return [
    "default-src 'self'",
    `script-src 'self' 'unsafe-inline' https://static.cloudflareinsights.com https://www.clarity.ms https://*.clarity.ms${ga ? " https://www.googletagmanager.com" : ""}${isProduction ? '' : " 'unsafe-eval'"}`,
    "style-src 'self' 'unsafe-inline'",
    "img-src 'self' data: blob: https://*.clarity.ms https://c.bing.com",
    "font-src 'self' data:",
    `connect-src 'self' https://cloudflareinsights.com https://*.clarity.ms https://c.bing.com${ga ? " https://*.google-analytics.com https://www.googletagmanager.com" : ""}`,
    "worker-src 'self' blob:",
    "object-src 'none'",
    "base-uri 'self'",
    "form-action 'self'",
    "frame-ancestors 'none'",
    ...(isProduction ? ['upgrade-insecure-requests'] : []),
  ].join('; ');
}

export function createSecurityHeaders({ isProduction }) {
  return [
    { key: 'Content-Security-Policy', value: createContentSecurityPolicy({ isProduction }) },
    { key: 'Cross-Origin-Opener-Policy', value: 'same-origin' },
    { key: 'Permissions-Policy', value: 'camera=(), geolocation=(), microphone=(), payment=(), usb=()' },
    { key: 'Referrer-Policy', value: 'strict-origin-when-cross-origin' },
    { key: 'Strict-Transport-Security', value: 'max-age=31536000; includeSubDomains' },
    { key: 'X-Content-Type-Options', value: 'nosniff' },
    { key: 'X-Frame-Options', value: 'DENY' },
  ];
}

export function createCloudflareHeaders() {
  const global = createSecurityHeaders({ isProduction: true })
    .map(({ key, value }) => `  ${key}: ${value}`)
    .join('\n');

  return `/*\n${global}\n\nhttps://:project.pages.dev/*\n  X-Robots-Tag: noindex\n\nhttps://:version.:project.pages.dev/*\n  X-Robots-Tag: noindex\n\n/_next/static/*\n  Cache-Control: public, max-age=31536000, immutable\n\n/api/health\n  Content-Type: application/json; charset=utf-8\n  Cache-Control: no-store\n\n/api/search\n  Content-Type: application/json; charset=utf-8\n\n/docs/*.mdx\n  Content-Type: text/markdown; charset=utf-8\n\n/version.json\n  Cache-Control: public, max-age=0, s-maxage=300, stale-while-revalidate=3600\n\n/.well-known/security.txt\n  Cache-Control: public, max-age=3600, s-maxage=86400\n`;
}
