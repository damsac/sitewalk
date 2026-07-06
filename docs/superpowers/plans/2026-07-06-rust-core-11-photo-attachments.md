# Murmur Rust Core — Plan 11: Photo Attachments

> **For agentic workers:** REQUIRED SUB-SKILL: use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax. The Rust tasks (1–3) are **hermetic**: in-memory / temp-file `Store`, `MockProvider`, `with_providers` — **no model, no `whisper` feature, no camera, no filesystem-of-real-photos, no network**. `cargo test --workspace` must NEVER require the `whisper` feature or a model file (the load-bearing CI invariant). The Swift task (4) is **not CI-gated** (real-core-only, needs the gitignored `MurmurCoreFFI` xcframework) and its **visual design is explicitly sac's** — `// sac:` markers throughout. Run `cargo`/`xcodegen` **inside** the Nix dev shell; run `xcodebuild` **outside** it (Nix linker env breaks Xcode `ld`). Never read `.env` or `project.local.yml`.

> **Plan review (2026-07-06) — APPROVE WITH CONDITIONS (applied).** Reviewed against real code: architecture sound; both worked examples (A cascade, B swap-demotion) recomputed by hand and held against `sessions.rs`/`items.rs`/`migrations.rs`; the DISCARD/`cancel()` path is covered because `cancel()` tombstones via the same `delete_session` cascade (which now includes photos). Two conditions applied: (1) Task 3 Step 1 test snippet rewritten to the real `vocabulary.rs` pattern (inline `SpyStore` + literal `Providers`) — the earlier `NullStore`/`providers()` helpers don't exist; (2) Task 4 sweep scoped to **app-open only** with the background-sweep-vs-in-flight-capture race spelled out.

**Goal.** During a site walk (or at document review) the user snaps photos. Photos attach to the **session** and optionally to a **specific captured item**, survive the live→authoritative board swap, and surface in the processed document. **Privacy rule (spec §1 pillar 2, R-privacy): photo *bytes* never leave the device** — same posture as audio. This was queued in the HANDOFF answers as "photos need a schema migration" (`meta/dam/STATE.md:46`) and sequenced as ROADMAP "Up Next" item 4 (`meta/ROADMAP.md:22`, "rides a migration after `source`").

**What lands:**
1. **murmur-core** — a `photos` table (migration **v5**, transactional per the house pattern), a `Photo` domain type, and `Store` CRUD. Photos carry a mandatory `session_id` and an **optional** `item_id`. The **session cascade delete tombstones photos** (the Plan 03 orphan lesson). The **swap-at-finish and the pre-process clear demote** item-linked photos to session-level when their item is swept — so a photo *never* dangles off a tombstoned item and is *never* lost.
2. **File-handling seam** — core stores **metadata + a relative filename only**; the shell owns the bytes directory (`<Documents>/photos/`). Byte deletion is a **shell-owned reconciling sweep** (the narrowed-sweep pattern from 07-carry / the audio-retention posture of spec §8), never a synchronous core filesystem call. Core exposes exactly one reconciliation query.
3. **Processing** — photos surface in the document at review time through a **parallel read path** (`list_photos_for_session`), **not** through the extraction LLM. `SessionProcessor::process()` is **unchanged**. Vision-model analysis is explicit future work.
4. **ffi** — throwing, panic-free `add_photo` / `list_photos` / `remove_photo` / `list_live_photo_filenames` on `MurmurEngine`, plus a `WalkSession.session_id()` getter for the in-walk path. A `PhotoRef` uniffi record and `EngineError::Photo`.
5. **apps/ios** — minimal capture wiring (camera/photo-picker → write bytes → call FFI) behind the existing `WalkEngine` seam; `DemoWalkEngine` no-op-ish in-memory conformance; the reconciling sweep on app-open; `// sac:` markers everywhere visual.

**What this plan is NOT (see Non-goals for the full list).** No vision-model photo analysis; no per-item `photo_count` on the *live board* (the `BoardItem.photo_count` seam stays `0` in v1 — wiring it needs a batched count in the board snapshot, a fast-follow); no photo sync/upload (bytes are local-only forever); no EXIF ingestion beyond an optional `captured_at`; no editing/annotation/thumbnailing; no evals (none apply — see §Evals).

---

## Hard dependencies (all DONE, on `main`)

