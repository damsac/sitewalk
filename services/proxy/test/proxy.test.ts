import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';

import { authenticate, readRawCredential } from '../src/auth';
import { costUsd, rateFor } from '../src/pricing';
import worker, { dayKey, type Env } from '../src/index';

const APP_SECRET = 'test-app-secret';
const GOOD = `jefe.install-abc-123.${APP_SECRET}`;

// ---------------------------------------------------------------- pricing

describe('pricing', () => {
  it('prices the three input classes at different rates', () => {
    // 1M of each class on Haiku ($1/$5 per MTok): full 1.00, write 1.25,
    // read 0.10, output 5.00.
    const usd = costUsd('claude-haiku-4-5', {
      input_tokens: 1_000_000,
      cache_creation_input_tokens: 1_000_000,
      cache_read_input_tokens: 1_000_000,
      output_tokens: 1_000_000,
    });
    expect(usd).toBeCloseTo(1 + 1.25 + 0.1 + 5, 6);
  });

  it('treats missing cache fields as zero rather than NaN', () => {
    // A non-caching response omits them. NaN here would poison the running
    // total and silently disable every later cap comparison.
    const usd = costUsd('claude-haiku-4-5', { input_tokens: 1_000_000, output_tokens: 0 });
    expect(usd).toBeCloseTo(1, 6);
  });

  it('resolves dated model snapshots by prefix', () => {
    expect(rateFor('claude-haiku-4-5-20251001')).toEqual(rateFor('claude-haiku-4-5'));
  });

  it('over-estimates an unknown model instead of under-estimating', () => {
    // Guessing low would let an unpriced model slip past the cap entirely.
    const unknown = rateFor('claude-something-unreleased');
    expect(unknown.input).toBeGreaterThanOrEqual(rateFor('claude-opus-5').input);
  });
});

// ------------------------------------------------------------------- auth

describe('auth', () => {
  it('accepts a well-formed credential and extracts the install id', () => {
    const res = authenticate(new Headers({ 'x-api-key': GOOD }), APP_SECRET);
    expect(res).toEqual({ ok: true, credential: { installId: 'install-abc-123' } });
  });

  it('reads the credential from either header the Rust provider sends', () => {
    expect(readRawCredential(new Headers({ 'x-api-key': GOOD }))).toBe(GOOD);
    expect(readRawCredential(new Headers({ authorization: `Bearer ${GOOD}` }))).toBe(GOOD);
  });

  it('rejects a wrong secret, a missing one, and a foreign key shape', () => {
    const cases: Array<[string, string]> = [
      ['bad secret', 'jefe.install-abc-123.wrong'],
      ['no secret', 'jefe.install-abc-123.'],
      ['raw anthropic key', 'sk-ant-whatever'],
      ['no install id', 'jefe..secret'],
    ];
    for (const [label, raw] of cases) {
      const res = authenticate(new Headers({ 'x-api-key': raw }), APP_SECRET);
      expect(res.ok, label).toBe(false);
    }
    expect(authenticate(new Headers(), APP_SECRET)).toEqual({ ok: false, reason: 'missing' });
  });

  it('rejects an oversized install id', () => {
    const huge = `jefe.${'a'.repeat(200)}.${APP_SECRET}`;
    expect(authenticate(new Headers({ 'x-api-key': huge }), APP_SECRET).ok).toBe(false);
  });
});

// ------------------------------------------------------------------ worker

/** Minimal in-memory KV good enough for the counter read/write path. */
function fakeKv(seed: Record<string, string> = {}) {
  const store = new Map<string, string>(Object.entries(seed));
  return {
    store,
    get: vi.fn(async (k: string) => store.get(k) ?? null),
    put: vi.fn(async (k: string, v: string) => {
      store.set(k, v);
    }),
  } as unknown as KVNamespace & { store: Map<string, string> };
}

function makeEnv(overrides: Partial<Env> = {}): Env {
  return {
    ANTHROPIC_API_KEY: 'sk-ant-real',
    APP_SECRET,
    METER: fakeKv(),
    UPSTREAM_BASE_URL: 'https://upstream.test',
    ...overrides,
  } as Env;
}

/** Collects waitUntil promises so metering can be awaited in tests. */
function makeCtx() {
  const pending: Promise<unknown>[] = [];
  return {
    ctx: {
      waitUntil: (p: Promise<unknown>) => {
        pending.push(p);
      },
      passThroughOnException: () => {},
    } as unknown as ExecutionContext,
    settle: () => Promise.all(pending),
  };
}

function messagesRequest(credential = GOOD, model = 'claude-haiku-4-5'): Request {
  return new Request('https://proxy.test/v1/messages', {
    method: 'POST',
    headers: { 'x-api-key': credential, 'content-type': 'application/json' },
    body: JSON.stringify({ model, max_tokens: 64, messages: [{ role: 'user', content: 'hi' }] }),
  });
}

