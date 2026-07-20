//! `MurmurEngine` + `EngineConfig` + provider routing (Plan 07 D11). The
//! entry point Swift constructs once per app; `begin_walk` (Task 7) hands out
//! per-session `WalkSession` objects.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use harness::{AnthropicProvider, FileMemoryStore, LlmProvider, Memory, MemoryStore};
use murmur_core::Store;

/// Fallible-path errors that cross the FFI boundary as a thrown error rather
/// than a panic (Plan 07 CANON: no panics across FFI). `flat_error` means the
/// Swift side receives the variant plus its `Display` message — no api key is
/// ever in these strings (store/runtime/session errors only).
#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum EngineError {
    /// The on-device store could not be opened (bad path, permissions, corrupt
    /// db). Recoverable by the host — surface, don't crash.
    #[error("failed to open store: {0}")]
    Store(String),
    /// The bridge's tokio runtime could not be started.
    #[error("failed to start the bridge runtime: {0}")]
    Runtime(String),
    /// A walk could not be started (store lock, session insert, template set).
    #[error("failed to begin walk: {0}")]
    BeginWalk(String),
    /// A memory / vocabulary mutation failed (lock poisoned, vocabulary full, or
    /// an empty term). Recoverable by the host — surface, don't crash. Never
    /// contains an api key (memory/vocab strings only).
    #[error("memory error: {0}")]
    Memory(String),
    /// A photo attachment operation failed (missing/tombstoned session, an
    /// item_id not in the session, an empty/duplicate filename, a poisoned lock,
    /// or a persistence failure). Recoverable — surface, don't crash. Contains
    /// store/validation strings only (never an api key).
    #[error("photo error: {0}")]
    Photo(String),
    /// A session-lifecycle operation outside `begin_walk`/`WalkSession` failed
    /// (the app-open zombie sweep, or `retry_failed_sessions`). A poisoned
    /// store lock or a store error — recoverable, surface don't crash. A
    /// still-Failed session after a retry attempt is NOT this variant — it's
    /// simply not counted in `retry_failed_sessions`'s return value.
    #[error("session error: {0}")]
    Session(String),
    /// An on-demand `build_document(kind)` call failed (Plan 13 D1/D8): a
    /// non-`Processed` session, an illegal `kind` for the session's
    /// template, a poisoned lock, or a store error reading back the
    /// artifact. Never a pricing-LLM failure — that degrades to a queued,
    /// unpriced document instead (R7); this variant is validation/store only.
    #[error("document build error: {0}")]
    Document(String),
    /// An item CRUD call (`update_item`/`add_item`/`remove_item`, Plan 16)
    /// failed: a missing/tombstoned item, an unknown `kind`, empty text, a
    /// non-`Processed` session (D3-16 — edits are review-surface only), a
    /// poisoned lock, or a store error. Recoverable — surface, don't crash.
    /// Contains store/validation strings only (never an api key).
    #[error("item error: {0}")]
    Item(String),
    /// A document-schema CRUD call (`list_document_schemas` /
    /// `save_document_schema` / `remove_document_schema`, Plan 19) failed:
    /// R6 save-time validation (unknown section/field/fill kind, ≠1
    /// line_items section, empty kind/label/prefix — nothing persisted), a
    /// missing/tombstoned id, a poisoned lock, or a store error.
    /// Recoverable — surface, don't crash. Contains store/validation strings
    /// only (never an api key).
    #[error("schema error: {0}")]
    Schema(String),
}

