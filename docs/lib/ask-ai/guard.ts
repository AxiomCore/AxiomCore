// Abuse protection for Ask AI: the Cloudflare Turnstile human check,
// per-visitor and site-wide question limits, and a capped body reader.
//
// The Pages Function keeps the limit counters in D1, so every Cloudflare
// location shares them; `next dev` uses the in-memory limiter. Edge rules on
// the axiomcore.dev zone block bursts before the function runs (see README).

/** The Turnstile action the chat widget uses; siteverify must report it. */
export const TURNSTILE_ACTION = 'ask-ai';
/** The request header that carries a single-use Turnstile token. */
export const TURNSTILE_HEADER = 'x-turnstile-token';

/** Questions one visitor (an IPv4 address or IPv6 /64 network) may ask per window. */
export const VISITOR_LIMITS = [
  { name: 'minute', ms: 60_000, limit: 4 },
  { name: 'hour', ms: 3_600_000, limit: 20 },
  { name: 'day', ms: 86_400_000, limit: 40 },
] as const;
/** Questions per UTC day across all visitors, unless ASK_AI_DAILY_LIMIT sets another. */
export const DEFAULT_SITE_DAILY_LIMIT = 300;
const DAY_MS = 86_400_000;

export type LimitResult = { ok: true } | { ok: false; scope: 'visitor' | 'site'; retryAfter: number };

export interface AskAiLimiter {
  /** Counts one question for `visitor` and says whether it may go ahead. */
  take(visitor: string): Promise<LimitResult>;
}

interface Bucket {
  key: string;
  expires: number;
  limit: number;
  retryAfter: number;
}

function bucket(name: string, ms: number, limit: number, now: number): Bucket {
  const slot = Math.floor(now / ms);
  const end = (slot + 1) * ms;
  return { key: `${name}:${slot}`, expires: end, limit, retryAfter: Math.ceil((end - now) / 1000) };
}

// Visitor windows count first, so a visitor over their limit does not use up
// the site-wide budget. Refused questions still count, so hammering a limit
// keeps it closed.
async function take(
  visitor: string,
  siteDailyLimit: number,
  increment: (buckets: Bucket[]) => Promise<number[]>,
): Promise<LimitResult> {
  const now = Date.now();
  const windows = VISITOR_LIMITS.map((window) => bucket(`v:${visitor}:${window.name}`, window.ms, window.limit, now));
  const counts = await increment(windows);
  const over = windows.filter((window, index) => counts[index] > window.limit);
  if (over.length) return { ok: false, scope: 'visitor', retryAfter: Math.max(...over.map((window) => window.retryAfter)) };
  const site = bucket('site:day', DAY_MS, siteDailyLimit, now);
  const [count] = await increment([site]);
  if (count > site.limit) return { ok: false, scope: 'site', retryAfter: site.retryAfter };
  return { ok: true };
}

/** Counters in this process only; for `next dev` and `next start`. */
export function memoryLimiter(siteDailyLimit = DEFAULT_SITE_DAILY_LIMIT): AskAiLimiter {
  const counts = new Map<string, { count: number; expires: number }>();
  return {
    async take(visitor) {
      const now = Date.now();
      if (counts.size > 10_000) for (const [key, entry] of counts) if (entry.expires <= now) counts.delete(key);
      return take(visitor, siteDailyLimit, async (buckets) =>
        buckets.map(({ key, expires }) => {
          const entry = counts.get(key);
          if (!entry || entry.expires <= now) {
            counts.set(key, { count: 1, expires });
            return 1;
          }
          return ++entry.count;
        }),
      );
    },
  };
}

/** The part of the Cloudflare D1 binding the limiter uses. */
export interface D1Database {
  prepare(query: string): D1PreparedStatement;
  batch(statements: D1PreparedStatement[]): Promise<{ results?: unknown[] }[]>;
}

interface D1PreparedStatement {
  bind(...values: unknown[]): D1PreparedStatement;
  run(): Promise<unknown>;
}

const SCHEMA = [
  'CREATE TABLE IF NOT EXISTS ask_ai_usage (bucket TEXT PRIMARY KEY, count INTEGER NOT NULL, expires_at INTEGER NOT NULL)',
  'CREATE INDEX IF NOT EXISTS ask_ai_usage_expires ON ask_ai_usage (expires_at)',
];
const INCREMENT =
  'INSERT INTO ask_ai_usage (bucket, count, expires_at) VALUES (?1, 1, ?2) ' +
  'ON CONFLICT (bucket) DO UPDATE SET count = count + 1 RETURNING count';

