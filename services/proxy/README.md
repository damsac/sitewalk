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
The seam for it is `authenticate()` in `src/auth.ts` — Phase 2 replaces that
function's body and adds a key-id store. Nothing else in the worker changes.

Until then, **the daily spend cap is the thing protecting you.** Keep it low.

---

## What it does

1. Accepts `POST /v1/messages` only (plus an unauthenticated `GET /health`).
2. Identifies the caller from a `jefe.<installId>.<appSecret>` credential,
   which arrives in `x-api-key` / `Authorization` because the Rust provider
   sends `config.api_key` in both and offers no custom-header seam. Packing
   both values into that one field keeps this a config change, not an FFI one.
3. Refuses **before forwarding** if the global or per-install daily USD cap is
   already reached.
4. Swaps the caller's credential for the real key and forwards the body
   **byte-identically** — re-serializing could reorder JSON keys and break
   upstream prompt-cache prefix matching.
5. Meters actual spend from the response's own `usage`, pricing the three input
   classes separately (full / ~1.25x cache write / ~0.1x cache read).

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

Counters are KV read-modify-write, so concurrent calls can lose an increment
and the cap can overshoot slightly. That is accepted: this is a blast-radius
limiter, not a billing ledger. Durable Objects are the upgrade if it ever needs
to be exact.

---

## Wiring the app (next step, not yet done)

`EngineResolution.swift` currently reads `PPQ_API_KEY` from `Info.plist`. To
move onto the proxy, it needs to instead:

- set `EngineConfig.baseUrl` to the worker URL,
- set `EngineConfig.apiKey` to `jefe.<installId>.<appSecret>`, where
  `installId` is generated once and kept in the Keychain and `appSecret` comes
  from `Info.plist`.

**No core or FFI change is required** — `EngineConfig` already carries
`baseUrl`, and `AnthropicProvider` already has `with_base_url`.

Keep the existing direct-key path working when no proxy URL is configured, so
local real-core development against a personal key still works.

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

These are **not** wired into the repo's GitHub Actions CI, which is Rust +
iOS only. Run them locally before deploying.
