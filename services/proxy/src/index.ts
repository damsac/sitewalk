/**
 * Jefe LLM proxy — Phase 1.
 *
 * Holds the real Anthropic key server-side so it stops shipping inside every
 * installed build, meters spend per install, and refuses to keep spending once
 * a daily cap is hit.
 *
 * PRIVACY — the single most important rule in this file: request bodies carry
 * walk transcripts, which are customer job-site data. NOTHING in this worker
 * may log, store, or forward a request or response BODY. Only token counts,
 * model names, install ids, and status codes are ever recorded. Adding a
 * `console.log(body)` here for debugging would turn a privacy-preserving
 * product into a transcript warehouse.
 */

import { authenticate, type AuthFailure } from './auth';
import { costUsd, type Usage } from './pricing';

export interface Env {
  /** Secret. The real `sk-ant-…` key. Never leaves this worker. */
  ANTHROPIC_API_KEY: string;
  /** Secret. Shared with the iOS build — see auth.ts on what it does and doesn't buy. */
  APP_SECRET: string;
  /** KV namespace holding daily spend counters. */
  METER: KVNamespace;
  /** Whole-service daily ceiling in USD. */
  DAILY_SPEND_CAP_USD?: string;
  /** Per-install daily ceiling in USD. */
  PER_INSTALL_DAILY_CAP_USD?: string;
  /** Override for tests / staging. Defaults to Anthropic. */
  UPSTREAM_BASE_URL?: string;
}

const DEFAULT_DAILY_CAP_USD = 25;
const DEFAULT_PER_INSTALL_CAP_USD = 2;
const DEFAULT_UPSTREAM = 'https://api.anthropic.com';
const ANTHROPIC_VERSION = '2023-06-01';

/** Counters expire well after their day so a stale key can't pin spend forever. */
const COUNTER_TTL_SECONDS = 60 * 60 * 24 * 3;

/**
 * Error shape mirroring Anthropic's, because the Rust provider surfaces
 * failures as `HTTP {status}: {body}`. Matching the shape means a cap trip
 * reads as a clear sentence in the app's logs instead of an opaque blob.
 */
