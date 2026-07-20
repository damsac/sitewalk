//! Document-schema CRUD across UniFFI (Plan 19 Stage 7): the seam sac's
//! authoring editor builds on. Engine-keyed, mirroring the vocabulary/items
//! CRUD discipline: lock → mutate → error as a typed `EngineError`, never a
//! panic (Plan 07 CANON). SQLite is the durable store — no separate persist
//! step (unlike vocabulary's memory-file save).
//!
//! `kind`/`fill` stay **String**s across the boundary (deliberate, Plan 19
//! Stage 7): strings let the app send a bad kind and get the exact R6
//! allowlist error back — an enum would make an unknown unrepresentable and
//! the reject path untestable from Swift.

use crate::engine::{EngineError, MurmurEngine};

/// FFI mirror of `murmur_core::SchemaField` (murmur-core stays UniFFI-free —
/// every record lives on this side of the boundary, the `NotesEntry`
/// precedent).
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct SchemaField {
    pub key: String,
    pub kind: String,
    pub label: String,
    pub fill: String,
    pub static_value: Option<String>,
}

/// FFI mirror of `murmur_core::SchemaSection`.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct SchemaSection {
    pub key: String,
    pub kind: String,
    pub label: String,
    pub priced: bool,
    pub fields: Vec<SchemaField>,
}

/// FFI mirror of `murmur_core::DocumentSchema`.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct DocumentSchema {
    /// Empty on save = "create" (core mints a UUIDv7); a built-in's fixed id
    /// or a prior save's id = upsert.
    pub id: String,
    pub kind: String,
    pub label: String,
    pub number_prefix: String,
    pub trade_key: Option<String>,
    pub total_kind: String,
    pub total_label_key: String,
    pub sections: Vec<SchemaSection>,
    pub schema_version: u32,
    pub created_at: u64,
    pub updated_at: u64,
    pub device_id: String,
}

fn field_to_core(f: &SchemaField) -> murmur_core::SchemaField {
    murmur_core::SchemaField {
        key: f.key.clone(),
        kind: f.kind.clone(),
        label: f.label.clone(),
        fill: f.fill.clone(),
        static_value: f.static_value.clone(),
    }
}

fn section_to_core(s: &SchemaSection) -> murmur_core::SchemaSection {
    murmur_core::SchemaSection {
        key: s.key.clone(),
        kind: s.kind.clone(),
        label: s.label.clone(),
        priced: s.priced,
        fields: s.fields.iter().map(field_to_core).collect(),
    }
}

fn schema_to_core(d: &DocumentSchema) -> murmur_core::DocumentSchema {
    murmur_core::DocumentSchema {
        id: d.id.clone(),
        kind: d.kind.clone(),
        label: d.label.clone(),
        number_prefix: d.number_prefix.clone(),
        trade_key: d.trade_key.clone(),
        total_kind: d.total_kind.clone(),
        total_label_key: d.total_label_key.clone(),
        sections: d.sections.iter().map(section_to_core).collect(),
        schema_version: d.schema_version,
        created_at: d.created_at,
        updated_at: d.updated_at,
        device_id: d.device_id.clone(),
    }
}

fn schema_from_core(d: murmur_core::DocumentSchema) -> DocumentSchema {
    DocumentSchema {
        id: d.id,
        kind: d.kind,
        label: d.label,
        number_prefix: d.number_prefix,
        trade_key: d.trade_key,
        total_kind: d.total_kind,
        total_label_key: d.total_label_key,
        sections: d
            .sections
            .into_iter()
            .map(|s| SchemaSection {
                key: s.key,
                kind: s.kind,
                label: s.label,
                priced: s.priced,
                fields: s
                    .fields
                    .into_iter()
                    .map(|f| SchemaField {
                        key: f.key,
                        kind: f.kind,
                        label: f.label,
                        fill: f.fill,
                        static_value: f.static_value,
                    })
                    .collect(),
            })
            .collect(),
        schema_version: d.schema_version,
        created_at: d.created_at,
        updated_at: d.updated_at,
        device_id: d.device_id,
    }
}

impl MurmurEngine {
    fn schema_err(msg: impl Into<String>) -> EngineError {
        EngineError::Schema(msg.into())
    }
}

