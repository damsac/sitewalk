//! Hands-on CLI for the processing pipeline: feed a transcript through
//! start → append → end → process, print what was extracted, then let the
//! reflection coordinator run if it wants to.
//!
//! ```sh
//! ANTHROPIC_API_KEY=sk-... nix shell nixpkgs#cargo nixpkgs#rustc -c \
//!     cargo run -p murmur-core --example walk -- transcript.txt
//! ```
//!
//! Usage: `walk <transcript-file> [--db <path>]`
//! - `<transcript-file>` — path to a plain-text transcript, or `-` for stdin.
//! - `--db <path>` — persistent database; repeated runs accumulate. Defaults
//!   to a fresh temp file (its path is printed). Memory persists in a
//!   `<db>.memory.json` file next to the database.

use std::io::Read;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use harness::{AnthropicProvider, FileMemoryStore, Memory, MemoryStore};
use murmur_core::{NewJob, ReflectionCoordinator, SessionProcessor, Store};

const MODEL: &str = "claude-haiku-4-5";

struct Args {
    transcript_path: String,
    db_path: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut transcript_path = None;
    let mut db_path = None;
    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--db" => {
                db_path = Some(argv.next().ok_or("--db requires a path")?);
            }
            "--help" | "-h" => return Err("help".into()),
            _ if transcript_path.is_none() => transcript_path = Some(arg),
            other => return Err(format!("unexpected argument: {other}")),
        }
    }
    Ok(Args {
        transcript_path: transcript_path.ok_or("missing <transcript-file>")?,
        db_path,
    })
}

fn read_transcript(path: &str) -> std::io::Result<String> {
    if path == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        Ok(buf)
    } else {
        std::fs::read_to_string(path)
    }
}

fn print_memory(memory: &Memory) {
    if memory.sections.is_empty() {
        println!("  (memory is empty)");
        return;
    }
    for (section, entries) in &memory.sections {
        println!("  [{section}]");
        for entry in entries {
            println!("    - {}", entry.text);
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), String> {
    let args = parse_args().map_err(|e| {
        format!("{e}\n\nUsage: walk <transcript-file> [--db <path>]  (use - for stdin)")
    })?;

    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .ok()
        .filter(|k| !k.trim().is_empty())
        .ok_or_else(|| {
            "ANTHROPIC_API_KEY is not set.\n\
             Export your Anthropic API key to run the pipeline against the real API:\n\
             ANTHROPIC_API_KEY=sk-... cargo run -p murmur-core --example walk -- transcript.txt"
                .to_string()
        })?;

    let transcript = read_transcript(&args.transcript_path)
        .map_err(|e| format!("cannot read {}: {e}", args.transcript_path))?;

    // Resolve the db path: --db persists across runs, default is a temp file.
    let db_path = match &args.db_path {
        Some(p) => p.clone(),
        None => {
            let p = std::env::temp_dir()
                .join(format!("murmur-walk-{}.db", std::process::id()))
                .to_string_lossy()
                .into_owned();
            println!("db: {p} (temp — pass --db <path> to persist)");
            p
        }
    };
    let db_is_fresh = !std::path::Path::new(&db_path).exists();
    let store = Store::open(&db_path, "walk-cli").map_err(|e| format!("cannot open db: {e}"))?;

    // Memory lives next to the db so it persists across runs with --db.
    let memory_store = Arc::new(FileMemoryStore::new(format!("{db_path}.memory.json")));
    let memory = memory_store.load().map_err(|e| format!("cannot load memory: {e}"))?;

    if db_is_fresh {
        let job = store
            .create_job(NewJob { name: "CLI walk".into(), ..Default::default() })
            .map_err(|e| format!("cannot create job: {e}"))?;
        println!("created job: {} ({})", job.name, job.id);
    }
    let job_id = store
        .list_jobs()
        .map_err(|e| format!("cannot list jobs: {e}"))?
        .first()
        .map(|j| j.id.clone());

    // Record the session.
    let session = store
        .start_session(job_id.as_deref())
        .map_err(|e| format!("cannot start session: {e}"))?;
    store
        .append_transcript(&session.id, &transcript)
        .map_err(|e| format!("cannot append transcript: {e}"))?;
    store
        .end_and_record_session(&session.id)
        .map_err(|e| format!("cannot end session: {e}"))?;
    println!("session: {}", session.id);

    // Process it.
    let mut provider = AnthropicProvider::new(api_key, MODEL);
    if let Ok(base) = std::env::var("ANTHROPIC_BASE_URL") {
        if !base.trim().is_empty() {
            provider = provider.with_base_url(base);
        }
    }
    let provider = Arc::new(provider);
    let store = Arc::new(Mutex::new(store));
    let memory = Arc::new(Mutex::new(memory));
    let processor = SessionProcessor::new(
        provider.clone(),
        store.clone(),
        memory.clone(),
        memory_store.clone(),
    );
    println!("processing with {MODEL}...\n");
    let outcome = processor
        .process(&session.id)
        .await
        .map_err(|e| format!("processing failed: {e}"))?;

    // Print what came out.
    {
        let store = store.lock().map_err(|_| "store lock poisoned".to_string())?;
        let items = store
            .list_items_for_session(&session.id)
            .map_err(|e| format!("cannot list items: {e}"))?;
        println!("items ({}):", items.len());
        for item in &items {
            println!("  {}: {}", item.kind, item.text);
        }

        let contacts = store.list_contacts().map_err(|e| format!("cannot list contacts: {e}"))?;
        println!("contacts ({}):", contacts.len());
        for c in &contacts {
            let trade = c.trade.as_deref().unwrap_or("no trade");
            println!("  {} ({trade})", c.name);
        }

        let artifacts = store
            .list_artifacts_for_session(&session.id)
            .map_err(|e| format!("cannot list artifacts: {e}"))?;
        for a in &artifacts {
            println!("report: {}\n{}", a.title, a.body);
        }
    }

    println!(
        "\nsummary: {}",
        outcome.session.summary.as_deref().unwrap_or("(none)")
    );
    let (input_total, output_total) = store
        .lock()
        .map_err(|_| "store lock poisoned".to_string())?
        .usage_totals()
        .map_err(|e| format!("cannot read usage totals: {e}"))?;
    println!("usage totals: {input_total} input / {output_total} output tokens");

    // Reflection: runs only when cadence + activity warrant it.
    let coordinator = ReflectionCoordinator::new(provider, store, memory.clone(), memory_store);
    match coordinator.maybe_reflect().await {
        Ok(Some(churn)) => {
            println!("\nreflection ran (churn {churn:.2}); memory is now:");
            let memory = memory.lock().map_err(|_| "memory lock poisoned".to_string())?;
            print_memory(&memory);
        }
        Ok(None) => println!("\nreflection skipped (cadence not due or no activity)"),
        Err(e) => println!("\nreflection failed: {e}"),
    }

    Ok(())
}
