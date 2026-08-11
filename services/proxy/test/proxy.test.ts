import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';

import { authenticate, readRawCredential } from '../src/auth';
import { costUsd, isAllowedModel, rateFor } from '../src/pricing';
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

  it('prices an unknown model above every model Anthropic actually sells', () => {
    // The regression this pins: the fallback used to be $5/$25, but Claude
    // Fable 5 is $10/$50 and matches no prefix in RATES — so it metered at
    // HALF its real cost. "Unknown must overestimate" has to hold against the
    // real price list, not just against the table's own cheapest guesses.
    const unknown = rateFor('claude-fable-5');
    expect(unknown.input).toBeGreaterThan(10);
    expect(unknown.output).toBeGreaterThan(50);
  });

  it('keeps the fallback strictly above every priced entry', () => {
    const unknown = rateFor('definitely-not-a-model');
    for (const known of ['claude-haiku-4-5', 'claude-sonnet-4-5', 'claude-opus-5']) {
      expect(unknown.input, known).toBeGreaterThan(rateFor(known).input);
      expect(unknown.output, known).toBeGreaterThan(rateFor(known).output);
    }
  });
});

// -------------------------------------------------------------- allowlist

describe('model allowlist', () => {
  it('admits exactly the models the shipped app sends', () => {
    // EngineResolution.swift: modelLive/modelReflection = haiku-4-5,
    // modelProcessing = sonnet-4-5. Nothing else is on that path.
    expect(isAllowedModel('claude-haiku-4-5')).toBe(true);
    expect(isAllowedModel('claude-sonnet-4-5')).toBe(true);
  });

  it('admits a dated snapshot of an allowed family', () => {
    // The harness tests use this exact id; a snapshot bills at family rates.
    expect(isAllowedModel('claude-haiku-4-5-20251001')).toBe(true);
  });

  it('refuses priced-but-unsent models and anything unrecognised', () => {
    for (const model of [
      'claude-opus-5', // in RATES, but the app never sends it
      'claude-sonnet-4-6',
      'claude-fable-5', // the under-metering case
      'claude-mythos-5',
      '',
      'unknown',
      'claude-haiku', // prefix of the prefix — must not squeak through
    ]) {
      expect(isAllowedModel(model), model).toBe(false);
    }
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

/** Same route/credential, but an arbitrary body — for the validation tests. */
function rawMessagesRequest(body: Record<string, unknown>): Request {
  return new Request('https://proxy.test/v1/messages', {
    method: 'POST',
    headers: { 'x-api-key': GOOD, 'content-type': 'application/json' },
    body: JSON.stringify(body),
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

  // ------------------------------------------------- stream: the metering hole

  it('refuses stream:true rather than forwarding an unmeterable request', async () => {
    // An SSE response is a 200 whose body is not JSON. The pricing path would
    // throw, log `unpriceable_response`, and return it having called addSpend
    // ZERO times — so both daily counters stay at 0 forever and every later
    // request sails past the cap check. Refuse instead of forwarding.
    const kv = fakeKv();
    const env = makeEnv({ METER: kv });
    const { ctx, settle } = makeCtx();

    const res = await worker.fetch(
      rawMessagesRequest({
        model: 'claude-haiku-4-5',
        max_tokens: 64,
        stream: true,
        messages: [{ role: 'user', content: 'hi' }],
      }),
      env,
      ctx,
    );
    await settle();

    expect(res.status).toBe(400);
    expect(await res.json()).toMatchObject({
      type: 'error',
      error: { type: 'invalid_request_error' },
    });
    // Nothing forwarded, nothing spent, no counter written.
    expect(globalThis.fetch).not.toHaveBeenCalled();
    expect(kv.store.size).toBe(0);
  });

  it('checks the credential BEFORE it looks at the body', async () => {
    // Placement is deliberate: no body work happens for an unauthenticated
    // caller, matching every other body read in this file. The consequence
    // matters to whoever writes the post-deploy smoke check — an UNAUTHENTICATED
    // stream:true probe gets 401, not 400, so it cannot prove the guard is
    // live. Give the probe `jefe.<anything>.$JEFE_APP_SECRET` and it will see
    // the 400. If this test ever starts failing with 400, someone moved the
    // validation ahead of auth; read the comment above the body read first.
    const env = makeEnv();
    const { ctx } = makeCtx();
    const res = await worker.fetch(
      new Request('https://proxy.test/v1/messages', {
        method: 'POST',
        headers: { 'x-api-key': 'jefe.someone.wrong-secret', 'content-type': 'application/json' },
        body: JSON.stringify({ model: 'claude-fable-5', stream: true, messages: [] }),
      }),
      env,
      ctx,
    );
    expect(res.status).toBe(401);
    expect(globalThis.fetch).not.toHaveBeenCalled();
  });

  it('rejects an authenticated stream probe cheaply enough for a smoke check', async () => {
    // What the post-deploy smoke check will actually do. Asserts the probe is
    // free: nothing forwarded, no counter written, so it can run on every
    // deploy without touching spend.
    const kv = fakeKv();
    const env = makeEnv({ METER: kv });
    const { ctx, settle } = makeCtx();
    const res = await worker.fetch(
      new Request('https://proxy.test/v1/messages', {
        method: 'POST',
        headers: { 'x-api-key': `jefe.ci-smoke.${APP_SECRET}`, 'content-type': 'application/json' },
        body: JSON.stringify({ model: 'claude-haiku-4-5', stream: true, messages: [] }),
      }),
      env,
      ctx,
    );
    await settle();
    expect(res.status).toBe(400);
    expect(globalThis.fetch).not.toHaveBeenCalled();
    expect(kv.store.size).toBe(0);
  });

  it('still forwards when stream is absent or explicitly false', async () => {
    const env = makeEnv();
    const { ctx, settle } = makeCtx();

    const explicit = await worker.fetch(
      rawMessagesRequest({
        model: 'claude-haiku-4-5',
        max_tokens: 64,
        stream: false,
        messages: [{ role: 'user', content: 'hi' }],
      }),
      env,
      ctx,
    );
    await settle();
    expect(explicit.status).toBe(200);

    const absent = await worker.fetch(messagesRequest(), env, ctx);
    await settle();
    expect(absent.status).toBe(200);
    expect(globalThis.fetch).toHaveBeenCalledTimes(2);
  });

  // ---------------------------------------------------------- model allowlist

  it('refuses a model the app never sends, before spending anything', async () => {
    // The body is forwarded verbatim, so `model` is attacker-chosen. Fable 5 is
    // the concrete under-metering case: $10/$50 per MTok, matching no prefix in
    // RATES, so it used to meter at the fallback's rate instead of its own.
    const kv = fakeKv();
    const env = makeEnv({ METER: kv });
    const { ctx, settle } = makeCtx();

    const res = await worker.fetch(messagesRequest(GOOD, 'claude-fable-5'), env, ctx);
    await settle();

    expect(res.status).toBe(400);
    const body = (await res.json()) as { error: { message: string } };
    expect(body.error.message).toContain('claude-fable-5');
    expect(globalThis.fetch).not.toHaveBeenCalled();
    expect(kv.store.size).toBe(0);
  });

  it('truncates an oversized model string instead of echoing it back whole', async () => {
    const env = makeEnv();
    const { ctx } = makeCtx();
    const res = await worker.fetch(messagesRequest(GOOD, 'x'.repeat(5000)), env, ctx);
    expect(res.status).toBe(400);
    const body = (await res.json()) as { error: { message: string } };
    expect(body.error.message.length).toBeLessThan(300);
    expect(globalThis.fetch).not.toHaveBeenCalled();
  });

  it('forwards a dated snapshot of an allowed model', async () => {
    const env = makeEnv();
    const { ctx, settle } = makeCtx();
    const res = await worker.fetch(messagesRequest(GOOD, 'claude-haiku-4-5-20251001'), env, ctx);
    await settle();
    expect(res.status).toBe(200);
    expect(globalThis.fetch).toHaveBeenCalledTimes(1);
  });

  // --------------------------------------------------------- the kill switch

  it('honors an explicit zero global cap as a kill switch', async () => {
    // The obvious incident move. `parsed > 0` used to turn this into the $25
    // default — a kill switch that failed OPEN.
    const env = makeEnv({ DAILY_SPEND_CAP_USD: '0' });
    const { ctx } = makeCtx();
    const res = await worker.fetch(messagesRequest(), env, ctx);
    expect(res.status).toBe(503);
    expect(globalThis.fetch).not.toHaveBeenCalled();
  });

  it('honors an explicit zero per-install cap', async () => {
    const env = makeEnv({ PER_INSTALL_DAILY_CAP_USD: '0' });
    const { ctx } = makeCtx();
    const res = await worker.fetch(messagesRequest(), env, ctx);
    expect(res.status).toBe(429);
    expect(globalThis.fetch).not.toHaveBeenCalled();
  });

  it('falls back to the default cap for genuine garbage, not to zero', async () => {
    // A stray `= ""` or a typo must NOT become an accidental kill switch
    // (`Number('')` is 0), and must not disable the cap either.
    for (const raw of ['', '   ', 'twenty-five', '-5', 'NaN']) {
      vi.mocked(globalThis.fetch as ReturnType<typeof vi.fn>).mockClear();
      const env = makeEnv({ DAILY_SPEND_CAP_USD: raw });
      const { ctx, settle } = makeCtx();
      const res = await worker.fetch(messagesRequest(), env, ctx);
      await settle();
      // Default is $25 and the counter is empty, so the call goes through.
      expect(res.status, raw).toBe(200);
    }
  });

  it('still trips the default cap when garbage is configured and spend is high', async () => {
    const today = dayKey(new Date());
    const kv = fakeKv({ [`spend:global:${today}`]: '30' });
    const env = makeEnv({ METER: kv, DAILY_SPEND_CAP_USD: 'twenty-five' });
    const { ctx } = makeCtx();
    const res = await worker.fetch(messagesRequest(), env, ctx);
    expect(res.status).toBe(503);
    expect(globalThis.fetch).not.toHaveBeenCalled();
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