- **Plan 03** (`crates/murmur-core`): `Store` single-writer API; the transactional migration runner (`store/migrations.rs`: `MIGRATIONS` append-only array, `migrate_with` — each entry's DDL + `user_version` bump commit in one transaction, mid-batch failure rolls back — `migrations.rs:126–152`); UUIDv7 ids (`ids::new_id`); tombstones (`deleted_at`); `delete_session` **cascade** (session + items + artifacts in one tx — `sessions.rs:296–316`); `foreign_keys = ON` (`store/mod.rs:47`); the injectable `Clock` (`store/mod.rs:53–60`).
- **Plan 06a** (`items.source`): `ItemSource { Live, Authoritative, Manual }`; the finish-swap (`finish_session_processed` tombstones `source IN ('live','authoritative')` not in `run_item_ids` — `sessions.rs:370–405`); the pre-process clear (`clear_authoritative_outputs` tombstones authoritative items + all artifacts — `sessions.rs:429–445`). **These are the two places the demotion rule hooks in.**
- **Plan 07** (`crates/ffi`): `MurmurEngine` (`store: Arc<Mutex<Store>>`, `engine.rs:131`); `WalkSession` (holds `session_id: String` + `store: Arc<StdMutex<Store>>`, `session.rs:19–24`); `EngineError` (`#[uniffi(flat_error)]`, no panics across FFI — `engine.rs:15–33`); the `BoardItem.photo_count: u32` **seam already exists** (`events.rs:9–16`, defaulted to `0` in `convert::board_item` — `convert.rs:11–19`, doc'd "photo attachment sync is Deferred 6"); the display-copy-free projection posture (`document.rs`).
- **Plan 10** (`crates/ffi/src/vocabulary.rs`): the **exact precedent** for engine-keyed CRUD across FFI — a `#[uniffi::export] impl MurmurEngine` block in its own file, throwing/panic-free, lock-then-return. Photos follow this shape.
- **iOS shell** (`apps/ios`): `WalkEngine` protocol + `DemoWalkEngine`/`MurmurEngine` conformers (`Engine/WalkEngine.swift`, `Engine/MurmurEngine.swift`); `@Observable AppModel`; the `// sac:` handoff convention Plan 10 established (`Flow/VocabularyView.swift`).

**Verified API facts (checked against source, not guessed):**
- Migrations are a `&[&str]` appended to (`migrations.rs:7`); **current length = 4** (v1 schema, v2 `items.source`, v3 `sessions.template`, v4 `document_sequences`), so `user_version` on a current DB = **4**. This plan adds entry index 4 → **v5**. `Store::open_in_memory` asserts it migrates to `MIGRATIONS.len()` (`store/mod.rs:68–75`).
- `delete_session` is the cascade template to copy: `unchecked_transaction()`, tombstone the parent (guard `changed == 0` → `NotFound`), then tombstone each child `WHERE session_id = ?2 AND deleted_at IS NULL`, `tx.commit()` (`sessions.rs:296–316`). Its test pins raw-rows-survive (`delete_session_cascades_to_items_and_artifacts`, `sessions.rs:731–756`).
- `finish_session_processed` runs its item-sweep UPDATE, then `mark_session_processed`, then `record_llm_usage`, all inside one `unchecked_transaction()` (`sessions.rs:377–404`). The demotion UPDATE inserts **after the item-sweep, before commit**.
- `clear_authoritative_outputs` tombstones authoritative items + all artifacts in one tx (`sessions.rs:429–445`).
- `add_item_with_source` validates the session via `get_session` (NotFound if missing/tombstoned) then `insert_item` (`items.rs:39–48`). `delete_item` is a single-row tombstone keyed by id (`items.rs:149–159`).
- `Store` internals available to new code: `self.conn`, `self.device_id`, `self.now()` (`store/mod.rs:31–60`); `new_id()` (UUIDv7, `ids.rs`).
- `EngineError` variants today: `Store`/`Runtime`/`BeginWalk`/`Memory`, all `String`-carrying, `flat_error` (`engine.rs:17–33`). `WalkSession.session_id` is a private field (`session.rs:20`) — no getter exists yet.
- The board snapshot (`emit_board_snapshot`, `session.rs:359–378`) maps items through `convert::board_item`, which hardcodes `photo_count: 0`. **v1 leaves this `0`** (decision D6).

**Spec basis:** §1 pillar 2 ("Audio never leaves the device … all data is local-first"); §8 ("Nothing but LLM requests leaves the device"; audio retention is user-controlled — the **retention/sweep** precedent for bytes); §9 (UUIDv7, every row `created_at`/`updated_at`/`device_id`, deletes are tombstones, single-writer); Rev 2 §1 (artifacts are a **seam** — informs the photos-vs-artifacts call, D1); §2 story 10 (manual parity — nothing agent-only); the "capture is never lost" invariant carried through Plans 06a/07/08.

---

## Architecture — decisions, justified (reviewers read these first)

### D1. Photos are a **separate table**, not an artifact `kind` (argued, then chosen)

The `artifacts` table (`migrations.rs:62–72`) is `(id, session_id, kind, title, body, …)` — a **generated-document** seam (Rev 2 §1): the model writes reports/estimates/documents whose payload is markdown/JSON in `body`, hung off a session. Photos are structurally different:

| | artifact | photo |
|---|---|---|
| origin | agent-generated | **user-captured** (device camera) |
| payload | text `body` (synced) | **binary bytes on disk (local-only, never synced)** |
| needs `item_id` | no | **yes (optional)** — attach to a specific captured item |
| needs `captured_at` | no (`created_at` suffices) | yes (EXIF/shot time ≠ row-insert time) |
| lifecycle vs swap | cleared wholesale by `clear_authoritative_outputs` | must **survive** the swap, demote (D3) |

Modeling a photo as `kind='photo'` would force `title`/`body` to carry a filename by convention (untyped, no `item_id`, no `captured_at`), and — fatally — `clear_authoritative_outputs` tombstones **all** artifacts (`sessions.rs:438–442`), so every reprocess would wipe the user's photos. A separate table gets its own typed columns, its own cascade/demote rules, and stays clear of the artifact sweep. **Chosen: separate `photos` table.** (Rejected: artifact `kind='photo'` — wrong lifecycle, untyped, would be wiped on reprocess.)

The table still follows the **sync-ready row shape** (spec §9): UUIDv7 `id`, `created_at`/`updated_at`/`device_id`, `deleted_at` tombstone. The **metadata row syncs; the bytes do not** (D4) — this is the one place the two diverge, and it is exactly the audio posture (transcript syncs, audio is local, spec §8).

### D2. Schema — `photos` (migration v5, transactional)

Appended as `MIGRATIONS[4]` (→ `user_version = 5`). New table only, **no backfill** — existing rows are untouched, so a v4 DB upgrades by running one `CREATE TABLE` + two indexes in a single transaction (the runner already wraps each entry in `BEGIN … PRAGMA user_version=5 … COMMIT`, `migrations.rs:137–141`).

```sql
-- v5: photo attachments (Plan 11). Metadata + a relative filename only; the
-- BYTES live in the shell's Documents dir and never sync (privacy: photos
-- never leave the device, spec §1/§8). Row shape is sync-ready (§9).
CREATE TABLE photos (
    id          TEXT PRIMARY KEY,
    session_id  TEXT NOT NULL REFERENCES sessions(id),
    item_id     TEXT REFERENCES items(id),   -- NULL = session-level attachment
    filename    TEXT NOT NULL,               -- shell-owned, opaque to core (relative name)
    captured_at INTEGER NOT NULL,            -- unix seconds (EXIF shot-time or now())
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL,
    device_id   TEXT NOT NULL,
    deleted_at  INTEGER
);
-- At most one LIVE row per filename → the reconciling sweep (D4) is unambiguous.
CREATE UNIQUE INDEX idx_photos_filename_live ON photos(filename) WHERE deleted_at IS NULL;
CREATE INDEX idx_photos_session ON photos(session_id) WHERE deleted_at IS NULL;
```

`item_id` is nullable and FK-checked (`foreign_keys=ON`). Because tombstones are **soft** (`deleted_at`, the row stays), a tombstoned `item` still satisfies the FK — so the FK never *breaks* when an item is swept; the danger is the **logical** dangle (D3), which the demotion rule handles.

### D3. The item_id-across-swap bug — confronted head-on: **demote, never dangle, never lose**

**The bug.** During recording the live board holds an item `I1` (`source='live'`). The user attaches photo `P2` with `item_id = I1`. At `finish()`, `process()` re-extracts a *fresh* authoritative item `A1` (new UUID) and `finish_session_processed` **tombstones `I1`** (it is `live` and not in `run_item_ids`). Now `P2.item_id` points at a **tombstoned** item: every item read filters `deleted_at IS NULL` (`items.rs:110–118`), so `P2` is logically attached to a row that has vanished. If the UI groups photos under items, `P2` becomes an orphan under an invisible item. There is **no** correspondence between `I1` and `A1` (the model re-extracts from scratch), so we *cannot* re-point `P2 → A1`.

**The rule (chosen).** *An item tombstone demotes its still-live photos to session-level (`item_id := NULL`); a session tombstone cascade-tombstones its photos.* Concretely, wherever items get swept, the same transaction runs a **demotion**: any live photo whose `item_id` references a now-tombstoned item in that session is set to `item_id = NULL` (and `updated_at` bumped). The photo is **never lost** (it still carries `session_id`, surfaces in `list_photos_for_session`), and it **never dangles** (no live photo references a tombstoned item). Item linkage is preserved for items that *survive* the swap (`manual` items; this-run `authoritative` items). Re-linking a demoted photo to the authoritative successor is **future work** (needs item-identity matching we do not have) and is named as such.

Demotion hooks into exactly three item-tombstone sites; the fourth (session delete) cascades instead:
1. `finish_session_processed` — after the item-sweep, before commit (the primary case).
2. `clear_authoritative_outputs` — after the authoritative-item tombstone (a prior run's item that held a photo).
3. `delete_item` — the user manually deletes an item; demote its photos (non-destructive: deleting a wrong todo must not delete a real photo).
4. `delete_session` — cascade-tombstone photos (the session is gone; keep the bytes only until the sweep).

(Rejected alternative: *leave `item_id` dangling, read only session-scoped.* The FK doesn't break, but a live photo pointing at a logically-dead item is a latent UI orphan and a trap for any future item-JOIN; demotion is one extra scoped UPDATE and makes the invariant true, not merely tolerated.)

(Rejected alternative: *forbid attaching to `live` items — only `manual`/`authoritative`.* But the live board is exactly what the user sees mid-walk; the natural gesture is "tap this item, snap a photo." Forbidding it pushes the complexity into the UI and still leaves the review-time attach case. Demotion is simpler and universal.)

### D4. File-handling seam — core owns metadata, the **shell owns bytes**, deletion is a reconciling sweep

Core is a SQLite store with **no filesystem handle for media** and must stay that way (testable in-memory, sync-ready). So:

- **Bytes live in the shell**, at `<Documents>/photos/<name>`. The `filename` is a **shell-chosen unique string, opaque to core** (core only enforces non-empty + the live-unique index, D2). The shell should name files by a fresh UUID (`<uuid>.jpg`) — *independent* of the row `id`, so there is no mint-order chicken-and-egg (the shell need not wait for `add_photo` to return an id before naming the file).
- **Write order = bytes first, then metadata.** The shell writes `<uuid>.jpg`, *then* calls `add_photo(..., filename: "<uuid>.jpg", ...)`. A crash in between orphans a file with no row → collected by the sweep. A committed row therefore **always** has its bytes (the sweep never deletes a file whose filename is in the live set). This mirrors "transcript persists before extraction" — capture is never lost, and a not-yet-committed capture is acceptably lost.
- **Deletion is a shell-owned sweep, not a synchronous core call.** `remove_photo(id)` (or a session/item tombstone/demote) sets `deleted_at` on the **row** (sync-safe: the tombstone syncs). The **bytes** are reclaimed by a **reconciling sweep** the shell runs on app-open / background: *delete every file in `photos/` whose name is not in `core.list_live_photo_filenames()`.* Idempotent, crash-safe, order-independent; it collects both tombstoned-row files and never-committed orphan files with one rule. No "bytes-deleted" flag is needed — the filesystem *is* the byte-state; the live set *is* the intent. This is the narrowed-sweep pattern (07-carry) and the audio-retention posture (spec §8: "discard audio after N days" is a sweep, not a foreground delete).
- **Core's entire file contract is one query:** `list_live_photo_filenames() -> Vec<String>` (all sessions, `deleted_at IS NULL`). Core never opens, writes, or deletes a byte.

**Who deletes files on tombstone?** The **shell**, via the sweep — never core. Justified: core has no media FS handle, must stay hermetically testable, and the sweep is strictly more robust than a synchronous delete (it also reaps crash-orphans and demote/cascade byte-garbage with the same code path).

### D5. Processing — photos surface via a **parallel read path**; `process()` is untouched; vision is future work

Minimal v1: the extraction/summary/`build_document` pipeline does **not** read, analyze, or reference photo bytes — no vision model, no image tokens, no change to `SessionProcessor::process()` or any prompt. Photos surface at **document review** through `list_photos_for_session(session_id)` (a session gallery) and are independently attachable/removable there. The document artifact JSON is **not** extended with photo references in v1 (so exported PDFs embedding photos is future work). Rationale: vision analysis is a large, separate capability (model choice, cost/R9, privacy re-analysis) and coupling photos into the LLM path now would bloat the plan and the token budget. **Named future work:** (a) feeding photos to a vision pass in extraction; (b) embedding photo refs into the document artifact for share/PDF. Neither is built here; both are clean seams (the `photos` table + `list_photos_for_session` already exist for them).

### D6. Live board — **no new event; `photo_count` stays `0` in v1**

The `BoardItem.photo_count` seam exists (`events.rs:9–16`) but `convert::board_item` hardcodes `0`. Wiring a real per-item count into the live board requires the board snapshot (`emit_board_snapshot`, `session.rs:359–378`) to **batch-load** counts (one `GROUP BY item_id` query, to avoid an N+1 per snapshot) and thread them into the projection. That is a self-contained **fast-follow**, not v1: the value of live per-item counts mid-walk is low (the user just took the photo — the capture UI can confirm locally), and it touches the hot board path. **v1 decision:** leave `photo_count = 0`, add **no** `WalkEvent`. Attachment feedback during the walk is the capture UI's own local concern. Core ships `count_photos_for_session` (cheap, for the review screen and the fast-follow) but the live board does not consume it in v1. (Named future work: batched per-item counts in `emit_board_snapshot`.)

### D7. FFI — engine-keyed CRUD (Plan 10 precedent) + one `WalkSession` getter

Photos are usable **during** a walk (attach to the active session) **and after** it (review-time gallery, add/remove on a `Processed` session — there is no live `WalkSession` then). So the CRUD is **engine-keyed by `session_id`** (exactly like vocabulary is engine-keyed, `vocabulary.rs`), not gated behind a live `WalkSession`. To let the in-walk capture UI obtain its session id, add a tiny `WalkSession.session_id()` getter (the only new `WalkSession` surface).

```rust
// on MurmurEngine (crates/ffi/src/photos.rs — new file, Plan 10 shape)
fn add_photo(&self, session_id: String, item_id: Option<String>,
             filename: String, captured_at: Option<u64>) -> Result<PhotoRef, EngineError>;
fn list_photos(&self, session_id: String) -> Result<Vec<PhotoRef>, EngineError>;
fn remove_photo(&self, photo_id: String) -> Result<(), EngineError>;
fn list_live_photo_filenames(&self) -> Result<Vec<String>, EngineError>; // sweep (D4)

// on WalkSession (crates/ffi/src/session.rs)
fn session_id(&self) -> String;  // getter, so the capture UI can call engine.add_photo(...)
```

`PhotoRef` is a `uniffi::Record` (`id`, `session_id`, `item_id: Option<String>`, `filename`, `captured_at`) — a display-copy-free projection (D-Plan07 posture; the shell resolves `filename` → a real file URL). Errors are panic-free (Plan 07 CANON): a poisoned lock or store error → `EngineError`; validation (empty filename, missing session, `item_id` not in the session) → a new **`EngineError::Photo(String)`**. `captured_at = None` → core stamps `self.now()`. Methods are `&self` (no session handed out), mirroring vocabulary.

### D8. Sync posture — the row syncs, the bytes never do (documented, not built)

No sync engine ships in v1 (spec §12). But the design is sync-ready and **honest about the split**: the `photos` **row** is a normal sync-ready row (UUIDv7, timestamps, `device_id`, tombstone) that a future change-log layer picks up like any other; the **bytes are permanently local** (privacy). A synced tombstone on another device means "this photo is deleted" (the other device's sweep reaps its own local copy, if it ever had one); a synced *insert* referencing a `filename` whose bytes never crossed the wire is a **missing-media** row the UI renders as a placeholder. This is documented as the deliberate posture; no reconciliation is built. (Flagged for dam in Open Questions: if photo *sharing* across devices is ever wanted, that is a bytes-transport feature, explicitly out of the local-first privacy model as written.)

---

## File Structure

```
crates/
  murmur-core/src/
    store/
      migrations.rs   # MODIFY: append MIGRATIONS[4] — photos table + 2 indexes (v5)
      photos.rs       # NEW: Photo row mapping + Store CRUD (add/list/get/remove/
                      #      list_live_photo_filenames/count_photos_for_session) + demotion helper
      mod.rs          # MODIFY: `mod photos;`
      sessions.rs     # MODIFY: delete_session cascades to photos; finish_session_processed
                      #         + clear_authoritative_outputs demote item-linked photos
      items.rs        # MODIFY: delete_item demotes its photos to session-level
    domain.rs         # MODIFY: `Photo` struct (serde, sync-ready shape)
    lib.rs            # MODIFY: pub use domain::Photo
  ffi/src/
    photos.rs         # NEW: #[uniffi::export] impl MurmurEngine { add/list/remove_photo,
                      #      list_live_photo_filenames } + PhotoRef record + convert
    session.rs        # MODIFY: #[uniffi::export] fn session_id(&self) -> String on WalkSession
    engine.rs         # MODIFY: EngineError::Photo variant
    lib.rs            # MODIFY: pub mod photos; pub use photos::PhotoRef;
apps/ios/Sources/
  Engine/WalkEngine.swift      # MODIFY: photo methods on the protocol (+ sessionId)
  Engine/DemoWalkEngine.swift  # MODIFY: in-memory conformance (demo works with no backend)
  Engine/MurmurEngine.swift    # MODIFY: forward to FFI (#if canImport(MurmurCoreFFI))
  App/AppModel.swift           # MODIFY: photo state + capture/remove + app-open sweep
  <Photo capture/gallery>      # NEW: functional-plain capture + gallery (// sac: visuals)
  Flow/*.swift                 # MODIFY: entry points (capture button, review gallery) — // sac:
docs/
  superpowers/plans/2026-07-06-rust-core-11-photo-attachments.md   # THIS FILE
meta/ROADMAP.md                # MODIFY (Task 5): mark photo-attachment schema landed
```

---

## Part A — Core: schema, domain, CRUD, cascade + demotion

### Task 1: migration v5 + `Photo` domain type + `Store` CRUD

**Files:** modify `store/migrations.rs`, `store/mod.rs`, `domain.rs`, `lib.rs`; create `store/photos.rs`.

- [ ] **Step 1 — failing tests** (`store/photos.rs` `mod tests`). Use the `store_with_session` helper shape from `items.rs:169–173` (injected clock `|| 1000`).

```rust
#[test]
fn migrates_to_v5() {
    let s = Store::open_in_memory("device-a").unwrap();
    let v: i64 = s.conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
    assert_eq!(v, 5, "photos migration bumps user_version to 5");
}

#[test]
fn add_list_get_photo_session_level() {
    let (s, sid) = store_with_session();
    let p = s.add_photo(&sid, None, "a1b2.jpg", Some(1234)).unwrap();
    assert_eq!(p.session_id, sid);
    assert_eq!(p.item_id, None);
    assert_eq!(p.filename, "a1b2.jpg");
    assert_eq!(p.captured_at, 1234);
    assert_eq!(p.created_at, 1000); // injected clock
    assert_eq!(s.list_photos_for_session(&sid).unwrap(), vec![p.clone()]);
    assert_eq!(s.get_photo(&p.id).unwrap(), p);
}

#[test]
fn add_photo_captured_at_defaults_to_now() {
    let (s, sid) = store_with_session();
    let p = s.add_photo(&sid, None, "x.jpg", None).unwrap();
    assert_eq!(p.captured_at, 1000, "None captured_at stamps now()");
}

#[test]
fn add_photo_to_item_validates_membership() {
    let (s, sid) = store_with_session();
    let item = s.add_item(&sid, "todo", "deck").unwrap();
    let p = s.add_photo(&sid, Some(&item.id), "d.jpg", None).unwrap();
    assert_eq!(p.item_id.as_deref(), Some(item.id.as_str()));
    // item that isn't in this session → error (NotFound item)
    let other = s.start_session(None).unwrap();
    let other_item = s.add_item(&other.id, "todo", "x").unwrap();
    assert!(matches!(
        s.add_photo(&sid, Some(&other_item.id), "e.jpg", None),
        Err(CoreError::InvalidState(_))
    ));
}

#[test]
fn add_photo_to_missing_session_is_not_found() {
    let (s, _) = store_with_session();
    assert!(matches!(
        s.add_photo("nope", None, "z.jpg", None),
        Err(CoreError::NotFound { entity: "session", .. })
    ));
}

#[test]
fn live_filename_uniqueness_is_enforced() {
    let (s, sid) = store_with_session();
    s.add_photo(&sid, None, "dup.jpg", None).unwrap();
    assert!(s.add_photo(&sid, None, "dup.jpg", None).is_err(), "no two live rows share a filename");
    // after tombstone, the name frees up
    let again = s.list_photos_for_session(&sid).unwrap()[0].id.clone();
    s.remove_photo(&again).unwrap();
    assert!(s.add_photo(&sid, None, "dup.jpg", None).is_ok(), "tombstoned filename can be reused");
}

#[test]
fn remove_photo_is_a_tombstone() {
    let (s, sid) = store_with_session();
    let p = s.add_photo(&sid, None, "a.jpg", None).unwrap();
    s.remove_photo(&p.id).unwrap();
    assert!(s.list_photos_for_session(&sid).unwrap().is_empty());
    assert!(matches!(s.remove_photo(&p.id), Err(CoreError::NotFound { .. })));
    // raw row survives (tombstone, not erase)
    let raw: i64 = s.conn.query_row("SELECT COUNT(*) FROM photos WHERE id=?1", [&p.id], |r| r.get(0)).unwrap();
    assert_eq!(raw, 1);
}

#[test]
fn list_live_photo_filenames_spans_sessions_and_skips_tombstoned() {
    let (s, sid_a) = store_with_session();
    let sid_b = s.start_session(None).unwrap().id;
    s.add_photo(&sid_a, None, "a.jpg", None).unwrap();
    let gone = s.add_photo(&sid_b, None, "b.jpg", None).unwrap();
    s.add_photo(&sid_b, None, "c.jpg", None).unwrap();
    s.remove_photo(&gone.id).unwrap();
    let mut names = s.list_live_photo_filenames().unwrap();
    names.sort();
    assert_eq!(names, vec!["a.jpg".to_string(), "c.jpg".to_string()], "b.jpg tombstoned → excluded");
}

#[test]
fn count_photos_for_session_counts_live_only() {
    let (s, sid) = store_with_session();
    s.add_photo(&sid, None, "a.jpg", None).unwrap();
    let g = s.add_photo(&sid, None, "b.jpg", None).unwrap();
    s.remove_photo(&g.id).unwrap();
    assert_eq!(s.count_photos_for_session(&sid).unwrap(), 1);
}
```

- [ ] **Step 2 — migration** (`store/migrations.rs`): append the D2 SQL as a new `r#"…"#` entry at the end of `MIGRATIONS` (index 4). Do **not** edit existing entries (append-only invariant, `migrations.rs:5–6`).

- [ ] **Step 3 — domain type** (`domain.rs`, sync-ready shape mirroring `Artifact`/`CapturedItem`):
```rust
/// A user-captured photo attached to a session (spec: photos never leave the
/// device — only this METADATA row is sync-ready; the BYTES live in the shell's
/// Documents dir, local-only forever, Plan 11 D4). `item_id` is an optional
/// attachment to a specific captured item; it is demoted to `None` if that item
/// is swept (Plan 11 D3), so a live photo never references a tombstoned item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Photo {
    pub id: String,
    pub session_id: String,
    pub item_id: Option<String>,
    /// Shell-owned, opaque to core: a relative filename in `<Documents>/photos/`.
    pub filename: String,
    pub captured_at: u64,
    pub created_at: u64,
    pub updated_at: u64,
    pub device_id: String,
}
```
Add `pub use domain::Photo;` to `crates/murmur-core/src/lib.rs` (alongside the other re-exports).

- [ ] **Step 4 — CRUD** (`store/photos.rs`, mirroring `items.rs`/`artifacts.rs` idioms — `PHOTO_COLS` const, `photo_from_row`, session-validate before insert):
  - `add_photo(&self, session_id, item_id: Option<&str>, filename, captured_at: Option<u64>) -> Result<Photo, CoreError>`: `get_session(session_id)?` (NotFound if missing/tombstoned); if `item_id` is `Some`, `get_item(item)?` and assert `item.session_id == session_id` else `InvalidState` (membership); `captured_at.unwrap_or(self.now())`; insert with `new_id()`, `created_at=updated_at=self.now()`, `self.device_id`. A live-filename collision surfaces as the sqlite unique-index error → map to `CoreError` (it already `From`s `rusqlite::Error`).
  - `get_photo`, `list_photos_for_session` (`WHERE session_id=?1 AND deleted_at IS NULL ORDER BY id ASC` — insertion order via UUIDv7), `remove_photo` (tombstone, `changed==0 → NotFound`), `list_live_photo_filenames` (`SELECT filename FROM photos WHERE deleted_at IS NULL`), `count_photos_for_session` (`SELECT COUNT(*) … deleted_at IS NULL`).
  - Add `mod photos;` to `store/mod.rs`.

- [ ] **Step 5 — verify:** `nix develop -c cargo test -p murmur-core photos` green; `open_in_memory_migrates_to_latest` (`store/mod.rs:68`) still passes (it asserts `== MIGRATIONS.len()`, now 5).

- [ ] **Step 6 — commit:** `feat(core): photos table (migration v5) + Photo domain + Store CRUD`

---

### Task 2: session cascade + the swap/clear/delete-item **demotion** (the D3 bug)

**Files:** modify `store/sessions.rs`, `store/items.rs`; tests in `store/photos.rs` (+ extend the existing cascade test).

- [ ] **Step 1 — failing tests** (arithmetic-pinned; reviewers recompute these by hand):

```rust
#[test]
fn delete_session_cascades_to_photos() {
    // Session S; items I1,I2; P1 session-level, P2 attached to I1.
    let s = Store::open_in_memory("device-a").unwrap().with_clock(std::sync::Arc::new(|| 5000));
    let sid = s.start_session(None).unwrap().id;
    let i1 = s.add_item(&sid, "todo", "I1").unwrap();
    let _i2 = s.add_item(&sid, "todo", "I2").unwrap();
    let p1 = s.add_photo(&sid, None, "p1.jpg", None).unwrap();
    let p2 = s.add_photo(&sid, Some(&i1.id), "p2.jpg", None).unwrap();

    s.delete_session(&sid).unwrap();

    // Both photos tombstoned in the same op; nothing readable, raw rows survive.
    assert!(s.list_photos_for_session(&sid).unwrap().is_empty());
    for id in [&p1.id, &p2.id] {
        let raw: i64 = s.conn.query_row("SELECT COUNT(*) FROM photos WHERE id=?1", [id], |r| r.get(0)).unwrap();
        assert_eq!(raw, 1, "tombstone, not erase");
        let del: i64 = s.conn.query_row("SELECT deleted_at FROM photos WHERE id=?1", [id], |r| r.get(0)).unwrap();
        assert_eq!(del, 5000);
    }
    // filenames leave the live set → the shell sweep will reap the bytes.
    assert!(s.list_live_photo_filenames().unwrap().is_empty());
}

#[test]
fn finish_swap_demotes_photos_of_swept_items_and_never_loses_them() {
    use crate::domain::ItemSource;
    let s = Store::open_in_memory("device-a").unwrap().with_clock(std::sync::Arc::new(|| 6000));
    let sid = s.start_session(None).unwrap().id;
    // Live board: I1 (live). Photos: P1 session-level, P2 attached to I1.
    let i1 = s.add_item_with_source(&sid, "todo", "live", ItemSource::Live).unwrap();
    let p1 = s.add_photo(&sid, None, "p1.jpg", None).unwrap();
    let p2 = s.add_photo(&sid, Some(&i1.id), "p2.jpg", None).unwrap();
    // This run extracts one authoritative item A1.
    let a1 = s.add_item_with_source(&sid, "todo", "auth", ItemSource::Authoritative).unwrap();
    s.end_session(&sid).unwrap();
    s.finish_session_processed(&sid, "done", &harness::Usage::default(), &[a1.id.clone()]).unwrap();

    // I1 swept (deleted_at=6000). A1 survives. Both photos survive at session scope.
    let items: Vec<String> = s.list_items_for_session(&sid).unwrap().into_iter().map(|i| i.id).collect();
    assert_eq!(items, vec![a1.id.clone()], "I1 swept, A1 remains");
    let photos = s.list_photos_for_session(&sid).unwrap();
    let by_id = |pid: &str| photos.iter().find(|p| p.id == pid).unwrap();
    assert_eq!(photos.len(), 2, "no photo lost in the swap");
    assert_eq!(by_id(&p1.id).item_id, None, "session-level P1 untouched");
    // THE FIX: P2's item_id was I1 (now tombstoned) → demoted to NULL, updated_at bumped.
    assert_eq!(by_id(&p2.id).item_id, None, "P2 demoted to session-level, not left dangling");
    assert_eq!(by_id(&p2.id).updated_at, 6000, "demotion bumps updated_at (sync-visible)");
    // Neither photo references a tombstoned item.
    assert!(photos.iter().all(|p| p.item_id.is_none()));
}

#[test]
fn finish_swap_keeps_photos_on_surviving_manual_and_this_run_items() {
    use crate::domain::ItemSource;
    let s = Store::open_in_memory("device-a").unwrap().with_clock(std::sync::Arc::new(|| 7000));
    let sid = s.start_session(None).unwrap().id;
    let manual = s.add_item_with_source(&sid, "note", "manual", ItemSource::Manual).unwrap();
    let a1 = s.add_item_with_source(&sid, "todo", "auth", ItemSource::Authoritative).unwrap();
    let pm = s.add_photo(&sid, Some(&manual.id), "pm.jpg", None).unwrap();
    let pa = s.add_photo(&sid, Some(&a1.id), "pa.jpg", None).unwrap();
    s.end_session(&sid).unwrap();
    // manual survives (never swept); a1 is in run_item_ids (survives) → both linkages kept.
    s.finish_session_processed(&sid, "done", &harness::Usage::default(), &[a1.id.clone()]).unwrap();
    let photos = s.list_photos_for_session(&sid).unwrap();
    assert_eq!(photos.iter().find(|p| p.id == pm.id).unwrap().item_id.as_deref(), Some(manual.id.as_str()));
    assert_eq!(photos.iter().find(|p| p.id == pa.id).unwrap().item_id.as_deref(), Some(a1.id.as_str()));
}

#[test]
fn clear_authoritative_outputs_demotes_photos_of_swept_authoritative_items() {
    use crate::domain::ItemSource;
    let s = Store::open_in_memory("device-a").unwrap().with_clock(std::sync::Arc::new(|| 8000));
    let sid = s.start_session(None).unwrap().id;
    let stale = s.add_item_with_source(&sid, "todo", "stale auth", ItemSource::Authoritative).unwrap();
    let p = s.add_photo(&sid, Some(&stale.id), "p.jpg", None).unwrap();
    s.clear_authoritative_outputs(&sid).unwrap();
    // stale auth item tombstoned; its photo demoted, not lost.
    assert!(s.list_items_for_session(&sid).unwrap().is_empty());
    let got = s.get_photo(&p.id).unwrap();
    assert_eq!(got.item_id, None, "demoted");
    assert_eq!(got.updated_at, 8000);
}

#[test]
fn delete_item_demotes_its_photos_not_deletes_them() {
    let s = Store::open_in_memory("device-a").unwrap().with_clock(std::sync::Arc::new(|| 9000));
    let sid = s.start_session(None).unwrap().id;
    let item = s.add_item(&sid, "todo", "wrong todo").unwrap();
    let p = s.add_photo(&sid, Some(&item.id), "p.jpg", None).unwrap();
    s.delete_item(&item.id).unwrap();
    // the item is gone but the user's photo survives, demoted to session-level.
    assert!(s.list_items_for_session(&sid).unwrap().is_empty());
    let got = s.get_photo(&p.id).unwrap();
    assert_eq!(got.item_id, None);
    assert_eq!(got.updated_at, 9000);
    assert_eq!(s.count_photos_for_session(&sid).unwrap(), 1);
}
```

- [ ] **Step 2 — demotion helper** (`store/photos.rs`): a private, session-scoped demotion that nulls `item_id` for live photos pointing at a now-tombstoned item in that session. Called **inside** the caller's open transaction (bare statement on `self.conn`, like the other cascade writers):
```rust
impl Store {
    /// Demote live photos whose `item_id` references a now-tombstoned item in
    /// `session_id` to session-level (`item_id := NULL`). Runs INSIDE the
    /// caller's transaction, AFTER the item tombstone. Order-independent: it
    /// keys off `deleted_at IS NOT NULL` on the item, so it demotes exactly the
    /// items just swept (and any earlier-swept item, already idempotent). Plan
    /// 11 D3 — a live photo must never reference a tombstoned item.
    pub(crate) fn demote_photos_of_tombstoned_items(&self, session_id: &str) -> Result<(), CoreError> {
        let now = self.now() as i64;
        self.conn.execute(
            "UPDATE photos SET item_id = NULL, updated_at = ?1
             WHERE session_id = ?2 AND deleted_at IS NULL AND item_id IS NOT NULL
               AND item_id IN (SELECT id FROM items
                               WHERE session_id = ?2 AND deleted_at IS NOT NULL)",
            rusqlite::params![now, session_id],
        )?;
        Ok(())
    }
}
```

- [ ] **Step 3 — wire the four sites:**
  - `delete_session` (`sessions.rs:296–316`): add a fourth cascade UPDATE inside the existing tx — `UPDATE photos SET deleted_at=?1, updated_at=?1 WHERE session_id=?2 AND deleted_at IS NULL` (mirror the items/artifacts lines). **Cascade, not demote** (the session is gone).
  - `finish_session_processed` (`sessions.rs:377–404`): after the item-sweep UPDATE and **before** `mark_session_processed`, call `self.demote_photos_of_tombstoned_items(session_id)?` (same open tx).
  - `clear_authoritative_outputs` (`sessions.rs:429–445`): after the authoritative-item tombstone and **before** `tx.commit()`, call `self.demote_photos_of_tombstoned_items(session_id)?`.
  - `delete_item` (`items.rs:149–159`): it is keyed only by item id, so demote directly — before/after the item tombstone, `UPDATE photos SET item_id=NULL, updated_at=?1 WHERE item_id=?2 AND deleted_at IS NULL`. Wrap the two writes in an `unchecked_transaction()` so the item-tombstone + photo-demote are one op (mirror `delete_session`'s tx discipline). Keep the `changed==0 → NotFound` guard on the item tombstone.
  - Extend the **existing** `delete_session_cascades_to_items_and_artifacts` test (`sessions.rs:731`) or rely on the new `delete_session_cascades_to_photos` — either is fine; do not weaken the existing assertions.

- [ ] **Step 4 — verify:** `nix develop -c cargo test -p murmur-core` green (photos + sessions + items). Confirm the pipeline tests (`pipeline/mod.rs` swap/clear tests) still pass — demotion is a no-op when no photos exist, so they are unaffected.

- [ ] **Step 5 — commit:** `feat(core): photo cascade on session delete + demote-to-session on item sweep (Plan 11 D3)`

---

## Part B — FFI: photo CRUD across UniFFI

### Task 3: `PhotoRef` + `EngineError::Photo` + engine methods + `WalkSession.session_id()`

**Files:** create `crates/ffi/src/photos.rs`; modify `engine.rs` (error variant), `session.rs` (`session_id` getter), `lib.rs` (`pub mod photos;`, re-export `PhotoRef`).

- [ ] **Step 1 — failing tests** (`crates/ffi/src/photos.rs` `mod tests`, using the `SpyStore` + `with_providers` pattern from `vocabulary.rs:77–139`; a store handle is reachable for setup via `MurmurEngine::with_providers(store, …)` — start a session on it first):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{MurmurEngine, Providers};
    use harness::{HarnessError, Memory, MemoryStore, MockProvider};
    use std::sync::{Arc, Mutex as StdMutex};

    // Inline SpyStore + literal Providers — the exact vocabulary.rs:84-108 pattern
    // (no `NullStore`/`providers()` helpers exist in this crate).
    struct SpyStore { saved: StdMutex<Vec<Memory>> }
    impl MemoryStore for SpyStore {
        fn load(&self) -> Result<Memory, HarnessError> { Ok(Memory::default()) }
        fn save(&self, m: &Memory) -> Result<(), HarnessError> { self.saved.lock().unwrap().push(m.clone()); Ok(()) }
    }
    // Takes an already-opened Store so tests can start a session on it first,
    // then hand ownership to the engine (with_providers consumes the Store).
    fn engine_with(store: murmur_core::Store) -> Arc<MurmurEngine> {
        MurmurEngine::with_providers(
            store,
            Memory::default(),
            Arc::new(SpyStore { saved: StdMutex::new(Vec::new()) }),
            Providers {
                live: Arc::new(MockProvider::new(vec![])),
                processing: Arc::new(MockProvider::new(vec![])),
                reflection: Arc::new(MockProvider::new(vec![])),
            },
        )
    }

    #[tokio::test]
    async fn add_list_remove_round_trip() {
        let store = murmur_core::Store::open_in_memory("device-a").unwrap();
        let sid = store.start_session(None).unwrap().id;
        let e = engine_with(store);
        let p = e.add_photo(sid.clone(), None, "a.jpg".into(), Some(42)).unwrap();
        assert_eq!(p.session_id, sid);
        assert_eq!(p.item_id, None);
        assert_eq!(p.filename, "a.jpg");
        assert_eq!(p.captured_at, 42);
        assert_eq!(e.list_photos(sid.clone()).unwrap(), vec![p.clone()]);
        e.remove_photo(p.id.clone()).unwrap();
        assert!(e.list_photos(sid).unwrap().is_empty());
    }

    #[tokio::test]
    async fn add_photo_to_missing_session_is_a_photo_error() {
        let e = engine_with(murmur_core::Store::open_in_memory("device-a").unwrap());
        assert!(matches!(e.add_photo("nope".into(), None, "z.jpg".into(), None), Err(EngineError::Photo(_))));
    }

    #[tokio::test]
    async fn list_live_photo_filenames_feeds_the_sweep() {
        let store = murmur_core::Store::open_in_memory("device-a").unwrap();
        let sid = store.start_session(None).unwrap().id;
        let e = engine_with(store);
        e.add_photo(sid.clone(), None, "keep.jpg".into(), None).unwrap();
        let gone = e.add_photo(sid.clone(), None, "drop.jpg".into(), None).unwrap();
        e.remove_photo(gone.id).unwrap();
        assert_eq!(e.list_live_photo_filenames().unwrap(), vec!["keep.jpg".to_string()]);
    }
}
```
(`WalkSession.session_id()` is exercised by the existing `session.rs` tests: after `begin_walk`, `session.session_id()` returns the id the store recorded — add a one-liner assertion to an existing `begin_walk_wires_a_working_session`-style test.)

- [ ] **Step 2 — `EngineError::Photo`** (`engine.rs`, beside `Memory`):
```rust
/// A photo attachment operation failed (missing/tombstoned session, an
/// item_id not in the session, an empty/duplicate filename, a poisoned lock,
/// or a persistence failure). Recoverable — surface, don't crash. Contains
/// store/validation strings only (never an api key).
#[error("photo error: {0}")]
Photo(String),
```

- [ ] **Step 3 — `PhotoRef` + convert + engine methods** (`crates/ffi/src/photos.rs`, Plan 10 file shape):
```rust
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct PhotoRef {
    pub id: String,
    pub session_id: String,
    pub item_id: Option<String>,
    pub filename: String,       // shell resolves → <Documents>/photos/<filename>
    pub captured_at: u64,
}
fn photo_ref(p: &murmur_core::Photo) -> PhotoRef { /* field copy */ }

#[uniffi::export]
impl MurmurEngine {
    pub fn add_photo(&self, session_id: String, item_id: Option<String>,
                     filename: String, captured_at: Option<u64>) -> Result<PhotoRef, EngineError> {
        let store = self.store.lock().map_err(|_| EngineError::Photo("store lock poisoned".into()))?;
        let p = store.add_photo(&session_id, item_id.as_deref(), &filename, captured_at)
            .map_err(|e| EngineError::Photo(e.to_string()))?;
        Ok(photo_ref(&p))
    }
    pub fn list_photos(&self, session_id: String) -> Result<Vec<PhotoRef>, EngineError> { /* map */ }
    pub fn remove_photo(&self, photo_id: String) -> Result<(), EngineError> { /* map err → Photo */ }
    pub fn list_live_photo_filenames(&self) -> Result<Vec<String>, EngineError> { /* … */ }
}
```
Panic-free (Plan 07 CANON): every `lock()`/store call maps to `EngineError`, never `unwrap`. Add `pub mod photos;` + `pub use photos::PhotoRef;` to `lib.rs`.

- [ ] **Step 4 — `WalkSession.session_id()`** (`session.rs`, in the `#[uniffi::export] impl WalkSession` block):
```rust
/// The store id of this walk's session — so the capture UI can call
/// `engine.add_photo(session_id, …)` during and after the walk (Plan 11 D7).
pub fn session_id(&self) -> String { self.session_id.clone() }
```

- [ ] **Step 5 — verify:** `nix develop -c cargo test -p ffi` green (photos + session + vocabulary), `whisper` feature **off**. Note the generated-binding delta: `MurmurEngine` gains 4 methods, `WalkSession` gains `sessionId()`, a `PhotoRef` record and an `.photo` error case appear — nothing else changes.

- [ ] **Step 6 — commit:** `feat(ffi): photo CRUD on MurmurEngine + WalkSession.session_id (PhotoRef, EngineError::Photo)`

---

## Part C — Swift: capture, gallery, sweep (functional-plain; **sac owns visuals**)

### Task 4: `WalkEngine` photo methods + capture wiring + app-open sweep

> **⚠️ VISUAL DESIGN IS SAC'S** (`meta/CANON.md`, division of labor). This task delivers *functional* wiring only — a capture button that writes bytes and calls FFI, a plain review-gallery, and the reconciling sweep. Every visual decision (capture affordance placement, gallery layout, thumbnails, per-item attach gesture, empty state) gets a `// sac:` comment and is left for sac. Do **not** invent visual direction.

**Files:** modify `Engine/WalkEngine.swift`, `Engine/DemoWalkEngine.swift`, `Engine/MurmurEngine.swift`, `App/AppModel.swift`, the capture/review screens; add a photo capture+gallery view.

- [ ] **Step 1 — regenerate bindings** (needs Task 3 present): from the dev shell, `cd apps/ios && ./build-ffi.sh && ./generate.sh`. Confirm `MurmurCoreFFI` exposes `addPhoto(sessionId:itemId:filename:capturedAt:)`, `listPhotos(sessionId:)`, `removePhoto(photoId:)`, `listLivePhotoFilenames()`, `WalkSession.sessionId()`, a `PhotoRef` struct, and a `.photo` error case.

- [ ] **Step 2 — extend the `WalkEngine` protocol** (`Engine/WalkEngine.swift`), following the Plan 10 vocabulary comment style:
```swift
// Photo attachments (Plan 11). Bytes are the SHELL's responsibility: write the
// file into <Documents>/photos/ FIRST, then call addPhoto(...) with its relative
// filename. Deletion is the reconciling sweep — see sweepPhotoBytes(). Throwing:
// the FFI methods are fallible (missing session, bad item_id, persistence).
func attachPhoto(sessionId: String, itemId: String?, filename: String, capturedAt: UInt64?) throws -> PhotoModel
func listPhotos(sessionId: String) throws -> [PhotoModel]
func removePhoto(photoId: String) throws
func liveLivePhotoFilenames() throws -> [String]   // for the sweep
```
Add a plain `struct PhotoModel { id, sessionId, itemId, filename, capturedAt }` (app-facing mirror of `PhotoRef`), like `DocumentModel`/`CapturedFixture`.

- [ ] **Step 3 — conform both engines:**
  - `DemoWalkEngine`: back it with an in-memory `[PhotoModel]` so the demo gallery works with no backend and no real files (demo capture can attach a bundled placeholder image name). Mirror the Rust semantics loosely (session-level attach, remove).
  - `MurmurEngine` (`#if canImport(MurmurCoreFFI)`): forward each method to the FFI call, mapping `PhotoRef → PhotoModel` and translating the thrown FFI error to the app's error surface. `WalkSession.sessionId()` gives the id for in-walk attach.

- [ ] **Step 4 — `AppModel`** (`App/AppModel.swift`): `private(set) var photos: [PhotoModel] = []`, a `photoError: String?`, and:
```swift
func capturePhoto(image: Data, itemId: String?) {
    // sac: the capture UX (camera vs picker, confirm, where the button lives) is yours.
    let name = "\(UUID().uuidString).jpg"
    do {
        try writePhotoBytes(image, name: name)          // bytes FIRST (Plan 11 D4 write order)
        let p = try engine.attachPhoto(sessionId: currentSessionId, itemId: itemId,
                                       filename: name, capturedAt: nil)
        photos.append(p)
    } catch { photoError = "\(error)" }                  // sac: how errors surface is a design call
}
func removePhoto(_ p: PhotoModel) {
    do { try engine.removePhoto(photoId: p.id); photos.removeAll { $0.id == p.id } }
    catch { photoError = "\(error)" }
    // bytes are reaped by sweepPhotoBytes() on next app-open, not here (D4)
}
func loadPhotos(sessionId: String) { photos = (try? engine.listPhotos(sessionId: sessionId)) ?? [] }

/// Reconciling sweep (Plan 11 D4): delete every file in <Documents>/photos/
/// whose name is NOT in the engine's live set. Idempotent, crash-safe; reaps
/// tombstoned-row bytes AND never-committed capture orphans with one rule.
func sweepPhotoBytes() {
    guard let live = try? Set(engine.liveLivePhotoFilenames()) else { return }
    for file in photoDirContents() where !live.contains(file) { try? deletePhotoFile(file) }
}
```
Call `sweepPhotoBytes()` on app launch **only** (v1, per the Open Questions answer). Do **not** also run it on a background task: a concurrent sweep would race an in-flight capture in the window between the bytes-write and the `add_photo` call — the file exists on disk but its row is not yet live, so it isn't in `list_live_photo_filenames()` and a background sweep would delete the user's just-captured photo. App-open is a quiescent point (no capture in flight), so the sweep is safe there.

- [ ] **Step 5 — capture + gallery view** (functional-plain, `// sac:` throughout): a capture button (during the walk and/or at review) that presents the camera/picker and calls `model.capturePhoto`; a gallery in the document-review screen that shows `model.photos` (load via `model.loadPhotos(sessionId:)`) with a delete affordance. Bare `AsyncImage`/`Image(contentsOfFile:)` grid is acceptable — the app has no photo UI yet; restyle is sac's.

- [ ] **Step 6 — verify (dam, manual, real-core, OUTSIDE the Nix shell):**
  - Demo build (clean-checkout, no FFI dep): `cd apps/ios && xcodegen generate` then `xcodebuild -project SitewalkGallery.xcodeproj -scheme SitewalkGallery -destination 'platform=iOS Simulator,name=iPhone 17' build` — capture/gallery work against `DemoWalkEngine`.
  - Real-core: after `build-ffi.sh` + `generate.sh`, capture a photo in a walk → confirm a file lands in `<Documents>/photos/`, a row in `{db_path}` `photos`, it shows in review; delete it → next launch's sweep removes the file. Not a CI gate.

- [ ] **Step 7 — commit:** `feat(ios): photo capture + gallery + reconciling sweep wired through WalkEngine (visuals: sac handoff)`

---

## Part D — Docs & final review

### Task 5: ROADMAP note + gates + independent whole-artifact review

- [ ] **Step 1 — docs:** in `meta/ROADMAP.md`, move "Photo attachment schema" from "Up Next" to done, noting: `photos` table (v5), optional `item_id` with **demote-on-swap** (never dangles/loses), the **shell-owned bytes + reconciling sweep** contract, FFI CRUD, functional iOS capture/gallery (**visuals sac's**). Name the open follow-ups: **vision analysis** (D5), **document-artifact photo refs / PDF embed** (D5), **live-board per-item `photo_count`** (D6), **cross-device photo sharing** (D8, out of the privacy model as written). Cross-reference this plan.

- [ ] **Step 2 — full hermetic gate** (inside the dev shell; paste real output — exit codes, not grep counts, per the tee/pipefail lesson):
  - `nix develop -c cargo test --workspace`
  - `nix develop -c cargo clippy --workspace --all-targets -- -D warnings`
  - confirm neither compiles the `whisper` feature; iOS demo build is unaffected (no FFI dep in the base `project.yml` — `#if canImport(MurmurCoreFFI)` is false on a clean checkout, so all photo FFI code compiles out).

- [ ] **Step 3 — independent whole-artifact review** (CANON: a **separate agent** from the builder; the final review has caught a real cross-module issue in every plan). Read the diff `migrations → domain → store/photos → sessions/items cascade+demote → ffi/photos → Swift` as one artifact and re-check, recomputing the pinned traces:
  - **Migration safety:** `MIGRATIONS[4]` is append-only (no existing entry edited), transactional, `user_version` 4→5; a v4 DB upgrades by one `CREATE TABLE` + 2 indexes; `open_in_memory_migrates_to_latest` asserts `== MIGRATIONS.len()`.
  - **Cascade (Plan 03 lesson):** `delete_session` tombstones photos in the **same tx** as session/items/artifacts; recompute `delete_session_cascades_to_photos` (all `deleted_at = 5000`, raw rows survive, live filenames empty).
  - **Demotion (D3 — the load-bearing fix):** recompute `finish_swap_demotes_photos_of_swept_items_and_never_loses_them` by hand — I1 swept, A1 kept, P1 untouched (`None`), **P2 demoted `None` with `updated_at=6000`**, count still 2, no photo references a tombstoned item. Confirm the demotion runs **inside** the finish/clear transaction and in `delete_item`'s tx; confirm surviving `manual`/this-run-`authoritative` linkages are kept (`finish_swap_keeps_photos_on_surviving_manual_and_this_run_items`).
  - **File contract (D4):** core touches **no** bytes; deletion is the shell sweep against `list_live_photo_filenames`; the live-unique filename index makes the sweep unambiguous; bytes-first write order means a committed row always has bytes and orphans are reaped.
  - **FFI (Plan 07 CANON):** all methods panic-free (lock/store → `EngineError`), display-copy-free `PhotoRef`, `session_id()` getter added; `EngineError::Photo` carries no api key.
  - **Scope honesty:** vision analysis, doc-artifact photo refs, live-board `photo_count`, and cross-device sharing are named out-of-scope **with seams**, not half-built. No evals touched (correctly — see §Evals).
  - **CI hermeticity:** `cargo test --workspace` needs no `whisper`/model/network/camera; the Swift task is correctly outside the CI gate with the `// sac:` handoff prominent.

- [ ] **Step 4 — commit:** `docs: Plan 11 photo-attachments — ROADMAP note + independent review sign-off`

---

## Worked examples (arithmetic-pinned — reviewers recompute by hand)

**A. Session cascade.** Clock `|| 5000`. Session `S`; items `I1`,`I2`; `P1` (item_id=`NULL`, "p1.jpg"), `P2` (item_id=`I1`, "p2.jpg"). `delete_session(S)` (one tx):
| row | before | after |
|---|---|---|
| S | deleted_at=NULL | **5000** |
| I1, I2 | NULL | **5000, 5000** |
| artifacts(S) | NULL | 5000 |
| P1 | deleted_at=NULL, item_id=NULL | **deleted_at=5000** |
| P2 | deleted_at=NULL, item_id=I1 | **deleted_at=5000** |
`list_photos_for_session(S)` → `[]`. Raw `COUNT(*) WHERE id=P1/P2` → 1 each (tombstone). `list_live_photo_filenames()` → `[]` → shell sweep reaps "p1.jpg","p2.jpg".

**B. Swap-at-finish (the D3 bug, resolved).** Clock `|| 6000`. `S` recording. Live item `I1` (source=live). `P1` (item_id=`NULL`), `P2` (item_id=`I1`). Run extracts authoritative `A1`; `finish_session_processed(S, run_item_ids=[A1])`:
1. item-sweep UPDATE: tombstone items `source IN(live,auth) AND id NOT IN [A1]` → **I1.deleted_at=6000**; A1 kept.
2. `demote_photos_of_tombstoned_items(S)`: photos with `item_id IN (SELECT id FROM items WHERE session_id=S AND deleted_at IS NOT NULL)` = `{I1}` → **P2.item_id := NULL, P2.updated_at := 6000**. P1 skipped (item_id already NULL).
3. mark processed + log usage — same tx, commit.

Final `list_photos_for_session(S)`:
| photo | item_id | updated_at | lost? |
|---|---|---|---|
| P1 | NULL (unchanged) | 1000 | no |
| P2 | **NULL (demoted from I1)** | **6000** | **no** |

Invariant restored: **no live photo references a tombstoned item; no photo lost.** (Contrast the bug: without step 2, `P2.item_id = I1` dangles at a `deleted_at=6000` row.) Re-linking `P2 → A1` is future work (no I1↔A1 identity).

**C. File sweep.** `photos/` on disk = {`a.jpg`,`b.jpg`,`c.jpg`,`orphan.jpg`}. Core live set = {`a.jpg`,`c.jpg`} (`b.jpg` tombstoned via `remove_photo`; `orphan.jpg` was written but the app crashed before `add_photo`). `sweepPhotoBytes()` deletes files ∉ live set → removes `b.jpg`,`orphan.jpg`; keeps `a.jpg`,`c.jpg`. Idempotent (a second run finds nothing to delete).

---

## Migration safety

- **Version bump:** `MIGRATIONS.len()` goes 4 → 5; `user_version` 4 → 5. Append-only (`migrations.rs:5–6`) — no shipped entry is edited.
- **Existing DBs:** a v4 DB runs exactly one new entry (index 4): `CREATE TABLE photos` + two indexes, wrapped by the runner in `BEGIN … PRAGMA user_version=5 … COMMIT` (`migrations.rs:137–141`). **No backfill** — no existing table/row is read or altered, so upgrade is a pure additive DDL; a mid-statement failure rolls the whole entry back and leaves `user_version=4` (the `failed_migration_rolls_back_cleanly` guarantee, `migrations.rs:158–178`).
- **Fresh DBs:** `open_in_memory`/`open` run v1→v5 in order; `open_in_memory_migrates_to_latest` (`store/mod.rs:68`) and `reopen_is_idempotent` (`:77`) both assert `== MIGRATIONS.len()` and keep passing.

## Gates

- `nix develop -c cargo test --workspace` — green, **`whisper` feature off**.
- `nix develop -c cargo clippy --workspace --all-targets -- -D warnings` — clean.
- **iOS demo build unaffected** — the base `project.yml` has no `MurmurCoreFFI` dependency, so all photo FFI code is behind `#if canImport(MurmurCoreFFI)` (false on a clean checkout) and compiles out; `xcodegen generate` + demo `xcodebuild` succeed with zero setup.
- Swift real-core build is a **manual dam check** (not CI), post `build-ffi.sh` + `generate.sh`.

## Evals

**None.** The `crates/evals` suite grades transcript-in → extracted-items/document F0.5; photos are user-captured media with no transcript-derived ground truth and no extraction behavior to score. Photo correctness is fully covered by the hermetic `murmur-core`/`ffi` unit tests (CRUD, cascade, demotion, sweep-feed). No corpus, grader, or runner changes.

## Non-goals

- **Vision-model photo analysis** — `process()` does not read photo bytes; no image tokens, no prompt change (D5). Seam: the `photos` table + `list_photos_for_session` are ready for a future vision pass.
- **Photo references in the document artifact / PDF embedding** — the doc JSON is not extended in v1 (D5); the review gallery is a parallel read path.
- **Live-board per-item `photo_count`** — stays `0`; no new `WalkEvent` (D6). Seam: `count_photos_for_session` + the existing `BoardItem.photo_count` field.
- **Photo sync / cross-device sharing / byte upload** — bytes are local-only forever (privacy). The row is sync-ready; the bytes are not (D8).
- **EXIF ingestion beyond an optional `captured_at`**, thumbnailing, annotation/markup, re-linking a demoted photo to the authoritative successor item, camera/permissions UX polish — none built.
- **Photo visual design** — sac's (per the division of labor).
- **Re-pointing `P2 → A1` across the swap** — impossible without item-identity matching (the model re-extracts fresh UUIDs); demotion is the honest v1 behavior (D3).

## Acceptance criteria

1. `cargo test --workspace` + `cargo clippy --workspace --all-targets -- -D warnings` green with **`whisper` off**; no whisper/model/network/camera dependency; iOS demo build unaffected.
2. Migration v5 adds `photos` (append-only, transactional); a v4 DB upgrades cleanly with no backfill; `open_in_memory_migrates_to_latest` passes at 5.
3. `Store` has photo CRUD: `add_photo` (session-validated, optional item-membership-validated, `captured_at` defaults to `now()`, live-unique filename), `get_photo`, `list_photos_for_session`, `remove_photo` (tombstone), `list_live_photo_filenames`, `count_photos_for_session`.
4. **`delete_session` cascade-tombstones photos** in one tx (Plan 03 lesson); the pinned trace (A) holds.
5. **Demotion (D3):** `finish_session_processed`, `clear_authoritative_outputs`, and `delete_item` demote item-linked photos to session-level (`item_id := NULL`, `updated_at` bumped) in the same tx as the item tombstone; a photo is **never lost** and **never references a tombstoned item**; surviving-item linkages are kept. The pinned trace (B) holds.
6. **File contract (D4):** core touches no bytes; deletion is the shell's reconciling sweep against `list_live_photo_filenames`; bytes-first write order + live-unique filename index make the sweep correct and crash-safe. The pinned trace (C) holds.
7. `MurmurEngine` exposes `add_photo`/`list_photos`/`remove_photo`/`list_live_photo_filenames` (throwing, panic-free, `EngineError::Photo`), `WalkSession` exposes `session_id()`; `PhotoRef` is display-copy-free.
8. The iOS shell captures (bytes-first), lists, and removes photos through `WalkEngine` (demo + real-core), and runs the sweep on app-open; functional-plain with prominent `// sac:` handoffs; builds demo + real-core (manual, dam — not CI).
9. Independent whole-artifact review (separate agent) signs off on the Task 5 Step 3 checklist, recomputing traces A/B/C.

## Open questions (need a call)

- **[dam] `delete_item` semantics on photos:** v1 **demotes** (keep the photo, unlink) rather than deleting — deleting a wrongly-extracted item must not destroy a real photo. Confirm this is the desired product behavior (vs. cascade-deleting the photo with the item).
- **[dam] Byte retention timing:** the sweep runs on app-open (and optionally background). Is app-open frequent enough, or should a `remove_photo` also trigger an immediate opportunistic sweep? (v1: app-open only — simplest, robust.)
- **[dam] `captured_at` source:** v1 lets the shell pass EXIF shot-time or defaults to `now()`. Do we want core to require a real EXIF time, or is `now()` acceptable for v1? (v1: optional, defaults to `now()`.)
- **[dam+sac] Cross-device photo sharing (D8):** bytes are local-only forever in the current privacy model. If sharing a walk's photos across a user's own devices is ever wanted, that is a bytes-transport feature explicitly outside the local-first model as written — named, not scoped.
- **[sac] Capture + gallery visuals & entry points (D6/Task 4):** capture affordance placement (during walk vs review), per-item attach gesture, gallery layout/thumbnails, empty state — sac's. This plan ships a functional placeholder behind `// sac:` markers.
