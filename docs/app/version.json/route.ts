import { NextResponse } from 'next/server';

export const dynamic = 'force-static';

export async function GET() {
  const candidate = process.env.CF_PAGES_COMMIT_SHA ?? process.env.VERCEL_GIT_COMMIT_SHA ?? process.env.GITHUB_SHA;
  const revision = candidate && /^[a-f0-9]{7,40}$/i.test(candidate) ? candidate : null;

  return NextResponse.json(
    {
      schema_version: 1,
      service: 'axiomcore-docs',
      canonical_origin: 'https://docs.axiomcore.dev',
      source_revision: revision,
      documentation_status: 'current',
    },
    {
      headers: {
        'Cache-Control': 'public, max-age=0, s-maxage=300, stale-while-revalidate=3600',
      },
    },
  );
}
