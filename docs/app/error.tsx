'use client';

import { useEffect } from 'react';

export default function ErrorPage({ error, reset }: { error: Error & { digest?: string }; reset: () => void }) {
  useEffect(() => {
    console.error('Documentation route failed', error.digest ?? 'no-digest');
  }, [error]);

  return (
    <main className="mx-auto flex min-h-[70vh] max-w-2xl flex-col justify-center px-6 py-16">
      <p className="mb-3 text-sm font-semibold uppercase tracking-[0.18em] text-fd-muted-foreground">
        Documentation error
      </p>
      <h1 className="text-4xl font-bold tracking-tight">This page could not be rendered</h1>
      <p className="mt-4 text-fd-muted-foreground">
        Retry once. If the failure persists, report the page address and the
        diagnostic reference without including credentials or private data.
      </p>
      {error.digest ? <p className="mt-3 font-mono text-sm">Reference: {error.digest}</p> : null}
      <div className="mt-8 flex flex-wrap gap-3">
        <button className="rounded-md bg-fd-primary px-4 py-2 font-medium text-fd-primary-foreground" onClick={reset} type="button">
          Retry
        </button>
        <a className="rounded-md border border-fd-border px-4 py-2 font-medium" href="/reference/support-and-feedback">
          Support guidance
        </a>
      </div>
    </main>
  );
}
