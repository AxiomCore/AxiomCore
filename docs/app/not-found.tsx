import Link from 'next/link';

export default function NotFound() {
  return (
    <main className="mx-auto flex min-h-[70vh] max-w-2xl flex-col justify-center px-6 py-16">
      <p className="mb-3 text-sm font-semibold uppercase tracking-[0.18em] text-fd-muted-foreground">
        Error 404
      </p>
      <h1 className="text-4xl font-bold tracking-tight">Documentation page not found</h1>
      <p className="mt-4 text-fd-muted-foreground">
        The address may be outdated or the page may have moved. Search the current
        documentation before relying on an older command or compatibility claim.
      </p>
      <div className="mt-8 flex flex-wrap gap-3">
        <Link className="rounded-md bg-fd-primary px-4 py-2 font-medium text-fd-primary-foreground" href="/">
          Documentation home
        </Link>
        <Link className="rounded-md border border-fd-border px-4 py-2 font-medium" href="/reference/support-and-feedback">
          Report a broken link
        </Link>
      </div>
    </main>
  );
}