#[uniffi::export]
impl MurmurEngine {
    /// Live schemas, id order (built-ins first). `Some(trade)` filters to
    /// that trade plus template-agnostic (NULL-trade) schemas; `None`
    /// returns everything live.
    pub fn list_document_schemas(
        &self,
        trade_key: Option<String>,
    ) -> Result<Vec<DocumentSchema>, EngineError> {
        let store = self.store.lock().map_err(|_| Self::schema_err("store lock poisoned"))?;
        let schemas = store
            .list_document_schemas(trade_key.as_deref())
            .map_err(|e| Self::schema_err(e.to_string()))?;
        Ok(schemas.into_iter().map(schema_from_core).collect())
    }

    /// Upsert by id (R6: save-time validation rejects unknown section/field/
    /// fill kinds and ≠1 line_items section BEFORE any write — nothing is
    /// persisted on rejection). Returns the saved schema (with its minted id
    /// and bumped timestamps) so the editor updates in one round-trip.
    pub fn save_document_schema(
        &self,
        schema: DocumentSchema,
    ) -> Result<DocumentSchema, EngineError> {
        let store = self.store.lock().map_err(|_| Self::schema_err("store lock poisoned"))?;
        let saved = store
            .save_document_schema(&schema_to_core(&schema))
            .map_err(|e| Self::schema_err(e.to_string()))?;
        Ok(schema_from_core(saved))
    }

    /// Tombstone. A second remove of the same id errors (the store's
    /// tombstone `NotFound`). A removed BUILT-IN stays removed forever — the
    /// seed guard sees the tombstone on every launch (WE-A).
    pub fn remove_document_schema(&self, id: String) -> Result<(), EngineError> {
        let store = self.store.lock().map_err(|_| Self::schema_err("store lock poisoned"))?;
        store.remove_document_schema(&id).map_err(|e| Self::schema_err(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use harness::{
        CompletionResponse, ContentBlock, HarnessError, Memory, MemoryStore, MockProvider,
        StopReason, Usage,
    };

    use crate::engine::Providers;

    use super::*;

    struct NullMemoryStore;
    impl MemoryStore for NullMemoryStore {
        fn load(&self) -> Result<Memory, HarnessError> {
            Ok(Memory::default())
        }
        fn save(&self, _m: &Memory) -> Result<(), HarnessError> {
            Ok(())
        }
    }

    fn tool_use(name: &str, input: serde_json::Value) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::ToolUse { id: "tu".into(), name: name.into(), input }],
            stop_reason: StopReason::ToolUse,
            usage: Usage { input_tokens: 10, output_tokens: 5 },
        }
    }