/// Config crossing the FFI boundary. `api_key` is an opaque `String` from the
/// iOS Keychain and must NEVER be logged — `Debug` is hand-written (never
/// derived) so it always redacts the key, even if a field is added later.
#[derive(uniffi::Record, Clone)]
pub struct EngineConfig {
    pub db_path: String,
    pub device_id: String,
    pub api_key: String,
    pub base_url: Option<String>,
    pub model_live: String,
    pub model_processing: String,
    pub model_reflection: String,
    /// Absolute path to the bundled whisper GGML model (D5). `None` → the walk
    /// runs text-only (no audio ingest). Not secret: fine to print in `Debug`.
    pub stt_model_path: Option<String>,
    /// DONE flush-vs-speed toggle (D6). `true` (default) flushes the final
    /// buffered utterance through the append path before processing.
    pub stt_flush_on_finish: bool,
    /// Whether whisper may use the GPU (Metal). Device builds: `true`. iOS
    /// SIMULATOR builds MUST pass `false`: Metal on sim hard-crashes (SIGTRAP
    /// in ggml_metal_buffer_set_tensor) instead of degrading (falsified D7
    /// assumption); CPU/BLAS decode on sim is proven working. Swift derives
    /// this from `#if targetEnvironment(simulator)`.
    pub stt_use_gpu: bool,
    /// Per-window RMS-energy pre-gate for STT (see `SttConfig::vad_rms_threshold`).
    /// Default `0.0` = decode everything. Device tuning sweeps this (~0.01) to
    /// suppress construction-noise decodes without dropping speech. Not secret:
    /// fine to print in `Debug`. Swift `sttvad=<float>` launch arg overrides it.
    pub stt_vad_rms_threshold: f32,
    /// no_speech_prob post-check for STT (see `SttConfig::no_speech_prob_threshold`).
    /// Default `0.6`. Higher = keep more borderline segments; lower = drop more
    /// suspected hallucinations. Not secret: fine to print in `Debug`. Swift
    /// `sttnsp=<float>` launch arg overrides it.
    pub stt_no_speech_prob_threshold: f32,
}

impl std::fmt::Debug for EngineConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineConfig")
            .field("db_path", &self.db_path)
            .field("device_id", &self.device_id)
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .field("model_live", &self.model_live)
            .field("model_processing", &self.model_processing)
            .field("model_reflection", &self.model_reflection)
            .field("stt_model_path", &self.stt_model_path)
            .field("stt_flush_on_finish", &self.stt_flush_on_finish)
            .field("stt_use_gpu", &self.stt_use_gpu)
            .field("stt_vad_rms_threshold", &self.stt_vad_rms_threshold)
            .field("stt_no_speech_prob_threshold", &self.stt_no_speech_prob_threshold)
            .finish()
    }
}

/// Three routing purposes (D11): `live` (cheap), `processing` (strong),
/// `reflection` (cheap). One `AnthropicProvider` per distinct (model, key,
/// base_url), `Arc`-deduped across purposes that share a model.
///
/// `pub` (not `pub(crate)`) so `crates/ffi/tests/bridge_e2e.rs` can inject
/// mock providers via `MurmurEngine::with_providers` — never crosses FFI (no
/// `#[uniffi::export]`), so it doesn't affect the generated Swift bindings.
#[doc(hidden)]
pub struct Providers {
    pub live: Arc<dyn LlmProvider>,
    pub processing: Arc<dyn LlmProvider>,
    pub reflection: Arc<dyn LlmProvider>,
}

fn build_providers(config: &EngineConfig) -> Providers {
    let mut cache: HashMap<String, Arc<dyn LlmProvider>> = HashMap::new();
    let mut make = |model: &str| -> Arc<dyn LlmProvider> {
        cache
            .entry(model.to_string())
            .or_insert_with(|| {
                let mut provider = AnthropicProvider::new(config.api_key.clone(), model.to_string());
                if let Some(base) = &config.base_url {
                    provider = provider.with_base_url(base.clone());
                }
                Arc::new(provider) as Arc<dyn LlmProvider>
            })
            .clone()
    };
    Providers {
        live: make(&config.model_live),
        processing: make(&config.model_processing),
        reflection: make(&config.model_reflection),
    }
}

