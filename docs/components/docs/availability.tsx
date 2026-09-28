import type { ReactNode } from "react";

export type AvailabilityStatus =
  | "available"
  | "alpha"
  | "experimental"
  | "coming-soon";

const labels: Record<AvailabilityStatus, string> = {
  available: "Available",
  alpha: "Alpha",
  experimental: "Experimental",
  "coming-soon": "Coming soon",
};

export function StatusBadge({ status }: { status: AvailabilityStatus }) {
  return (
    <span
      className="docs-status not-prose inline-flex items-center"
      data-status={status}
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
  caption = "Capability availability",
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
            <th scope="col" className="px-4 py-3 font-semibold">
              Capability
            </th>
            <th scope="col" className="px-4 py-3 font-semibold">
              Status
            </th>
            <th scope="col" className="px-4 py-3 font-semibold">
              Current scope
            </th>
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr
              key={row.capability}
              className="border-t border-fd-border align-top"
            >
              <th scope="row" className="px-4 py-3 font-medium">
                {row.capability}
              </th>
              <td className="whitespace-nowrap px-4 py-3">
                <StatusBadge status={row.status} />
              </td>
              <td className="px-4 py-3 text-fd-muted-foreground">
                {row.scope}
              </td>
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
    <aside className="docs-status-note not-prose my-6 border border-fd-border bg-fd-card">
      <div className="mb-2 flex flex-wrap items-center gap-2">
        <strong>{title}</strong>
        <StatusBadge status={status} />
      </div>
      <div className="text-sm leading-6 text-fd-muted-foreground">
        {children}
      </div>
    </aside>
  );
}
