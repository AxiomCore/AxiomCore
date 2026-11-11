// Ask AI for `next dev` and `next start`. Production is a static export served
// by Cloudflare Pages, where functions/api/chat.ts answers instead; the static
// build leaves this route out. Both use lib/ask-ai/chat.ts. Limits are kept in
// memory here, and the Turnstile check runs only when TURNSTILE_SECRET_KEY is set.
import { handleAskAi, type DocsIndex } from '@/lib/ask-ai/chat';
import { loadDocsCorpus } from '@/lib/ask-ai/corpus.mjs';
import { buildDocsIndex, readerPathFor } from '@/lib/ask-ai/docs-index.mjs';
import { memoryLimiter } from '@/lib/ask-ai/guard';

export const dynamic = 'force-dynamic';

let corpus: ReturnType<typeof loadDocsCorpus> | null = null;
let index: DocsIndex | null = null;
const limiter = memoryLimiter();

function load() {
  // Rebuild in development so edited pages are searchable without a restart.
  if (!corpus || process.env.NODE_ENV === 'development') {
    corpus = loadDocsCorpus(process.cwd());
    index = buildDocsIndex(corpus);
  }
  return { corpus, index: index! };
}

export async function POST(request: Request): Promise<Response> {
  return handleAskAi(request, {
    apiKey: process.env.OPENROUTER_API_KEY,
    model: process.env.DOCS_AI_MODEL,
    limiter,
    clientAddress: request.headers.get('x-forwarded-for')?.split(',')[0]?.trim() ?? null,
    hashSecret: process.env.ASK_AI_HASH_SECRET,
    turnstileSecret: process.env.TURNSTILE_SECRET_KEY,
    async loadIndex() {
      return load().index;
    },
    async readMarkdown(path) {
      const { corpus: pages, index: current } = load();
      const page = pages.find((candidate) => readerPathFor(current, candidate.url) === path);
      return page?.markdown ?? null;
    },
  });
}
