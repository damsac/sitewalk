//! Audio-pump e2e (Plan 08 Task 2/8): push_audio -> the dedicated STT pump ->
//! the EXISTING append path -> a live BoardUpdated snapshot + TranscriptCommitted,
//! then finish() flushes the final utterance into the document. Fully hermetic:
//! a `ScriptedDecoder`-backed `SttStream` (no model, no `whisper` feature, no
//! mic). Uses only `ffi::` public API + the `#[doc(hidden)]` audio test
//! constructor, plus the pure `stt`/`murmur-core` seams. Mirrors `bridge_e2e`
//! one path over (audio instead of text).

use std::sync::{Arc, Mutex as StdMutex};

use harness::{
    CompletionResponse, ContentBlock, HarnessError, Memory, MemoryStore, MockProvider, StopReason,
    Usage,
};
use murmur_core::{LiveExtractor, Store};
use stt::{RawSegment, ScriptedDecoder, SttConfig, SttStream};

use ffi::{WalkEvent, WalkEventListener, WalkSession};

struct NullMemoryStore;
impl MemoryStore for NullMemoryStore {
    fn load(&self) -> Result<Memory, HarnessError> {
        Ok(Memory::default())
    }
    fn save(&self, _m: &Memory) -> Result<(), HarnessError> {
        Ok(())
    }
}

struct CollectingListener(StdMutex<Vec<WalkEvent>>);
impl WalkEventListener for CollectingListener {
    fn on_event(&self, event: WalkEvent) {
        self.0.lock().unwrap().push(event);
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

/// 9 s of PCM -> two 5 s/1 s windows (drained in one poll); the final "today"
/// straddles the horizon and is only finalized by end() (the finish() flush).
fn scripted_stt() -> Arc<SttStream> {
    let seg = |cs0: i64, cs1: i64, t: &str| RawSegment { start_cs: cs0, end_cs: cs1, text: t.into(), no_speech_prob: 0.0 };
    let decoder = ScriptedDecoder::new(vec![
        vec![seg(0, 180, "order twelve"), seg(180, 360, "two by tens"), seg(360, 480, "for the")],
        vec![seg(0, 80, "for the"), seg(80, 300, "deck framing"), seg(300, 480, "today")],
        vec![seg(0, 80, "today")], // flush window: only end() decodes this
    ]);
    Arc::new(SttStream::with_decoder(Box::new(decoder), SttConfig::default(), &[]))
}

#[tokio::test]
async fn push_audio_drives_the_pump_the_append_path_and_a_finish_flush() {
    let store = Store::open_in_memory("device-a").unwrap();
    let sid = store.start_session(None).unwrap().id;
    let store = Arc::new(StdMutex::new(store));
    let memory = Arc::new(StdMutex::new(Memory::default()));

    let mut extractor = LiveExtractor::new(
        Arc::new(MockProvider::new(vec![
            tool_use("add_item", serde_json::json!({"kind": "todo", "text": "order lumber"})),
            end_turn("captured"),
        ])),
        store.clone(),
        memory.clone(),
        &sid,
    );
    extractor.min_new_chars = 1;

    let session = WalkSession::new_audio_test_session(
        sid.clone(),
        store.clone(),
        extractor,
        // Processing provider: unused here (we assert on the live board + flush).
        Arc::new(MockProvider::new(vec![])),
        memory,
        Arc::new(NullMemoryStore),
        tokio::runtime::Handle::current(),
        Some("landscape".into()),
        scripted_stt(),
        true, // flush on finish
    );

    let listener = Arc::new(CollectingListener(StdMutex::new(Vec::new())));
    session.clone().set_event_listener(listener.clone());

    // Mic PCM -> the pump wakes, decodes both windows, feeds finalized text
    // through the EXISTING append path and emits a TranscriptCommitted.
    session.clone().push_audio(vec![0.0; 144_000]);

    // Wait for both a committed transcript AND a live board tick.
    let (committed, board) = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            {
                let events = listener.0.lock().unwrap();
                let committed = events.iter().find_map(|e| match e {
                    WalkEvent::TranscriptCommitted { text } => Some(text.clone()),
                    _ => None,
                });
                let board = events.iter().rev().find_map(|e| match e {
                    WalkEvent::BoardUpdated { items } if !items.is_empty() => Some(items.clone()),
                    _ => None,
                });
                if let (Some(c), Some(b)) = (committed, board) {
                    return (c, b);
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("pump did not drive the append path in time");

    assert!(committed.contains("deck framing"), "finalized text committed: {committed:?}");
    assert_eq!(board.len(), 1);
    assert_eq!(board[0].text, "order lumber", "same finalized text drove extraction");

    // finish() flushes the held final utterance into the transcript before
    // processing (D6): "today" is only finalized by end().
    let _ = session.finish().await;
    let transcript = store.lock().unwrap().get_session(&sid).unwrap().transcript;
    assert!(transcript.contains("today"), "finish() flushed the last utterance: {transcript:?}");
}
