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
import { ALLOWED_MODELS_FOR_MESSAGE, costUsd, isAllowedModel, type Usage } from './pricing';
import {
  AttestError,
  CHALLENGE_TTL_SECONDS,
  TOKEN_TTL_SECONDS,
  claimChallenge,
  fromBase64,
  isPlausibleKeyId,
  issueChallenge,
  loadAttestedKey,
  mintToken,
  saveAttestedKey,
  verifyAssertion,
  verifyAttestation,
  verifyToken,
} from './attest';

/**
 * How hard attestation is enforced.
 *
 * `off`     — the credential's token, if any, is ignored entirely.
 * `monitor` — tokens are verified and the outcome is logged, but a failure
 *             never blocks a request.
 * `enforce` — `/v1/messages` requires a valid token.
 *
 * Deployed as `off` and moved forward one notch at a time. Going straight to
 * `enforce` would brick every already-installed build the moment it lands,
 * including the testers' — attestation has to be observed working on real
 * devices before it is allowed to say no.
 */
export type AttestMode = 'off' | 'monitor' | 'enforce';

export interface Env {
  /** Secret. The real `sk-ant-…` key. Never leaves this worker. */
  ANTHROPIC_API_KEY: string;
  /** Secret. Shared with the iOS build — see auth.ts on what it does and doesn't buy. */
  APP_SECRET: string;
  /** KV namespace holding daily spend counters. */
  METER: KVNamespace;
  /** Whole-service daily ceiling in USD. `"0"` is the emergency kill switch. */
  DAILY_SPEND_CAP_USD?: string;
  /** Per-install daily ceiling in USD. `"0"` blocks every install. */
  PER_INSTALL_DAILY_CAP_USD?: string;
  /** Override for tests / staging. Defaults to Anthropic. */
  UPSTREAM_BASE_URL?: string;