/// The FFI entry point. One per app; `begin_walk` (Task 7) hands out
/// per-session `WalkSession`s.
// Fields are read by `begin_walk` (Task 7), which is deliberately deferred
// out of this task so `cargo test -p ffi engine` compiles standalone.
#[derive(uniffi::Object)]
pub struct MurmurEngine {
    pub(crate) store: Arc<Mutex<Store>>,
    pub(crate) memory: Arc<Mutex<Memory>>,
    pub(crate) memory_store: Arc<dyn MemoryStore>,
    pub(crate) providers: Providers,
    /// Handle used to spawn live-extraction ticks from the SYNC
    /// `append_transcript` export (D7: fire-and-forget — the tick runs off
    /// whatever executor called us, which for a plain sync FFI export is not
    /// guaranteed to be a tokio context). Production owns the `Runtime` that
    /// backs this handle (`_runtime`, kept alive for the engine's lifetime);
    /// tests borrow the `#[tokio::test]` runtime instead of spinning up a
    /// second one.
    pub(crate) runtime_handle: tokio::runtime::Handle,
    /// Bundled whisper model path (D5), passed to `SttStream::with_model` at
    /// `begin_walk` under the `whisper` feature. `None` → text-only walks.
    // Read only by the `whisper`-gated build_stt_stream; the feature-off build
    // ignores it (text-only), so it is intentionally unread there.
    #[cfg_attr(not(feature = "whisper"), allow(dead_code))]
    pub(crate) stt_model_path: Option<String>,
    /// DONE flush toggle (D6), threaded onto each `WalkSession`.
    pub(crate) stt_flush_on_finish: bool,
    /// GPU toggle for the whisper backend (see `EngineConfig::stt_use_gpu`).
    #[cfg_attr(not(feature = "whisper"), allow(dead_code))]
    pub(crate) stt_use_gpu: bool,
    /// RMS pre-gate for the whisper backend (see `EngineConfig::stt_vad_rms_threshold`).
    #[cfg_attr(not(feature = "whisper"), allow(dead_code))]
    pub(crate) stt_vad_rms_threshold: f32,
    /// no_speech_prob post-check (see `EngineConfig::stt_no_speech_prob_threshold`).
    #[cfg_attr(not(feature = "whisper"), allow(dead_code))]
    pub(crate) stt_no_speech_prob_threshold: f32,
    _runtime: Option<Arc<tokio::runtime::Runtime>>,
}

#[uniffi::export]
impl MurmurEngine {
    /// Fallible across FFI (uniffi throwing constructor): opening the store or
    /// starting the runtime can fail on a real device, and a panic here would
    /// crash the host app instead of letting Swift handle it.
    #[uniffi::constructor]
    pub fn new(config: EngineConfig) -> Result<Arc<Self>, EngineError> {
        let store = Store::open(&config.db_path, config.device_id.clone())
            .map_err(|e| EngineError::Store(e.to_string()))?;
        let memory_store: Arc<dyn MemoryStore> =
            Arc::new(FileMemoryStore::new(format!("{}.memory.json", config.db_path)));
        let memory = memory_store.load().unwrap_or_default();
        let providers = build_providers(&config);
        let runtime = Arc::new(
            tokio::runtime::Runtime::new().map_err(|e| EngineError::Runtime(e.to_string()))?,
        );
        let runtime_handle = runtime.handle().clone();
        Ok(Arc::new(MurmurEngine {
            store: Arc::new(Mutex::new(store)),
            memory: Arc::new(Mutex::new(memory)),
            memory_store,
            providers,
            runtime_handle,
            stt_model_path: config.stt_model_path.clone(),
            stt_flush_on_finish: config.stt_flush_on_finish,
            stt_use_gpu: config.stt_use_gpu,
            stt_vad_rms_threshold: config.stt_vad_rms_threshold,
            stt_no_speech_prob_threshold: config.stt_no_speech_prob_threshold,
            _runtime: Some(runtime),
        }))
    }
}

