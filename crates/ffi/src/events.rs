//! FFI-facing event/board projections (Plan 07 D2/D3). Thin dictionaries over
//! `murmur-core` domain types — never harness wire types, never
//! `serde_json::Value`.

/// One item on the live/authoritative board. `right` and `photo_count` have
/// no core equivalent yet (photo attachment sync is Deferred 6; `right` is
/// board-chrome text the Swift layer owns) — the projection defaults them so
/// the FFI boundary stays honest about what core actually knows.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct BoardItem {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub right: String,
    pub photo_count: u32,
}

/// A whole-board snapshot per live pass (D3) — not per-item diffs. The
/// live→authoritative swap at `finish()` is just the terminal snapshot this
/// carries.
#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum WalkEvent {
    BoardUpdated { items: Vec<BoardItem> },
}

/// Foreign-implemented listener — `with_foreign`, never `callback_interface`
/// (the boxed-trait-object-as-parameter shape fails to compile under uniffi
/// 0.28+: mozilla/uniffi-rs#2797). Stored/passed as `Arc<dyn WalkEventListener>`.
#[uniffi::export(with_foreign)]
pub trait WalkEventListener: Send + Sync {
    fn on_event(&self, event: WalkEvent);
}