    fn end_turn(text: &str) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::Text { text: text.into() }],
            stop_reason: StopReason::EndTurn,
            usage: Usage { input_tokens: 10, output_tokens: 5 },
        }
    }

    fn engine_with(processing: Vec<CompletionResponse>) -> Arc<MurmurEngine> {
        MurmurEngine::with_providers(
            murmur_core::Store::open_in_memory("device-a").unwrap(),
            Memory::default(),
            Arc::new(NullMemoryStore),
            Providers {
                live: Arc::new(MockProvider::new(vec![])),
                processing: Arc::new(MockProvider::new(processing)),
                reflection: Arc::new(MockProvider::new(vec![])),
            },
        )
    }

    fn custom_schema(id: &str, kind: &str) -> DocumentSchema {
        DocumentSchema {
            id: id.into(),
            kind: kind.into(),
            label: kind.into(),
            number_prefix: "HOA".into(),
            trade_key: Some("landscape".into()),
            total_kind: "sum".into(),
            total_label_key: "total".into(),
            sections: vec![SchemaSection {
                key: "line_items".into(),
                kind: "line_items".into(),
                label: "Items".into(),
                priced: false,
                fields: vec![],
            }],
            schema_version: 1,
            created_at: 0,
            updated_at: 0,
            device_id: String::new(),
        }
    }

    #[tokio::test]
    async fn ffi_save_list_remove_round_trip() {
        let e = engine_with(vec![]);
        // The seven built-ins are already listed.
        let landscape = e.list_document_schemas(Some("landscape".into())).unwrap();
        assert_eq!(
            landscape.iter().map(|s| s.kind.as_str()).collect::<Vec<_>>(),
            vec!["estimate", "invoice", "work_order", "report"]
        );

        // Save (empty id mints), re-list, remove, remove again errors.
        let mut schema = custom_schema("", "hoa_addendum");
        schema.sections.push(SchemaSection {
            key: "approvals".into(),
            kind: "filled".into(),
            label: "Approvals".into(),
            priced: false,
            fields: vec![SchemaField {
                key: "hoa_no".into(),
                kind: "text".into(),
                label: "HOA approval #".into(),
                fill: "walk".into(),
                static_value: None,
            }],
        });
        let saved = e.save_document_schema(schema).unwrap();
        assert!(!saved.id.is_empty(), "an empty id minted a fresh UUIDv7");
        assert_eq!(saved.device_id, "device-a");
        let listed = e.list_document_schemas(Some("landscape".into())).unwrap();
        assert_eq!(listed.len(), 5);
        let round = listed.iter().find(|s| s.id == saved.id).unwrap();
        assert_eq!(round, &saved, "full sections round-trip through the boundary");
        assert_eq!(round.sections[1].fields[0].key, "hoa_no");

        e.remove_document_schema(saved.id.clone()).unwrap();
        assert_eq!(e.list_document_schemas(Some("landscape".into())).unwrap().len(), 4);
        assert!(matches!(
            e.remove_document_schema(saved.id),
            Err(EngineError::Schema(_))
        ));
    }

    /// WE-D end-to-end (§6): the exact R6 error surfaces as
    /// `EngineError::Schema` and the schema count is unchanged.
    #[tokio::test]
    async fn ffi_save_rejects_unknown_field_kind_nothing_persisted() {
        let e = engine_with(vec![]);
        let before = e.list_document_schemas(None).unwrap().len();
        let mut bad = custom_schema("custom-bad", "hoa_addendum");
        bad.sections.push(SchemaSection {
            key: "s2".into(),
            kind: "filled".into(),
            label: "S2".into(),
            priced: false,
            fields: vec![SchemaField {
                key: "b".into(),
                kind: "barcode".into(),
                label: "B".into(),
                fill: "walk".into(),
                static_value: None,
            }],
        });
        let err = e.save_document_schema(bad).unwrap_err();
        let EngineError::Schema(msg) = &err else {
            panic!("expected EngineError::Schema, got {err:?}");
        };
        assert!(
            msg.contains(
                "invalid field kind 'barcode'; must be one of: line_items, text, long_text, \
                 currency, quantity, date, static"
            ),
            "WE-D's exact allowlist message, got: {msg}"
        );
        assert_eq!(
            e.list_document_schemas(None).unwrap().len(),
            before,
            "count unchanged — nothing persisted (WE-D)"
        );
        // The same posture for an unknown section kind.
        let mut bad_section = custom_schema("custom-bad2", "hoa_addendum");
        bad_section.sections.push(SchemaSection {
            key: "gallery".into(),
            kind: "gallery".into(),
            label: "Gallery".into(),
            priced: false,
            fields: vec![],
        });
        assert!(matches!(
            e.save_document_schema(bad_section),
            Err(EngineError::Schema(_))
        ));
        assert_eq!(e.list_document_schemas(None).unwrap().len(), before);
    }

    /// Payload parity THROUGH the FFI: a built-in build is unchanged on
    /// every pre-Plan-19 field, plus the additive `fields: []` and today's
    /// prefix.
    #[tokio::test]
    async fn ffi_build_document_unchanged_for_builtins() {
        let e = engine_with(vec![
            tool_use("add_item", serde_json::json!({"kind": "todo", "text": "order lumber"})),
            end_turn("done"),
            tool_use("write_notes", serde_json::json!({"summary": "Lumber ordered."})),
        ]);
        let session = e.clone().begin_walk(None, "landscape".into()).unwrap();
        session.clone().append_transcript("order twelve two by tens".into());
        let sid = session.session_id();
        let _notes = session.finish().await;

        let payload = e.build_document(sid, "work_order".into()).await.unwrap();
        assert_eq!(payload.doc_kind, "work_order");
        assert_eq!(payload.doc_number, 1);
        assert_eq!(payload.total_kind, "sum");
        assert_eq!(payload.total_label_key, "total");
        assert_eq!(payload.static_total_cents, None);
        assert!(!payload.queued);
        assert_eq!(payload.lines.len(), 1);
        assert_eq!(payload.lines[0].title, "order lumber");
        assert_eq!(payload.lines[0].section, None);
        assert!(!payload.lines[0].is_gap);
        // The Plan 19 additive surface, inert for built-ins:
        assert!(payload.fields.is_empty(), "built-ins carry zero authored fields");
        assert_eq!(payload.number_prefix.as_deref(), Some("WO"), "today's prefix, from the row");
    }
}
