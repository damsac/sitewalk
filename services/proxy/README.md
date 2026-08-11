# Jefe LLM proxy

A Cloudflare Worker that sits between the iOS app and the Anthropic API so the
real `sk-ant-…` key stops shipping inside every installed build.

**Why it exists:** `EngineResolution.swift` reads the key from `Info.plist`,
which CI bakes in at build time. Anyone who downloads the IPA can extract it and
spend against the account, and there is no way to rate-limit or revoke one user.
That is survivable for a handful of invited TestFlight testers and is not
survivable on a public App Store listing.

Design rationale lives in `docs/design/2026-07-25-app-store-v1-tiers-and-template-upload.md` §4.5.

---

## ⚠️ Phase 2 — App Attest — is REQUIRED before the public listing

**This is Phase 1. It is not finished work, and shipping it publicly as-is
leaves a real hole.**

Phase 1 authenticates callers with a shared secret embedded in the app binary
(`src/auth.ts`). That secret is extractable by anyone willing to unzip an IPA.
What it buys is real but bounded:

- the Anthropic key is off every device — the actual launch blocker,
- spend is metered per install, attributable, and revocable,
- worst-case loss is bounded by the daily cap rather than by an attacker's
  appetite.

What it does **not** buy is proof that the caller is a genuine, unmodified copy
of the app on real Apple hardware. An open LLM proxy is an actively-scanned-for
target, and the day the App Store listing goes public is the day the binary
becomes downloadable by anyone rather than by testers you invited.

