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
  lines.push('## Start here');
  lines.push('');
  lines.push('- Agent workflow: https://docs.axiomcore.dev/guides/using-ai-agents/');
  lines.push('- Availability: https://docs.axiomcore.dev/introduction/support-matrix/');
  lines.push('- First app: https://docs.axiomcore.dev/getting-started/quickstart/');
  lines.push('- Existing stacks: https://docs.axiomcore.dev/guides/');
  lines.push('- Structured index: https://docs.axiomcore.dev/agents.json');
  lines.push('');
  lines.push('Fetch /docs/<path>.mdx for one page, /docs/index.mdx for the home page, or /llms-full.txt for the full corpus.');
  lines.push('Check tool versions and project instructions; never infer runtime or target support from accepted syntax.');
  lines.push('');
  lines.push('## Page index');
  lines.push('');
  for (const page of source.getPages().sort((a, b) => a.url.localeCompare(b.url))) {
    lines.push(`- [${page.data.title}](${origin}${page.url === "/" ? "/" : page.url.replace(/\/$/, "") + "/"}): ${page.data.description}`);
  }
  return new Response(lines.join('\n'), {
    headers: { 'Content-Type': 'text/plain; charset=utf-8' },
  });
}