impl MurmurEngine {
    /// Build the per-session `SttStream` for the audio path (D5). Fallible, NOT
    /// panicking: `begin_walk` is a `Result`-returning FFI export (the parallel
    /// fallible-constructor lane has landed), so a bad/corrupt model path
    /// surfaces as `Err` across FFI rather than a host crash. A `None` model
    /// path (or the feature off) yields `Ok(None)` — a text-only walk.
    #[cfg(feature = "whisper")]
    pub(crate) fn build_stt_stream(&self, bias: &[String]) -> Result<Option<Arc<stt::SttStream>>, EngineError> {
        match &self.stt_model_path {
            Some(path) => {
                let cfg = stt::SttConfig {
                    use_gpu: self.stt_use_gpu,
                    vad_rms_threshold: self.stt_vad_rms_threshold,
                    no_speech_prob_threshold: self.stt_no_speech_prob_threshold,
                    ..stt::SttConfig::default()
                };
                let stream = stt::SttStream::with_model(std::path::Path::new(path), cfg, bias)
                // Never print a key here (it isn't in scope, but keep the
                // message store/model-only — Plan 07 R6 redaction posture).
                .map_err(|e| EngineError::BeginWalk(format!("stt model load failed: {e}")))?;
                Ok(Some(Arc::new(stream)))
            }
            None => Ok(None),
        }
    }

    /// Feature-off build: no whisper backend is compiled in, so the walk always
    /// runs text-only regardless of any configured model path.
    #[cfg(not(feature = "whisper"))]
    pub(crate) fn build_stt_stream(&self, _bias: &[String]) -> Result<Option<Arc<stt::SttStream>>, EngineError> {
        Ok(None)
    }
}