**Phase 2 is [App Attest](https://developer.apple.com/documentation/devicecheck/establishing-your-app-s-integrity):**
the app asks the Secure Enclave for a hardware-backed key plus an attestation
blob; the server validates that blob against Apple's certificate chain and
pins the key id; every later request carries an assertion signed by that key.

### Status: server half done, device half not

`src/attest.ts` implements the verification, and it is wired in behind
`ATTEST_MODE`, which ships as `off`. **Nothing about the running service has
changed yet.** What is missing is the iOS side: `DCAppAttestService` key
generation, the enrolment calls, token caching, and the credential change.

Until the device half lands and has been soaked in `monitor`, **the daily spend
cap is still the thing protecting you.** Keep it low.

### The exchange

```
1.  app → POST /v1/attest/challenge        (Phase 1 credential)
    srv → { challenge, expires_in }        one-time, 5 minutes

2.  app   generates a Secure Enclave key, attests it over the challenge
    app → POST /v1/attest/attest           { key_id, challenge, attestation }
    srv   verifies the chain to Apple's root, stores the public key

3.  app → POST /v1/attest/assert           { key_id, challenge, assertion }
    srv → { token, expires_in }            1 hour

4.  app → POST /v1/messages                credential carries the token
```

Steps 1–3 run once an hour, not once per message — asserting per request would
double the round trips on the cellular link this app actually runs on.

### The credential change

`/v1/messages` has nowhere to put a token except the credential field itself,
because the Rust provider sends `config.api_key` as the only caller-controlled
header. So Phase 2 adds a second wire format alongside the first:

```
jefe.<installId>.<appSecret>                     # Phase 1, still accepted
jefeA.<installId>.<attestToken>~<appSecret>      # Phase 2
```

The app secret is still checked on attested credentials. Attestation adds a
layer; it does not remove the one underneath.

### Rolling it out

Move one notch at a time, and never skip `monitor`:

| `ATTEST_MODE` | Behaviour |
|---|---|
| `off` (default) | Tokens ignored entirely. |
| `monitor` | Tokens verified and logged; failures never block a request. |
| `enforce` | `/v1/messages` requires a valid token. |

`monitor` is what tells you whether `enforce` is safe. Watch for
`attest_token_rejected` in the logs; the count should fall to roughly zero as
builds carrying the device half roll out. Going straight to `enforce` bricks
every already-installed build the moment it lands, testers included.

Anything unrecognised in `ATTEST_MODE` means `off`, so a typo cannot lock the
fleet out. But `monitor`/`enforce` with missing config is a hard 500 — asking
for enforcement and quietly getting none is worse than a visibly failed deploy.

### What is deliberately not implemented

The attestation `receipt` is parsed but never sent to Apple for risk metrics or
key-refresh checks. That is a separate Apple endpoint with its own auth, it is
optional, and half-shipping it would be worse than not shipping it.

---

## What it does

1. Accepts `POST /v1/messages` only (plus an unauthenticated `GET /health`).
2. Identifies the caller from a `jefe.<installId>.<appSecret>` credential,
   which arrives in `x-api-key` / `Authorization` because the Rust provider
   sends `config.api_key` in both and offers no custom-header seam. Packing
   both values into that one field keeps this a config change, not an FFI one.
3. Refuses **before forwarding** if the global or per-install daily USD cap is
   already reached.
4. Validates the body: rejects `stream: true` and any model outside the
   allowlist, both with a 400. See **Request validation** below.
5. Swaps the caller's credential for the real key and forwards the body
   **byte-identically** — re-serializing could reorder JSON keys and break
   upstream prompt-cache prefix matching. The body is parsed only to *validate*;
   the original text is what gets forwarded.
6. Meters actual spend from the response's own `usage`, pricing the three input
   classes separately (full / ~1.25x cache write / ~0.1x cache read).

### Request validation

Two checks, both of which exist because everything that reaches metering has to
be meterable, and the body is attacker-controlled.

**`stream: true` is refused.** A streamed response is a 200 whose body is SSE,
not JSON. The metering path would fail to parse it, log `unpriceable_response`,
and return the response having called `addSpend` zero times — so both daily
counters would stay at 0 forever and every later request would pass the cap
check. That is a complete, silent metering bypass. Nothing in
`crates/harness/src/providers/` ever sets `stream` (`AnthropicProvider::complete`
builds `model` / `max_tokens` / `system` / `messages` / `tools`, plus optional
`tool_choice` and `cache_control`), so refusing it costs the app nothing.
Metering streams means parsing the SSE `message_delta` usage frame — a feature,
not a patch.

**Models are allowlisted**, matched by prefix (`pricing.ts`):

| Allowed prefix | Sent by |
|---|---|
| `claude-haiku-4-5` | `modelLive`, `modelReflection` |
| `claude-sonnet-4-5` | `modelProcessing` |

That is the complete set the shipped app can send — the three strings are
hardcoded in `EngineResolution.swift` and flow through `build_providers`
(`crates/ffi/src/engine.rs`) into the request body verbatim, with no launch arg
or env override on that path. Dated snapshots (`claude-haiku-4-5-20251001`) pass,
since a snapshot bills at its family's rate.

An allowlist rather than a bigger fallback number: the pricing table cannot be
kept exhaustive, and being wrong about an unlisted model is a *silent
under-meter*. `UNKNOWN_MODEL_RATE` remains as defense in depth, priced above
every model Anthropic currently sells so the "unknown must overestimate"
invariant actually holds.

### Privacy rule

Request bodies carry walk transcripts — customer job-site data. **The worker
never logs, stores, or forwards a request or response body.** Only token
counts, model names, install ids, and status codes are recorded. Adding a
`console.log(body)` for debugging would turn a privacy-preserving product into
a transcript warehouse. This is asserted by the file header in `src/index.ts`.

---

## Deploy

```sh
cd services/proxy
npm install

# 1. Create the KV namespace for spend counters, then paste the printed id
#    into wrangler.toml (replacing REPLACE_ME).
npx wrangler kv namespace create METER

# 2. Set the two secrets. Neither is committed.
npx wrangler secret put ANTHROPIC_API_KEY   # the real sk-ant-… key
npx wrangler secret put APP_SECRET          # a fresh random string; also goes in the iOS build

# 3. Ship it.
npx wrangler deploy
```

Verify:

```sh
curl https://<your-worker>.workers.dev/health
```

### Caps

Both live in `wrangler.toml` `[vars]`, so tuning them is a config deploy rather
than a code change:

| Var | Default | What it bounds |
|---|---|---|
| `DAILY_SPEND_CAP_USD` | `25` | Whole-service daily loss |
| `PER_INSTALL_DAILY_CAP_USD` | `2` | One noisy or hostile install |

Start them low. At the modeled ~$0.15/walk, $2/install/day is ~13 walks — far
above real use, far below an abuser's ambitions.

#### Emergency kill switch

Set `DAILY_SPEND_CAP_USD = "0"` in `wrangler.toml` and `npx wrangler deploy`.
Zero is honored **literally**: the global cap trips on the very next request and
the worker stops forwarding. Anything that is *not* a non-negative number —
unset, empty, a typo, a negative — falls back to the built-in default instead,
so `"0"` is the only spelling of the kill switch. (An empty string is
deliberately treated as garbage rather than as zero: `Number('')` is `0`, and a
stray `= ""` must not silently take the service down.)

Counters are KV read-modify-write, so concurrent calls can lose an increment
and the cap can overshoot slightly. That is accepted: this is a blast-radius
limiter, not a billing ledger. Durable Objects are the upgrade if it ever needs
to be exact.

---

## Wiring the app (done)

`EngineResolution.swift` → `resolveCredential()`:

- sets `EngineConfig.baseUrl` to `JEFE_PROXY_URL` from `Info.plist`,
- sets `EngineConfig.apiKey` to `jefe.<installId>.<appSecret>`, where
  `installId` is generated once and kept in the Keychain and `appSecret` is
  `JEFE_APP_SECRET` from `Info.plist`.

**No core or FFI change was required** — `EngineConfig` already carries
`baseUrl`, and `AnthropicProvider` already has `with_base_url`.

The proxy is the **only** path a shipped build has. `Info.plist` carries no
Anthropic key field at all, so there is nothing to extract from a downloaded
IPA. Local real-core development against a personal key still works, but only
through the `ANTHROPIC_API_KEY` **environment variable** — an env var cannot
ride along in a distributed build. With neither a proxy nor a dev key, the app
degrades to the scripted demo engine.

---

## Tests

```sh
npm test        # vitest, no network, no miniflare needed
npm run typecheck
```

Covers pricing (including the deliberate over-estimate for unknown models),
credential parsing and rejection, the credential-for-key swap, byte-identical
body forwarding, both caps refusing *before* upstream is called, fail-closed on
misconfiguration, and upstream error pass-through.

The attestation tests build a **real certificate chain with real ECDSA keys**
(`test/appattest-fixtures.ts`) rather than replaying a captured attestation.
That is the point: holding the private keys means a test can change exactly one
thing — the challenge, the counter, the aaguid, the signing CA — and assert
that *that specific check* is what rejects it. A recorded blob can only ever
prove one happy path, and every negative test against it degenerates into
"garbage in, error out", which a verifier that rejects everything would also
pass.

What the synthetic chain cannot prove is that we accept **Apple's** real root.
That is the residual risk, and the reason the rollout starts in `monitor`.

These are **not** wired into the repo's GitHub Actions CI, which is Rust +
iOS only. Run them locally before deploying.