  /** `off` | `monitor` | `enforce`. Anything unrecognised means `off`. */
  ATTEST_MODE?: string;
  /** `TEAMID.bundle.id`. Required once attestation is anything but `off`. */
  APP_ATTEST_APP_ID?: string;
  /** Apple's App Attest root CA, PEM. Public data, but configured not pinned. */
  APP_ATTEST_ROOT_CA?: string;
  /** `"true"` to also accept development attestations. Never in production. */
  APP_ATTEST_ALLOW_DEVELOPMENT?: string;
  /**
   * Secret. HMAC key for attestation tokens — and, unlike `APP_SECRET`, it is
   * NEVER shared with the app. That asymmetry is the whole point: a token
   * signed with a secret the client holds proves nothing the client could not
   * have claimed for itself. Required before `ATTEST_MODE` may leave `off`.
   */
  ATTEST_TOKEN_SECRET?: string;
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

/**
 * Parses a configured cap. An explicit `0` MEANS ZERO.
 *
 * Setting `DAILY_SPEND_CAP_USD = "0"` in wrangler.toml is the obvious move
 * during an incident, and it has to actually stop the service. The old
 * `parsed > 0` test silently turned that into the $25 default — a kill switch
 * that failed OPEN. Zero is a legitimate configured value, not garbage.
 *
 * Everything that is NOT a non-negative finite number still falls back:
 * unset, empty/whitespace (`Number('')` is 0, which would be an accidental
 * kill switch from a stray `= ""`), a typo, or a negative.
 */
function num(raw: string | undefined, fallback: number): number {
  if (raw === undefined || raw.trim() === '') return fallback;
  const parsed = Number(raw);
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : fallback;
}

/**
 * Reads a spend counter.
 *
 * The `> 0` test here is deliberately NOT the same bug as `num` above: this
 * function's fallback IS zero, so a stored "0" and a rejected "0" produce the
 * same answer. What the test buys is clamping a negative or corrupt value to
 * zero — and for a SPEND counter that is the fail-closed direction, since a
 * negative would manufacture headroom under the cap. Leave it alone.
 */
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

// ------------------------------------------------------------- attestation

export function attestMode(env: Env): AttestMode {
  const raw = env.ATTEST_MODE?.trim().toLowerCase();
  if (raw === 'monitor' || raw === 'enforce' || raw === 'off') return raw;
  // Anything else — unset, a typo, an empty string — is `off`. A misspelled
  // mode must not fail into rejecting every request in the field.
  //
  // But it must not be SILENT either. `off` is also what a correct deployment
  // looks like, so an operator who typed `enfroce` sees a service behaving
  // exactly as they configured it, forever. Truncated because it is echoed
  // into a log line; it is operator config, not caller input, so there is no
  // privacy question here — no request body is involved.
  if (raw !== undefined && raw !== '') {
    console.warn(
      JSON.stringify({ event: 'attest_mode_unrecognised', value: raw.slice(0, 32), using: 'off' }),
    );
  }
  return 'off';
}

interface AttestSetup {
  appId: string;
  rootCaPem: string;
  allowDevelopment: boolean;
  /** Server-only HMAC key. Never `APP_SECRET` — see below. */
  tokenSecret: string;
}

/**
 * The attestation config, or the name of the first thing wrong with it.
 *
 * One function rather than a boolean and a message, so the two can never
 * disagree about what "configured" means. The reason is carried because "the
 * proxy is not configured" is a fine thing to tell a caller and a useless thing
 * to tell the operator who has to fix it.
 *
 * Incomplete config in `enforce` mode is a hard 500 at the call site rather
 * than a silent downgrade: an operator who asked for enforcement and quietly
 * got none is strictly worse off than one whose deploy visibly fails.
 */
function attestSetup(env: Env): { ok: true; config: AttestSetup } | { ok: false; problem: string } {
  const appId = env.APP_ATTEST_APP_ID;
  const rootCaPem = env.APP_ATTEST_ROOT_CA;
  const tokenSecret = env.ATTEST_TOKEN_SECRET;
  if (!appId) return { ok: false, problem: 'missing_app_attest_app_id' };
  if (!rootCaPem) return { ok: false, problem: 'missing_app_attest_root_ca' };
  if (!tokenSecret) return { ok: false, problem: 'missing_attest_token_secret' };
  // Setting them equal is the exact failure this binding exists to prevent, and
  // it is one copy-paste away. Fail closed rather than quietly restoring a
  // token that anyone holding the IPA can mint.
  if (tokenSecret === env.APP_SECRET) {
    return { ok: false, problem: 'attest_token_secret_equals_app_secret' };
  }
  return {
    ok: true,
    config: {
      appId,
      rootCaPem,
      allowDevelopment: env.APP_ATTEST_ALLOW_DEVELOPMENT === 'true',
      tokenSecret,
    },
  };
}

function json(status: number, payload: unknown): Response {
  return new Response(JSON.stringify(payload), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

/** Bodies here are tiny control-plane JSON — never walk content. See the header. */
async function readJsonBody(request: Request): Promise<Record<string, unknown>> {
  const text = await request.text();
  if (text.length > 128 * 1024) throw new AttestError('body too large', 'too_large');
  try {
    const parsed = JSON.parse(text);
    if (!parsed || typeof parsed !== 'object') throw new Error('not an object');
    return parsed as Record<string, unknown>;
  } catch {
    throw new AttestError('body is not a JSON object', 'malformed_body');
  }
}

function stringField(body: Record<string, unknown>, name: string): string {
  const value = body[name];
  if (typeof value !== 'string' || value.length === 0) {
    throw new AttestError(`missing "${name}"`, 'missing_field');
  }
  return value;
}

async function handleAttestRoute(
  path: string,
  request: Request,
  env: Env,
  installId: string
): Promise<Response> {
  const setup = attestSetup(env);
  if (!setup.ok) {
    console.error(JSON.stringify({ event: 'attest_misconfigured', problem: setup.problem }));
    return errorResponse(503, 'api_error', 'attestation is not configured on this deployment');
  }
  const config = setup.config;

  try {
    if (path === '/v1/attest/challenge') {
      const challenge = await issueChallenge(env.METER, installId);
      return json(200, { challenge, expires_in: CHALLENGE_TTL_SECONDS });
    }

    const body = await readJsonBody(request);
    const keyId = stringField(body, 'key_id');
    const challenge = stringField(body, 'challenge');
    if (!isPlausibleKeyId(keyId)) {
      throw new AttestError('key id is not a base64 SHA-256', 'bad_key_id');
    }

    // Claimed BEFORE any expensive verification, so a flood of bogus
    // attestations can't be used to burn CPU on a challenge that was never
    // issued to this install.
    if (!(await claimChallenge(env.METER, challenge, installId))) {
      throw new AttestError('challenge is unknown, expired, or not yours', 'bad_challenge');
    }
    const challengeBytes = new TextEncoder().encode(challenge);

    if (path === '/v1/attest/attest') {
      const attestation = fromBase64(stringField(body, 'attestation'));
      const verified = await verifyAttestation(
        attestation,
        fromBase64(keyId),
        challengeBytes,
        config
      );
      await saveAttestedKey(env.METER, keyId, {
        spki: btoa(String.fromCharCode(...verified.spki)),
        curve: verified.curve,
        counter: 0,
        installId,
        attestedAt: Date.now(),
      });
      console.log(JSON.stringify({ event: 'attest_ok', installId, curve: verified.curve }));
      return json(200, { ok: true });
    }

    if (path === '/v1/attest/assert') {
      const stored = await loadAttestedKey(env.METER, keyId);
      if (!stored) throw new AttestError('key has not been attested', 'unknown_key');
      // A key belongs to the install that attested it. Without this, one
      // attested device could mint tokens for every install id it likes and
      // spread its spend across everyone else's caps.
      if (stored.installId !== installId) {
        throw new AttestError('key belongs to a different install', 'key_install_mismatch');
      }

      const assertion = fromBase64(stringField(body, 'assertion'));
      const { counter } = await verifyAssertion(
        assertion,
        challengeBytes,
        { spki: fromBase64(stored.spki), curve: stored.curve, counter: stored.counter },
        config.appId
      );
      await saveAttestedKey(env.METER, keyId, { ...stored, counter });

      // Signed with the server-only secret, NOT `APP_SECRET`. A token the
      // client could have minted for itself certifies nothing.
      const token = await mintToken(config.tokenSecret, installId, keyId);
      return json(200, { token, expires_in: TOKEN_TTL_SECONDS });
    }

    return errorResponse(404, 'not_found_error', 'unknown route');
  } catch (error) {
    if (error instanceof AttestError) {
      // The code is a fixed enum, so this leaks nothing about the caller.
      console.warn(JSON.stringify({ event: 'attest_failed', installId, code: error.code }));
      return json(403, {
        type: 'error',
        error: { type: 'permission_error', message: error.message, code: error.code },
      });
    }
    console.error(JSON.stringify({ event: 'attest_error', installId }));
    return errorResponse(500, 'api_error', 'attestation check failed');
  }
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

    const isAttestRoute = url.pathname.startsWith('/v1/attest/');
    if (request.method !== 'POST' || (url.pathname !== '/v1/messages' && !isAttestRoute)) {
      return errorResponse(404, 'not_found_error', 'unknown route');
    }

    // The attestation routes never call upstream, so they need the app secret
    // but not the Anthropic key. Requiring both here would make attestation
    // undeployable to a staging worker that has no key.
    if (!env.APP_SECRET || (!isAttestRoute && !env.ANTHROPIC_API_KEY)) {
      // Misconfiguration must fail loudly and CLOSED. Forwarding without a key
      // would 401 confusingly; running without APP_SECRET would accept anyone.
      return errorResponse(500, 'api_error', 'proxy is not configured');
    }

    const auth = authenticate(request.headers, env.APP_SECRET);
    if (!auth.ok) {
      return errorResponse(401, 'authentication_error', AUTH_FAILURE_MESSAGE[auth.reason]);
    }
    const { installId, attestToken } = auth.credential;

    // Enrolment happens under Phase 1 auth alone — it has to, since a device
    // cannot present an attestation token before it has attested.
    if (isAttestRoute) {
      return handleAttestRoute(url.pathname, request, env, installId);
    }

    const mode = attestMode(env);
    if (mode !== 'off') {
      const setup = attestSetup(env);
      if (!setup.ok) {
        // Asked to enforce, unable to. Fail closed and loudly rather than
        // serving traffic an operator believes is being checked.
        console.error(
          JSON.stringify({ event: 'attest_misconfigured', mode, problem: setup.problem }),
        );
        return errorResponse(500, 'api_error', 'proxy is not configured');
      }
      // `env.METER` is passed because verifying a token means looking up the
      // attested key it names — the check that makes the token mean anything.
      const result = attestToken
        ? await verifyToken(env.METER, setup.config.tokenSecret, attestToken, installId)
        : ({ ok: false, reason: 'malformed' } as const);

      if (!result.ok) {
        console.warn(
          JSON.stringify({ event: 'attest_token_rejected', installId, mode, reason: result.reason }),
        );
        if (mode === 'enforce') {
          // 401 rather than 403: the app's recovery is to re-attest and retry,
          // which is what a client does with an authentication failure.
          return errorResponse(
            401,
            'authentication_error',
            'device attestation required; re-attest and retry',
          );
        }
        // `monitor` — observed, counted, allowed through. This is the mode
        // that tells us whether enforcement would have been safe.
      }
    }

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

    // Read the body once, as text. We inspect `model` and `stream`, but the
    // body must be forwarded byte-identically — re-serializing parsed JSON
    // could reorder keys and would break prompt-cache prefix matching upstream.
    // So: parse to VALIDATE, forward the original text.
    //
    // BODY WORK STAYS BEHIND AUTH — deliberate, and load-bearing for the two
    // validations below. Moving them ahead of `authenticate` would let a
    // post-deploy smoke check probe them without a credential, which was
    // proposed; it was declined because the same proof is available by giving
    // the probe a credential (CI already holds JEFE_APP_SECRET, which must
    // equal APP_SECRET or every shipped build would 401). The costs of moving
    // them are real: `/v1/messages` has no body size cap, so an unauthenticated
    // caller could force text()+JSON.parse at will where today it costs one
    // header compare; the model rejection below names the allowlist, which is
    // not something to hand an unauthenticated scanner; and every other body
    // read in this file (readJsonBody, attest routes) is already post-auth and
    // size-capped. Keep new body-dependent checks below this line.
    const bodyText = await request.text();
    let model = 'unknown';
    let stream: unknown;
    try {
      const parsed = JSON.parse(bodyText) as { model?: unknown; stream?: unknown };
      if (typeof parsed.model === 'string') model = parsed.model;
      stream = parsed.stream;
    } catch {
      return errorResponse(400, 'invalid_request_error', 'body is not valid JSON');
    }

    // Streaming is a complete metering bypass, so it is refused outright.
    // An SSE response comes back 200 with a body that is not JSON; the pricing
    // path below would throw, log `unpriceable_response`, and return the
    // response having called `addSpend` exactly zero times. Both daily counters
    // would stay at 0 forever and every subsequent request would sail past the
    // cap check. Nothing in crates/harness/src/providers/ ever sets `stream`
    // (AnthropicProvider::complete builds model/max_tokens/system/messages/
    // tools, plus optional tool_choice and cache_control), so refusing it costs
    // the app nothing. Metering streamed responses means parsing the SSE
    // `message_delta` usage frame — a real feature, not a patch.
    if (stream === true) {
      return errorResponse(
        400,
        'invalid_request_error',
        'streaming responses are not supported by this proxy; omit "stream" or set it to false',
      );
    }

    // The body is forwarded verbatim, so `model` is attacker-chosen. Anything
    // the app does not actually send is refused rather than metered against a
    // rate we may have guessed wrong. See pricing.ts for how the list was
    // derived and why this is an allowlist and not a larger fallback number.
    if (!isAllowedModel(model)) {
      // Truncated: `model` is attacker-supplied and otherwise unbounded, and it
      // goes into both a log line and a response body.
      const shown = model.slice(0, 64);
      console.warn(JSON.stringify({ event: 'model_rejected', installId, model: shown }));
      return errorResponse(
        400,
        'invalid_request_error',
        `model "${shown}" is not permitted by this proxy; allowed: ${ALLOWED_MODELS_FOR_MESSAGE}`,
      );
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