impl MurmurEngine {
    /// Test-only constructor injecting mock providers (never crosses FFI —
    /// no `#[uniffi::export]`). Lets unit tests AND the `bridge_e2e`
    /// integration test exercise the bridge without a network provider.
    /// Borrows the calling `#[tokio::test]` runtime rather than spinning up a
    /// second one. `pub`, not `#[cfg(test)]`, because an integration test
    /// binary compiles this crate as an ordinary dependency — `#[cfg(test)]`
    /// items would not exist for it to call.
    #[doc(hidden)]
    pub fn with_providers(
        store: Store,
        memory: Memory,
        memory_store: Arc<dyn MemoryStore>,
        providers: Providers,
    ) -> Arc<Self> {
        Arc::new(MurmurEngine {
            store: Arc::new(Mutex::new(store)),
            memory: Arc::new(Mutex::new(memory)),
            memory_store,
            providers,
            runtime_handle: tokio::runtime::Handle::current(),
            // Mock-provider tests exercise the text path or an injected
            // ScriptedDecoder SttStream (via WalkSession::new_audio_test_session),
            // never a real model — so `None`/`true` defaults are correct and no
            // call site changes (finding 4: with_providers keeps its signature).
            stt_model_path: None,
            stt_flush_on_finish: true,
            stt_use_gpu: true,
            stt_vad_rms_threshold: 0.0,
            stt_no_speech_prob_threshold: 0.6,
            _runtime: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_debug_redacts_the_api_key() {
        let cfg = EngineConfig {
            db_path: ":memory:".into(),
            device_id: "dev".into(),
            api_key: "sk-super-secret".into(),
            base_url: None,
            model_live: "claude-haiku-4-5".into(),
            model_processing: "claude-sonnet-4-5".into(),
            model_reflection: "claude-haiku-4-5".into(),
            stt_model_path: Some("/bundle/ggml-base.en-q5_1.bin".into()),
            stt_flush_on_finish: true,
            stt_use_gpu: true,
            stt_vad_rms_threshold: 0.0,
            stt_no_speech_prob_threshold: 0.6,
        };
        let printed = format!("{cfg:?}");
        assert!(!printed.contains("sk-super-secret"), "api key must never be printable");
        // The new STT fields are not secret — they SHOULD print.
        assert!(printed.contains("ggml-base.en-q5_1.bin"), "model path is fine to print");
        assert!(printed.contains("stt_flush_on_finish"));
        assert!(printed.contains("stt_use_gpu"), "gpu knob is plumbed and printable");
        assert!(printed.contains("stt_vad_rms_threshold"), "vad knob is plumbed and printable");
        assert!(
            printed.contains("stt_no_speech_prob_threshold"),
            "no_speech_prob knob is plumbed and printable"
        );
    }

    #[test]
    fn stt_defaults_are_sane() {
        // A config with no STT model path builds providers normally — the STT
        // fields are additive and don't disturb the existing provider wiring.
        let cfg = EngineConfig {
            db_path: ":memory:".into(),
            device_id: "dev".into(),
            api_key: "sk-test".into(),
            base_url: None,
            model_live: "claude-haiku-4-5".into(),
            model_processing: "claude-sonnet-4-5".into(),
            model_reflection: "claude-haiku-4-5".into(),
            stt_model_path: None,
            stt_flush_on_finish: true,
            stt_use_gpu: true,
            stt_vad_rms_threshold: 0.0,
            stt_no_speech_prob_threshold: 0.6,
        };
        let providers = build_providers(&cfg);
        assert!(Arc::ptr_eq(&providers.live, &providers.reflection));
    }

    #[test]
    fn stt_threshold_knobs_thread_from_config_onto_the_engine() {
        // The two noise-gate knobs must survive the EngineConfig -> MurmurEngine
        // hop so build_stt_stream can hand them to SttConfig (device tuning).
        // Non-default values prove it's real plumbing, not a defaulted field.
        let cfg = EngineConfig {
            db_path: ":memory:".into(),
            device_id: "dev".into(),
            api_key: "sk-test".into(),
            base_url: None,
            model_live: "claude-haiku-4-5".into(),
            model_processing: "claude-sonnet-4-5".into(),
            model_reflection: "claude-haiku-4-5".into(),
            stt_model_path: None,
            stt_flush_on_finish: true,
            stt_use_gpu: true,
            stt_vad_rms_threshold: 0.01,
            stt_no_speech_prob_threshold: 0.42,
        };
        let engine = MurmurEngine::new(cfg).expect("engine construction with :memory: store");
        assert_eq!(engine.stt_vad_rms_threshold, 0.01, "vad threshold threaded onto the engine");
        assert_eq!(
            engine.stt_no_speech_prob_threshold, 0.42,
            "no_speech_prob threshold threaded onto the engine"
        );
    }

    #[test]
    fn new_returns_err_instead_of_panicking_on_unopenable_db_path() {
        // A path under a directory that does not exist can't be opened. The
        // constructor must surface this as EngineError (thrown across FFI),
        // never panic (which would crash the host app).
        let cfg = EngineConfig {
            db_path: "/no-such-dir-xyz-9d3f/murmur.db".into(),
            device_id: "dev".into(),
            api_key: "sk-test".into(),
            base_url: None,
            model_live: "claude-haiku-4-5".into(),
            model_processing: "claude-sonnet-4-5".into(),
            model_reflection: "claude-haiku-4-5".into(),
            stt_model_path: None,
            stt_flush_on_finish: true,
            stt_use_gpu: true,
            stt_vad_rms_threshold: 0.0,
            stt_no_speech_prob_threshold: 0.6,
        };
        assert!(matches!(MurmurEngine::new(cfg), Err(EngineError::Store(_))));
    }

    #[test]
    fn providers_dedupe_by_model() {
        let cfg = EngineConfig {
            db_path: ":memory:".into(),
            device_id: "dev".into(),
            api_key: "sk-test".into(),
            base_url: None,
            model_live: "claude-haiku-4-5".into(),
            model_processing: "claude-sonnet-4-5".into(),
            model_reflection: "claude-haiku-4-5".into(),
            stt_model_path: None,
            stt_flush_on_finish: true,
            stt_use_gpu: true,
            stt_vad_rms_threshold: 0.0,
            stt_no_speech_prob_threshold: 0.6,
        };
        let providers = build_providers(&cfg);
        assert!(Arc::ptr_eq(&providers.live, &providers.reflection), "same model shares one Arc");
        assert!(!Arc::ptr_eq(&providers.live, &providers.processing));
    }
}
