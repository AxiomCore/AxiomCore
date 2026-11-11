// Cloudflare Pages Function for Ask AI (POST /api/chat). The docs site is a
// static export; this is its only server code. Configuration comes from the
// Pages project: OPENROUTER_API_KEY, DOCS_AI_MODEL to choose the OpenRouter
// model, TURNSTILE_SECRET_KEY, ASK_AI_HASH_SECRET, the ASK_AI_DB D1 binding for
// the question limits, and optionally ASK_AI_DAILY_LIMIT.
import { handleAskAi, type DocsIndex } from '../../lib/ask-ai/chat';
import { ASK_AI_INDEX_PATH } from '../../lib/ask-ai/docs-index.mjs';
import { d1Limiter, DEFAULT_SITE_DAILY_LIMIT, type D1Database } from '../../lib/ask-ai/guard';

interface Env {
  ASSETS: { fetch(request: Request): Promise<Response> };
  OPENROUTER_API_KEY?: string;
  DOCS_AI_MODEL?: string;
  TURNSTILE_SECRET_KEY?: string;
  ASK_AI_HASH_SECRET?: string;
  ASK_AI_DAILY_LIMIT?: string;
  ASK_AI_DB?: D1Database;
}

interface PagesContext {
  request: Request;
  env: Env;
}

// Parsed once per isolate and reused by later requests.
let indexPromise: Promise<DocsIndex> | null = null;

export async function onRequestPost({ request, env }: PagesContext): Promise<Response> {
  // Without the human check or the shared limits the endpoint stays closed.
  if (!env.TURNSTILE_SECRET_KEY || !env.ASK_AI_DB) {
    console.error('Ask AI is missing TURNSTILE_SECRET_KEY or the ASK_AI_DB binding.');
    return new Response(JSON.stringify({ error: 'Ask AI is not configured.' }), {
      status: 503,
      headers: { 'content-type': 'application/json; charset=utf-8', 'cache-control': 'no-store' },
    });
  }
  const origin = new URL(request.url).origin;
  const asset = (path: string) => env.ASSETS.fetch(new Request(new URL(path, origin)));
  const dailyLimit = Math.floor(Number(env.ASK_AI_DAILY_LIMIT));

  return handleAskAi(request, {
    apiKey: env.OPENROUTER_API_KEY,
    model: env.DOCS_AI_MODEL,
    limiter: d1Limiter(env.ASK_AI_DB, dailyLimit > 0 ? dailyLimit : DEFAULT_SITE_DAILY_LIMIT),
    clientAddress: request.headers.get('cf-connecting-ip'),
    hashSecret: env.ASK_AI_HASH_SECRET,
    turnstileSecret: env.TURNSTILE_SECRET_KEY,
    loadIndex() {
      indexPromise ??= asset(ASK_AI_INDEX_PATH).then((response) => {
        if (!response.ok) throw new Error(`Ask AI index unavailable (HTTP ${response.status})`);
        return response.json() as Promise<DocsIndex>;
      });
      indexPromise.catch(() => {
        indexPromise = null;
      });
      return indexPromise;
    },
    async readMarkdown(path) {
      const response = await asset(path);
      return response.ok ? response.text() : null;
    },
  });
}

export function onRequest(): Response {
  return new Response(JSON.stringify({ error: 'Use POST.' }), {
    status: 405,
    headers: { 'content-type': 'application/json; charset=utf-8', allow: 'POST' },
  });
}
