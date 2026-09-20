import type { ReactNode } from 'react';

export type AvailabilityStatus =
  | 'available'
  | 'alpha'
  | 'experimental'
  | 'coming-soon';

const labels: Record<AvailabilityStatus, string> = {
  available: 'Available',
  alpha: 'Alpha',
  experimental: 'Experimental',
  'coming-soon': 'Coming soon',
};

const styles: Record<AvailabilityStatus, string> = {
  available:
    'border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300',
  alpha: 'border-blue-500/30 bg-blue-500/10 text-blue-700 dark:text-blue-300',
  experimental:
    'border-amber-500/30 bg-amber-500/10 text-amber-800 dark:text-amber-300',
  'coming-soon':
    'border-fd-border bg-fd-muted text-fd-muted-foreground',
};

export function StatusBadge({ status }: { status: AvailabilityStatus }) {
  return (
    <span
      className={`not-prose inline-flex items-center rounded-full border px-2.5 py-0.5 text-xs font-semibold leading-5 ${styles[status]}`}
      aria-label={`Availability: ${labels[status]}`}
    >
      {labels[status]}
    </span>
  );
}

export interface CapabilityRow {
  capability: string;
  status: AvailabilityStatus;
  scope: ReactNode;
}

export function CapabilityTable({
  rows,
  caption = 'Capability availability',
}: {
  rows: CapabilityRow[];
  caption?: string;
}) {
  return (
    <div className="not-prose my-6 overflow-x-auto rounded-xl border border-fd-border">
      <table className="w-full border-collapse text-left text-sm">
        <caption className="sr-only">{caption}</caption>
        <thead className="bg-fd-muted">
          <tr>
            <th scope="col" className="px-4 py-3 font-semibold">Capability</th>
            <th scope="col" className="px-4 py-3 font-semibold">Status</th>
            <th scope="col" className="px-4 py-3 font-semibold">Current scope</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.capability} className="border-t border-fd-border align-top">
              <th scope="row" className="px-4 py-3 font-medium">{row.capability}</th>
              <td className="whitespace-nowrap px-4 py-3">
                <StatusBadge status={row.status} />
              </td>
              <td className="px-4 py-3 text-fd-muted-foreground">{row.scope}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function StatusNote({
  status,
  title,
  children,
}: {
  status: AvailabilityStatus;
  title: string;
  children: ReactNode;
}) {
  return (
    <aside className="not-prose my-6 rounded-xl border border-fd-border bg-fd-card p-4">
      <div className="mb-2 flex flex-wrap items-center gap-2">
        <strong>{title}</strong>
        <StatusBadge status={status} />
      </div>
      <div className="text-sm leading-6 text-fd-muted-foreground">{children}</div>
    </aside>
  );
}