function errorResponse(status: number, type: string, message: string): Response {
  return new Response(JSON.stringify({ type: 'error', error: { type, message } }), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

const AUTH_FAILURE_MESSAGE: Record<AuthFailure, string> = {
  missing: 'no credential supplied',
  malformed: 'credential is not a well-formed Jefe install credential',
  bad_secret: 'credential rejected',
};

/** UTC day key. Deliberately not local time: the worker runs in every region. */
export function dayKey(now: Date): string {
  return now.toISOString().slice(0, 10);
}

function num(raw: string | undefined, fallback: number): number {
  const parsed = Number(raw);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

async function readSpend(kv: KVNamespace, key: string): Promise<number> {
  const raw = await kv.get(key);
  const parsed = Number(raw);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : 0;
}

/**
 * Adds to a counter.
 *
 * KV is eventually consistent and this is a read-modify-write, so concurrent
 * calls can lose an increment and the cap can overshoot slightly. That is an
 * accepted Phase 1 tradeoff: the cap is a blast-radius limiter, not a billing
 * ledger, and overshooting by one walk is harmless. Durable Objects give exact
 * counters and are the upgrade path if this ever needs to be precise.
 */
async function addSpend(kv: KVNamespace, key: string, delta: number): Promise<void> {
  const current = await readSpend(kv, key);
  await kv.put(key, String(current + delta), { expirationTtl: COUNTER_TTL_SECONDS });
}

export default {
  async fetch(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
    const url = new URL(request.url);

    // Unauthenticated liveness probe. Reveals nothing but "a worker is here".
    if (request.method === 'GET' && url.pathname === '/health') {
      return new Response(JSON.stringify({ ok: true }), {
        headers: { 'content-type': 'application/json' },
      });
    }

    if (request.method !== 'POST' || url.pathname !== '/v1/messages') {
      return errorResponse(404, 'not_found_error', 'unknown route');
    }

    if (!env.ANTHROPIC_API_KEY || !env.APP_SECRET) {
      // Misconfiguration must fail loudly and CLOSED. Forwarding without a key
      // would 401 confusingly; running without APP_SECRET would accept anyone.
      return errorResponse(500, 'api_error', 'proxy is not configured');
    }

    const auth = authenticate(request.headers, env.APP_SECRET);
    if (!auth.ok) {
      return errorResponse(401, 'authentication_error', AUTH_FAILURE_MESSAGE[auth.reason]);
    }
    const { installId } = auth.credential;

    const today = dayKey(new Date());
    const globalKey = `spend:global:${today}`;
    const installKey = `spend:install:${installId}:${today}`;

    const globalCap = num(env.DAILY_SPEND_CAP_USD, DEFAULT_DAILY_CAP_USD);
    const installCap = num(env.PER_INSTALL_DAILY_CAP_USD, DEFAULT_PER_INSTALL_CAP_USD);

    const [globalSpend, installSpend] = await Promise.all([
      readSpend(env.METER, globalKey),
      readSpend(env.METER, installKey),
    ]);

    // Caps are checked BEFORE forwarding. Checking after would mean the
    // request that breaches the cap is also the request that gets served.
    if (globalSpend >= globalCap) {
      console.warn(
        JSON.stringify({ event: 'global_cap_tripped', day: today, spend: globalSpend, cap: globalCap }),
      );
      return errorResponse(
        503,
        'overloaded_error',
        'service daily spend cap reached; try again tomorrow',
      );
    }
    if (installSpend >= installCap) {
      return errorResponse(
        429,
        'rate_limit_error',
        'this install reached its daily usage limit',
      );
    }

    // Read the body once, as text. We need `model` for pricing, and the body
    // must be forwarded byte-identically — re-serializing parsed JSON could
    // reorder keys and would break prompt-cache prefix matching upstream.
    const bodyText = await request.text();
    let model = 'unknown';
    try {
      const parsed = JSON.parse(bodyText) as { model?: unknown };
      if (typeof parsed.model === 'string') model = parsed.model;
    } catch {
      return errorResponse(400, 'invalid_request_error', 'body is not valid JSON');
    }

    let upstream: Response;
    try {
      upstream = await fetch(`${(env.UPSTREAM_BASE_URL ?? DEFAULT_UPSTREAM).replace(/\/+$/, '')}/v1/messages`, {
        method: 'POST',
        headers: {
          // The client's credential is dropped here and replaced with the real
          // key. This swap is the entire point of the service.
          'x-api-key': env.ANTHROPIC_API_KEY,
          'anthropic-version': ANTHROPIC_VERSION,
          'content-type': 'application/json',
        },
        body: bodyText,
      });
    } catch (e) {
      console.error(JSON.stringify({ event: 'upstream_unreachable', installId, model }));
      return errorResponse(502, 'api_error', 'upstream request failed');
    }

    const responseText = await upstream.text();

    if (upstream.ok) {
      // Meter from the response's OWN usage numbers rather than estimating.
      // Billing happens upstream whether or not we record it, so the counter
      // must follow reality, not our guess at it.
      try {
        const parsed = JSON.parse(responseText) as { usage?: Usage };
        const spend = costUsd(model, parsed.usage ?? {});
        if (spend > 0) {
          // waitUntil: metering must not add latency to a live walk, but it
          // also must not be dropped when the response returns first.
          ctx.waitUntil(
            Promise.all([
              addSpend(env.METER, globalKey, spend),
              addSpend(env.METER, installKey, spend),
            ]).then(() => undefined),
          );
        }
        // Token counts and model only — never body content. See the file header.
        console.log(
          JSON.stringify({
            event: 'call',
            installId,
            model,
            usd: Number(spend.toFixed(6)),
            input: parsed.usage?.input_tokens ?? 0,
            output: parsed.usage?.output_tokens ?? 0,
            cache_write: parsed.usage?.cache_creation_input_tokens ?? 0,
            cache_read: parsed.usage?.cache_read_input_tokens ?? 0,
          }),
        );
      } catch {
        // A success we can't price is worse than one we can: it spends real
        // money invisibly. Log it loudly; still return the response, since the
        // user's walk shouldn't fail over our accounting.
        console.error(JSON.stringify({ event: 'unpriceable_response', installId, model }));
      }
    } else {
      console.warn(
        JSON.stringify({ event: 'upstream_error', installId, model, status: upstream.status }),
      );
    }

    // Pass the upstream response through unchanged so the Rust provider parses
    // exactly what it would have parsed talking to Anthropic directly.
    return new Response(responseText, {
      status: upstream.status,
      headers: { 'content-type': 'application/json' },
    });
  },
} satisfies ExportedHandler<Env>;