// The table is created once per database and isolate.
const schemaReady = new WeakMap<D1Database, Promise<unknown>>();

function ensureSchema(db: D1Database) {
  let ready = schemaReady.get(db);
  if (!ready) {
    ready = db.batch(SCHEMA.map((sql) => db.prepare(sql))).catch((error) => {
      schemaReady.delete(db);
      throw error;
    });
    schemaReady.set(db, ready);
  }
  return ready;
}

/** Counters in D1, shared by every Cloudflare location. Rows are deleted after their window ends. */
export function d1Limiter(db: D1Database, siteDailyLimit = DEFAULT_SITE_DAILY_LIMIT): AskAiLimiter {
  return {
    async take(visitor) {
      await ensureSchema(db);
      if (Math.random() < 0.02) await db.prepare('DELETE FROM ask_ai_usage WHERE expires_at < ?1').bind(Date.now()).run();
      return take(visitor, siteDailyLimit, async (buckets) => {
        const results = await db.batch(buckets.map(({ key, expires }) => db.prepare(INCREMENT).bind(key, expires)));
        return results.map((result) => Number((result.results?.[0] as { count?: unknown } | undefined)?.count ?? Infinity));
      });
    },
  };
}

/**
 * The network a client address belongs to: the IPv4 address itself, or the
 * /64 prefix of an IPv6 address (one IPv6 user can rotate through a whole /64).
 */
export function networkOf(address: string): string {
  const ip = address.trim().toLowerCase();
  if (!ip.includes(':')) return ip;
  const mapped = /^::ffff:(\d{1,3}(?:\.\d{1,3}){3})$/.exec(ip);
  if (mapped) return mapped[1];
  const [head, tail] = ip.split('::');
  const left = head ? head.split(':') : [];
  const right = tail ? tail.split(':') : [];
  const groups = tail === undefined ? left : [...left, ...Array(Math.max(0, 8 - left.length - right.length)).fill('0'), ...right];
  return `${groups.slice(0, 4).map((group) => group.replace(/^0+(?=.)/, '')).join(':')}::/64`;
}

/** A keyed one-way hash of the client's network, so raw addresses are never stored. */
export async function visitorKey(address: string | null | undefined, secret: string | undefined): Promise<string> {
  const encoder = new TextEncoder();
  const data = encoder.encode(networkOf(address || 'unknown'));
  const digest = secret
    ? await crypto.subtle.sign(
        'HMAC',
        await crypto.subtle.importKey('raw', encoder.encode(secret), { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']),
        data,
      )
    : await crypto.subtle.digest('SHA-256', data);
  return [...new Uint8Array(digest)].slice(0, 16).map((byte) => byte.toString(16).padStart(2, '0')).join('');
}

const SITEVERIFY = 'https://challenges.cloudflare.com/turnstile/v0/siteverify';

interface SiteverifyResult {
  success?: boolean;
  action?: string;
  hostname?: string;
  metadata?: { result_with_testing_key?: boolean };
}

/**
 * Checks a Turnstile token with Cloudflare. Throws when Cloudflare cannot be
 * reached, so the caller can refuse the request rather than skip the check.
 */
export async function verifyTurnstile(
  token: string | null,
  options: { secret: string; remoteIp?: string | null; hostname: (hostname: string) => boolean },
): Promise<boolean> {
  if (!token || token.length > 2048) return false;
  const form = new FormData();
  form.append('secret', options.secret);
  form.append('response', token);
  if (options.remoteIp) form.append('remoteip', options.remoteIp);
  const response = await fetch(SITEVERIFY, { method: 'POST', body: form, signal: AbortSignal.timeout(8_000) });
  if (!response.ok) throw new Error(`Turnstile siteverify returned HTTP ${response.status}`);
  const result = (await response.json()) as SiteverifyResult;
  if (!result.success) return false;
  // Cloudflare's test keys (local development) report no action and example.com.
  if (result.metadata?.result_with_testing_key) return true;
  return result.action === TURNSTILE_ACTION && options.hostname(result.hostname ?? '');
}

/** Reads a request body up to `maxBytes`, or returns null when it is larger. */
export async function readLimitedText(request: Request, maxBytes: number): Promise<string | null> {
  if (Number(request.headers.get('content-length') ?? 0) > maxBytes) return null;
  if (!request.body) return '';
  const reader = request.body.getReader();
  const chunks: Uint8Array[] = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > maxBytes) {
      await reader.cancel();
      return null;
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return new TextDecoder().decode(bytes);
}
