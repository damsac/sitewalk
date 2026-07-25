# App Store v1 — tiers, payments, and "upload your own template"

**Owner:** sac · **Needs dam's read on:** §4.1 (vision content blocks) and §7 · **Supersedes the tiering in** `2026-07-12-customizable-paperwork.md` §4 · **Builds on** `2026-07-16-paperwork-structure-v2-plan.md` (the `DocumentSchema` seam, shipped in #244).

Three things ship together for the public App Store launch, because they are one project: **upload your own document template** (the wedge), **a paid tier** (the business), and **a key proxy** (the thing that makes both legal and safe). Isaac's calls: **$12.99/mo**, **minimal proxy, no accounts**, **all three in v1**.

---

## 1. Thinking

The v2 plan (#234) called upload "v3, dam-led, Premium." Two things have changed since, and both point the same way: **do it now, and don't gate it behind a third tier.**

**The cost premise is gone.** That doc priced upload as Premium because document comprehension "burns tokens per uploaded template." At today's pricing it is one vision call over one page — cents, once, per template, forever. There is no unit-economics argument for a Premium tier, and putting the wedge feature in tier three means most trial users never reach the thing that makes them pay.

**The seam already landed.** dam shipped `DocumentSchema` in #244: schema storage, CRUD FFI, core-minted numbering, and a fill pass that runs one focused LLM call over exactly the fields marked `fill=walk`. That means **"a custom document type the walk fills in" is a solved path in the core today.** Upload does not need new fill machinery. It needs to *produce a `DocumentSchema`* — and then it rejoins a road that is already paved.

The load-bearing invariant is unchanged and is what keeps this from shipping wrong paperwork: **the LLM only ever fills a named schema.** A raw PDF has no schema. So the feature is not "point the model at a stranger's document every walk" — it is *upload → infer once → the operator confirms → every future walk fills the confirmed schema*. The expensive, fallible comprehension step happens once at setup, under human review, and never again on the hot path.

**Fidelity decision (Isaac, 07-25): their structure, our renderer.** We infer the sections and fields from their upload and render through Jefe's existing PDF pipeline with their Letterhead Studio branding — we do not rasterize their page and overlay values onto it. Overlay looks more impressive in a demo and breaks in the field: a site walk produces N line items, and their template has room for a fixed number of rows. Overflow, page 2, and any edit to their template all break a pixel-mapped overlay. Structure + branding survives all three. Pixel-exact overlay stays open as a later, separate track for genuinely fixed-field forms (permits, HOA addenda, inspection checklists) where it actually fits.

---

## 2. Unit economics (measured, not assumed)

Pricing today: **Haiku 4.5 $1/$5 per MTok**, Sonnet-tier $3/$15. STT is on-device whisper — **zero marginal cost**, which is the whole local-first advantage and the reason these margins work at all.

**A walk is not a uniform unit of cost.** Processing re-sends the whole assembled transcript on *every* agent turn, so spend scales with transcript length × turns taken. The governing constants are `transcript_budget_tokens: 12_000` and `max_turns: 16` (`pipeline/mod.rs:108-120`) and, on the live side, an 8000-char window with `max_turns: 8` (`pipeline/live.rs:100-105`).

Modeled from those budgets (Sonnet 4.5 + Haiku 4.5, pre-lever):

| Per walk | Short (~10 min) | Long (~40 min, at the 12k budget) |
|---|---|---|
| Live extraction (Haiku) | ~$0.01 | ~$0.06 |
| Processing extraction (Sonnet, multi-turn) | ~$0.09 | ~$0.30 |
| `write_notes` summary | ~$0.02 | ~$0.05 |
| `build_document` | ~$0.02 | ~$0.02 |
| **Total** | **~$0.14** | **~$0.43** |

A long walk costs roughly **3x** a short one. One real safety valve: `transcript_budget_tokens` caps the transcript, so a two-hour walk is not 4x a thirty-minute one — per-walk cost has a ceiling near $0.50.

These are modeled, not measured. The meter that settles it shipped with migration v8 — `usage_totals_detailed()` returns the four token classes separately, and per-phase spend is one query:

```sql
SELECT purpose,
       SUM(input_tokens), SUM(cache_creation_input_tokens),
       SUM(cache_read_input_tokens), SUM(output_tokens), COUNT(*)
FROM llm_usage GROUP BY purpose;
```

**At $12.99 with Apple's Small Business Program (15%), net is $11.04.** At a post-lever blended ~$0.15/walk, break-even is **~74 walks/month**: a typical contractor at 20–40 walks costs $3–6 and clears **45–73% margin**. A heavy user at 60 walks is thin but positive. **A heavy user who also takes long walks (60 x $0.25 ≈ $15) is underwater.**

So the price holds for typical use, but the exposure is **walk length, not walk count** — 40 long walks cost more than 70 short ones. Cost reduction is not an optimization here; it is what keeps the headroom.

### 2.1 Where caching actually pays (corrected 07-25 after reading the loop)

An earlier draft of this doc claimed live extraction re-sends a growing transcript and is the prime cache target. **That is wrong, and worth recording so nobody re-derives it.** `LiveExtractor::maybe_extract` (`live.rs:139-149`) sends a **sliding cursor window** — only the transcript chars since the last pass, plus an "already captured" digest. There is no growing prefix in the messages, and the genuinely stable part (`tools` + `system`) is comfortably under Haiku 4.5's **4096-token minimum cacheable prefix**, so a breakpoint there would silently never cache.

The reuse is one level down, inside `Agent::run` (`agent.rs:74-88`): the loop re-sends `messages.clone()` every turn with **byte-identical `system` and `tools`**, growing by an assistant turn and its tool results each iteration. On the processing path that prefix carries the whole assembled transcript — far above Sonnet's 1024-token minimum — and a multi-turn extraction run re-sends it once per turn. **That is the win.**

Two consequences:

- **Caching is per-request, not global.** Cache writes bill at 1.25×, and break-even at the 5-minute TTL is two requests. Single-shot calls (`prompts::summarize`, the forced `build_document` call) would pay the premium and never read. So the request carries an explicit opt-in and only `Agent::run` sets it.
- **Cross-phase caching does not work today and is not worth forcing.** `run_llm_phases` makes two Sonnet calls over the same transcript, but with different system prompts *and* different tool sets. Tools render at position 0, so the prefix diverges immediately and phase 2 can never read phase 1. Fixing that means reshaping dam's phase architecture — out of scope here, noted as a future lever.

### 2.2 Usage accounting is a prerequisite, not a follow-up

`Usage` is `{input_tokens, output_tokens}` (`llm.rs:81-84`) and the `llm_usage` table mirrors it. The API reports `cache_creation_input_tokens` and `cache_read_input_tokens` as **separate fields**, and `input_tokens` becomes the *uncached remainder only*.

Turning on caching without extending `Usage` first would make R9 under-report and display a large cost drop that never happened — corrupting the one measurement that tells us whether $12.99 is viable. **Extend `Usage` and the `llm_usage` columns first, then enable caching.**

### 2.3 The Sonnet 5 swap is not a one-line change

**Claude Sonnet 5 turns adaptive thinking ON by default when the request omits a `thinking` field.** Sonnet 4.5 omitting it meant no thinking. A bare model-string swap therefore starts spending thinking tokens on every processing call and can truncate output, since `max_tokens` caps thinking + response together (`max_tokens` is 4096 for extraction, 1024 for summary).

Also note `Agent::run` sends `tool_choice: None` — the extraction agent reaches for `add_item` **voluntarily**, and Sonnet 5 with thinking disabled is documented as less likely to reach for tools. So `thinking: disabled` is the cheapest but risks quality on exactly the path that matters. Preferred config is **adaptive thinking + `effort: low`**, which preserves tool-reaching while controlling spend.

**Thinking can cost more than caching saves.** Sonnet 5's introductory pricing is −33% on the Sonnet legs, worth ~$0.05 on a long walk. But thinking bills at *output* rates: ~1,000 thinking tokens across the ~8 Sonnet calls in a walk is ~$0.08 at $10/MTok. Left uncontrolled, the swap is a net **loss** that also erases the caching win. It is only a gain at `effort: low` (or thinking off), which is precisely why it must not ship on faith.

This is measurable rather than a judgement call: `cargo test -p evals` is a deterministic F0.5 grader over a synthetic site-walk corpus. Land the swap behind an eval comparison — quality from the grader, cost from `usage_totals_detailed()` — not a vibe.

**Target after all three levers: ~$0.15/walk blended** (~$0.10 short, ~$0.25 long), break-even ~74 walks/month, 45–73% margin at typical use. Caching alone takes a long walk from ~$0.43 to ~$0.25 by cutting the processing-extraction leg ~70%.

---

## 3. Tiers

Two tiers, not three. The cost driver is **walks**, not features — so the free tier is capped on volume, not crippled on capability. A feature-gated free tier with unlimited walks means the heaviest free users cost the most and never convert.

| | Free | Pro — $12.99/mo |
|---|---|---|
| Walks | **5/month** (server config, tunable without a build) | Unlimited (fair-use ceiling, §5.3) |
| Core loop: voice → notes → document | Full | Full |
| Editable notes, photos, vocabulary | Yes | Yes |
| Letterhead Studio (logo, color, fonts) | Basic | Full |
| **Upload your own template** | — | **Yes** |
| Custom doc types / custom fields | — | Yes |
| "Prepared with Jefe" footer | Present | Removed |

Free does the marketing — every document a free user sends carries the footer — and costs ~$1/month at the cap. Pro is anchored on *their paperwork, their branding, no footer*.

---

## 4. What has to get built

### 4.1 Core: vision content blocks (dam's domain — the one hard dependency)

`ContentBlock` in `crates/harness/src/llm.rs:14` is `Text | ToolUse | ToolResult | Unknown`. **There is no image or document variant — the harness cannot send a PDF or a photo to the model today.** Template comprehension is impossible until this exists.

Additive, and small:

```rust
ContentBlock::Image    { source: ImageSource }   // {type:"base64", media_type, data}
ContentBlock::Document { source: DocumentSource } // {type:"base64", media_type:"application/pdf", data}
```

The existing `#[serde(other)] Unknown` fallback means adding variants is backward-compatible. This is the only piece that genuinely needs core work; everything else is app-side or proxy-side.

### 4.2 Core: the comprehension pass

`infer_document_schema(file_bytes, media_type, trade_key) -> DocumentSchema` — one vision call returning a *draft* schema in the exact shape `save_document_schema` already accepts. It infers sections, field labels, field `kind`, and a `fill` guess (`walk` / `manual` / `static`). It **does not persist** — the draft goes to the app for confirmation, and only the operator's confirmed version is saved. That boundary is what makes the confirm-once rule real rather than advisory.

### 4.3 App: the Document Builder (sac — and it is the confirm-once screen)

This was already sac's half of the v2 plan and was never built. **It is not additional scope for upload — it is the same screen.** The confirm-the-inferred-fields step *is* the Document Builder, pre-populated from §4.2 instead of empty. Building it once serves both the authoring path (§3 "custom doc types") and the upload path.

Sheet off the board, same pattern as Letterhead Studio: reorder/rename sections, add/remove fields, set each field's type and fill mode, name the doc type and its number prefix. Hands the result to `save_document_schema`.

### 4.4 App: schema-driven rendering

The review screen and PDF render from the schema's sections and fields rather than the fixed layout. `manual` fields become tap-to-fill (reuse the amount-edit interaction from #232). `static` fields render as blocks — and the app-side `DocumentLayout` terms/signature migrate in here as `static` fields, per the v2 plan §5.3.

### 4.5 Proxy + StoreKit (the same project)

Entitlement must be enforced **where the tokens are spent**. A client-side walk counter is cosmetic: reinstalling resets it, and a patched client hits the API directly.

**The client side is nearly free.** `EngineConfig` already carries `baseUrl` (`EngineResolution.swift:169`) and `AnthropicProvider` already has `with_base_url` — point `baseUrl` at the proxy and swap the baked key for a per-install token. **No core routing changes.**

The proxy (one serverless endpoint — Workers/Vercel; no accounts, no user table, no login):
1. Verify **App Attest** — caller is a genuine build of our app.
2. Verify the **StoreKit 2 transaction JWS** against the App Store Server API → entitled or not.
3. Free tier: check + increment the walk counter, keyed to **DeviceCheck**'s two persistent per-device bits, which survive reinstall. This is how we meter anonymously without forcing a contractor to make an account at first launch.
4. Forward to Anthropic with the real key. **Record the response's four token classes per install** — that is the cost-side meter §5.3 keys the fair-use ceiling on, and it costs nothing extra because the proxy already has the response in hand.

This also retires the launch blocker: **`EngineResolution.swift:136` reads a live `sk-ant-` key out of `Info.plist`, baked in by CI.** Anyone who downloads the IPA can extract it and spend against Isaac's account. Fine for a handful of testers; not fine on a public listing.

---

## 5. Decisions worth writing down

**5.1 The free-tier cap is server config, not a constant.** 5 walks/month is a starting guess. Making it a proxy-side value means tuning it never requires an App Review cycle.

**5.2 Upload is Pro, not a third tier.** §1 and §2. Revisit only if measured comprehension cost turns out to be materially higher than estimated.

**5.3 "Unlimited" carries a fair-use ceiling, and it keys on cost — not walk count.** §2 is the reason: a walk varies ~3x in cost with its length, so a count-based ceiling is calibrated to a unit whose price we don't control. Forty long walks cost more than seventy short ones. The proxy sees every call, so metering *tokens* is nearly free and is the number that actually predicts the bill. Trip review at roughly **$8/month of spend** rather than at a walk count. Meter it, don't advertise it, and only act if it happens.

**5.4 Free-tier walks stay a count, deliberately.** "5 walks/month" is legible to a contractor in a way "$1.20 of inference" never will be. The count is the *promise*; cost metering runs underneath it. The cap is low enough that length variance can't hurt much (5 long walks ≈ $1.25 worst case).

**5.5 One `line_items` section per document** (v2 plan §7.2, dam's recommendation — and already enforced core-side by `validate_schema`, which rejects a schema with 0 or 2+). Inference maps the template's obvious table region to it and drops any second candidate.

**5.6 The inferred schema is never auto-saved.** Draft → operator confirms → save. No exceptions; this is the whole reliability argument.

---

## 6. Phasing

Ordered so nothing is blocked on dam's return, and so the launch blocker clears early. Steps 1–3 are the cost levers and must run in this order — §2.2 explains why the meter has to be able to see cache tokens before caching is switched on.

1. **Usage accounting** — extend `Usage` + the `llm_usage` columns with `cache_creation_input_tokens` / `cache_read_input_tokens`. No behavior change; makes the R9 meter capable of telling a real saving from a reporting artifact.
2. **Prompt caching** — `cache_prefix` opt-in on `CompletionRequest`, emitted as top-level `cache_control` by the Anthropic provider, set by `Agent::run` only (§2.1).
3. **Sonnet 5 + explicit thinking config** — model string plus `thinking`/`effort`, landed against an eval comparison (§2.3).
4. **Proxy + StoreKit** — clears the baked-key blocker. Independent of the template feature; can ship to TestFlight behind the flag (§8) as soon as it's green.
5. **Document Builder + schema-driven rendering** — pure app-side, sac, no core dependency. Ships the "custom doc types" half of Pro on its own.
6. **Vision content blocks** (§4.1) — the one core item. Needed only for step 7.
7. **Comprehension pass + upload flow** — lands on the Builder from step 5 as the confirm screen.
8. **App Store submission.**

Steps 1–5 have zero dependency on §4.1, so a delay on the core item delays only upload, not the launch-readiness work.

---

## 7. Open questions for dam

1. **Vision blocks (§4.1)** — any objection to the additive `Image`/`Document` variants, or a shape you'd prefer? This is the only hard blocker on the feature.
2. **Which model for comprehension?** It's a one-shot vision + structured-output call. Sonnet 5 seems right; Opus if accuracy on messy scans demands it. Cost is negligible either way at once-per-template.
3. **Prompt caching on live extraction (§2)** — your loop. Is the growing-transcript prefix stable enough to take a `cache_control` breakpoint, given Haiku 4.5's 4096-token minimum?
4. **Does `infer_document_schema` belong on the engine or as a free function?** Schemas are operator-scoped, not session-scoped, so engine-keyed matches `save_document_schema` — but it touches no store state.

---

## 8. Boundary and build lanes

Core owns vision blocks, comprehension, fill, numbering, sync (dam). Document Builder, rendering, StoreKit UI, and the paywall are sac's. The proxy is new ground and nobody's yet — it is small enough that whoever gets there first should take it.

**Monetization ships behind a build-time flag, not a long-lived branch.** The release lane already keys on trigger, not branch identity: a push to `main` publishes internally to TestFlight, a `v*` tag cuts an external candidate (`release.yml:61-64`). Gate the paywall and premium tiering off in the branch-push lane and on in the tag lane, and TestFlight keeps receiving every field fix while never seeing a payment screen. The alternative — a long-lived `release/app-store` branch — inverts the testing story: the public build would be the one that got *less* field use than the beta, and every fix would owe a merge.
