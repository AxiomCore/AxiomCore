export const dynamic = 'force-static';

export async function GET() {
  const body = [
    'Contact: mailto:contact@yashmakan.com',
    'Expires: 2027-09-16T23:59:59Z',
    'Canonical: https://docs.axiomcore.dev/.well-known/security.txt',
    'Policy: https://github.com/AxiomCore/AxiomCore/blob/main/SECURITY.md',
    'Preferred-Languages: en',
    '',
  ].join('\n');

  return new Response(body, {
    headers: {
      'Content-Type': 'text/plain; charset=utf-8',
      'Cache-Control': 'public, max-age=3600, s-maxage=86400',
    },
  });
}
