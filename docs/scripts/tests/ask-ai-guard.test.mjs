import { test } from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import {
  d1Limiter,
  memoryLimiter,
  networkOf,
  readLimitedText,
  VISITOR_LIMITS,
  visitorKey,
} from '../../lib/ask-ai/guard.ts';

// The D1 binding API over an in-memory SQLite database, so the limiter's SQL runs for real.
function sqliteD1() {
  const db = new DatabaseSync(':memory:');
  const statement = (sql, values = []) => ({
    bind: (...next) => statement(sql, next),
    run: async () => db.prepare(sql).run(...values),
    rows: () => db.prepare(sql).all(...values),
  });
  return {
    prepare: (sql) => statement(sql),
    batch: async (statements) => statements.map((item) => ({ results: item.rows() })),
    count: () => db.prepare('SELECT COUNT(*) AS rows FROM ask_ai_usage').get().rows,
  };
}

const perMinute = VISITOR_LIMITS.find((window) => window.name === 'minute').limit;

test('visitors are IPv4 addresses or IPv6 /64 networks', () => {
  assert.equal(networkOf('203.0.113.7'), '203.0.113.7');
  assert.equal(networkOf('::ffff:203.0.113.7'), '203.0.113.7');
  assert.equal(networkOf('2001:db8:1:2:aaaa::1'), '2001:db8:1:2::/64');
  assert.equal(networkOf('2001:0db8:0001:0002:ffff:ffff:ffff:ffff'), '2001:db8:1:2::/64');
  assert.equal(networkOf('2001:db8::1'), '2001:db8:0:0::/64');
});

test('visitor keys are keyed hashes that never contain the address', async () => {
  const one = await visitorKey('2001:db8:1:2::1', 'secret');
  assert.equal(one, await visitorKey('2001:db8:1:2:ffff::9', 'secret'));
  assert.notEqual(one, await visitorKey('2001:db8:1:2::1', 'other secret'));
  assert.notEqual(one, await visitorKey('2001:db8:1:3::1', 'secret'));
  assert.match(one, /^[0-9a-f]{32}$/);
  assert.match(await visitorKey(null, undefined), /^[0-9a-f]{32}$/);
});

for (const [name, create] of [
  ['memory', (limit) => memoryLimiter(limit)],
  ['D1', (limit) => d1Limiter(sqliteD1(), limit)],
]) {
  test(`${name} limiter refuses a visitor over the per-minute limit and not others`, async () => {
    const limiter = create(100);
    for (let question = 0; question < perMinute; question++) assert.deepEqual(await limiter.take('a'), { ok: true });
    const refused = await limiter.take('a');
    assert.equal(refused.ok, false);
    assert.equal(refused.scope, 'visitor');
    assert.ok(refused.retryAfter > 0 && refused.retryAfter <= 60);
    assert.deepEqual(await limiter.take('b'), { ok: true });
  });

  test(`${name} limiter enforces the site-wide daily total, unaffected by refused visitors`, async () => {
    const limiter = create(perMinute + 1);
    for (let question = 0; question < perMinute; question++) await limiter.take('a');
    for (let attempt = 0; attempt < 5; attempt++) assert.equal((await limiter.take('a')).scope, 'visitor');
    assert.deepEqual(await limiter.take('b'), { ok: true });
    const refused = await limiter.take('c');
    assert.equal(refused.scope, 'site');
    assert.ok(refused.retryAfter > 0 && refused.retryAfter <= 86_400);
  });
}

test('D1 limiter stores one row per visitor window and site day', async () => {
  const db = sqliteD1();
  const limiter = d1Limiter(db, 100);
  await limiter.take('a');
  await limiter.take('a');
  assert.equal(db.count(), VISITOR_LIMITS.length + 1);
});

test('request bodies are read only up to the limit', async () => {
  const post = (body, headers = {}) => new Request('https://docs.axiomcore.dev/api/chat', { method: 'POST', body, headers, duplex: 'half' });
  assert.equal(await readLimitedText(post('{"messages":[]}'), 64), '{"messages":[]}');
  assert.equal(await readLimitedText(post('x', { 'content-length': '99999' }), 64), null);
  const stream = new ReadableStream({
    pull(controller) {
      controller.enqueue(new Uint8Array(32));
    },
  });
  assert.equal(await readLimitedText(post(stream), 64), null);
});