const UPSTREAM_OK = {
  content: [{ type: 'text', text: 'ok' }],
  stop_reason: 'end_turn',
  usage: { input_tokens: 1_000_000, output_tokens: 0 }, // exactly $1.00 on Haiku
};

describe('worker', () => {
  beforeEach(() => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response(JSON.stringify(UPSTREAM_OK), { status: 200 })),
    );
  });
  afterEach(() => vi.unstubAllGlobals());

  it('swaps the install credential for the real key and never forwards the caller\'s', async () => {
    const env = makeEnv();
    const { ctx, settle } = makeCtx();
    const res = await worker.fetch(messagesRequest(), env, ctx);
    await settle();

    expect(res.status).toBe(200);
    const call = (globalThis.fetch as ReturnType<typeof vi.fn>).mock.calls[0]!;
    const init = call[1] as RequestInit;
    const headers = new Headers(init.headers);
    expect(headers.get('x-api-key')).toBe('sk-ant-real');
    expect(headers.get('x-api-key')).not.toContain(APP_SECRET);
    expect(headers.get('authorization')).toBeNull();
  });

  it('forwards the body byte-identically so upstream prefix caching still matches', async () => {
    const env = makeEnv();
    const { ctx, settle } = makeCtx();
    const req = messagesRequest();
    const sent = await req.clone().text();
    await worker.fetch(req, env, ctx);
    await settle();

    const call = (globalThis.fetch as ReturnType<typeof vi.fn>).mock.calls[0]!;
    const init = call[1] as RequestInit;
    expect(init.body).toBe(sent);
  });

  it('meters the call against both the global and per-install counters', async () => {
    const kv = fakeKv();
    const env = makeEnv({ METER: kv });
    const { ctx, settle } = makeCtx();
    await worker.fetch(messagesRequest(), env, ctx);
    await settle();

    const today = dayKey(new Date());
    expect(Number(kv.store.get(`spend:global:${today}`))).toBeCloseTo(1, 6);
    expect(Number(kv.store.get(`spend:install:install-abc-123:${today}`))).toBeCloseTo(1, 6);
  });

  it('refuses BEFORE forwarding once the global cap is reached', async () => {
    const today = dayKey(new Date());
    const kv = fakeKv({ [`spend:global:${today}`]: '999' });
    const env = makeEnv({ METER: kv, DAILY_SPEND_CAP_USD: '25' });
    const { ctx } = makeCtx();

    const res = await worker.fetch(messagesRequest(), env, ctx);
    expect(res.status).toBe(503);
    // The request that breaches the cap must not also be the one that gets served.
    expect(globalThis.fetch).not.toHaveBeenCalled();
  });

  it('rate-limits one noisy install without stopping everyone else', async () => {
    const today = dayKey(new Date());
    const kv = fakeKv({ [`spend:install:install-abc-123:${today}`]: '999' });
    const env = makeEnv({ METER: kv, PER_INSTALL_DAILY_CAP_USD: '2' });
    const { ctx } = makeCtx();

    const res = await worker.fetch(messagesRequest(), env, ctx);
    expect(res.status).toBe(429);
    expect(globalThis.fetch).not.toHaveBeenCalled();
  });

  it('rejects a bad credential without spending anything', async () => {
    const env = makeEnv();
    const { ctx } = makeCtx();
    const res = await worker.fetch(messagesRequest('jefe.someone.wrong-secret'), env, ctx);
    expect(res.status).toBe(401);
    expect(globalThis.fetch).not.toHaveBeenCalled();
  });

  it('fails closed when the worker is misconfigured', async () => {
    // No APP_SECRET would otherwise mean "accept everyone".
    const env = makeEnv({ APP_SECRET: '' });
    const { ctx } = makeCtx();
    const res = await worker.fetch(messagesRequest(), env, ctx);
    expect(res.status).toBe(500);
    expect(globalThis.fetch).not.toHaveBeenCalled();
  });

  it('passes an upstream error through with its status intact', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => new Response(JSON.stringify({ type: 'error' }), { status: 429 })),
    );
    const env = makeEnv();
    const { ctx, settle } = makeCtx();
    const res = await worker.fetch(messagesRequest(), env, ctx);
    await settle();
    expect(res.status).toBe(429);
  });

  it('serves health without a credential and 404s anything else', async () => {
    const env = makeEnv();
    const { ctx } = makeCtx();
    const health = await worker.fetch(new Request('https://proxy.test/health'), env, ctx);
    expect(health.status).toBe(200);

    const nope = await worker.fetch(new Request('https://proxy.test/v1/complete'), env, ctx);
    expect(nope.status).toBe(404);
  });
});
