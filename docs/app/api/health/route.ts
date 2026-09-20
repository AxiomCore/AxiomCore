import { NextResponse } from 'next/server';

export const dynamic = 'force-static';

function sourceRevision() {
  const candidate = process.env.CF_PAGES_COMMIT_SHA ?? process.env.VERCEL_GIT_COMMIT_SHA ?? process.env.GITHUB_SHA;
  return candidate && /^[a-f0-9]{7,40}$/i.test(candidate) ? candidate : null;
}

export async function GET() {
  return NextResponse.json(
    {
      schema_version: 1,
      status: 'ok',
      service: 'axiomcore-docs',
      source_revision: sourceRevision(),
    },
    {
      headers: {
        'Cache-Control': 'no-store',
      },
    },
  );
}
