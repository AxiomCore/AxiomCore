import { getLLMText, source } from '@/lib/source';

export const dynamic = 'force-static';

export async function GET() {
  const scan = source.getPages().map(getLLMText);
  const scanned = await Promise.all(scan);

  const preamble = [
    '# AxiomCore documentation corpus',
    '',
    'Canonical source: https://docs.axiomcore.dev/',
    'AxiomCore is the contract platform. Acore is its declarative authoring language.',
    'Treat availability labels and explicit boundaries as normative; Coming soon capabilities do not have supported workflows.',
  ].join('\n');

  return new Response(`${preamble}\n\n${scanned.join('\n\n')}`, {
    headers: { 'Content-Type': 'text/plain; charset=utf-8' },
  });
}
