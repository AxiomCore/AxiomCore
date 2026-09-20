import { source } from '@/lib/source';

export const dynamic = 'force-static';

export async function GET() {
  const origin = 'https://docs.axiomcore.dev';
  const lines: string[] = [];
  lines.push('# AxiomCore documentation');
  lines.push('');
  lines.push('AxiomCore connects software through typed, auditable contracts. Acore is its declarative authoring language.');
  lines.push('Capability status and boundaries are part of the contract: Available, Alpha, Experimental, or Coming soon.');
  lines.push('');
  for (const page of source.getPages()) {
    lines.push(`- [${page.data.title}](${origin}${page.url}): ${page.data.description}`);
  }
  return new Response(lines.join('\n'), {
    headers: { 'Content-Type': 'text/plain; charset=utf-8' },
  });
}
