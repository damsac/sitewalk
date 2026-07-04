//! Gated real-API eval runner. Runs the synthetic corpus through the real
//! pipeline against the Anthropic API and emits a comparable JSON report.
//!
//! ```sh
//! ANTHROPIC_API_KEY=sk-... nix shell nixpkgs#cargo nixpkgs#rustc -c \
//!     cargo run -p evals --example eval -- --model claude-haiku-4-5 --out report.json
//! ```
//! Never prints the key. Opt-in only — no key → clear error, no run.

use std::sync::Arc;

use evals::corpus::load_corpus;
use evals::report::{render_table, SuiteReport};
use evals::run::run_scenario;
use harness::AnthropicProvider;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => { eprintln!("{e}"); std::process::ExitCode::FAILURE }
    }
}

async fn run() -> Result<(), String> {
    // arg parse: --model, --out, --scenario (repeatable), --fixtures <dir>
    let mut model = "claude-haiku-4-5".to_string();
    let mut out: Option<String> = None;
    let mut only: Vec<String> = Vec::new();
    let mut fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures").to_string();
    let mut argv = std::env::args().skip(1);
    while let Some(a) = argv.next() {
        match a.as_str() {
            "--model" => model = argv.next().ok_or("--model needs a value")?,
            "--out" => out = Some(argv.next().ok_or("--out needs a path")?),
            "--scenario" => only.push(argv.next().ok_or("--scenario needs an id")?),
            "--fixtures" => fixtures = argv.next().ok_or("--fixtures needs a dir")?,
            "-h" | "--help" => return Err("usage: eval [--model M] [--out report.json] [--scenario id]... [--fixtures dir]".into()),
            other => return Err(format!("unexpected arg: {other}")),
        }
    }

    let api_key = std::env::var("ANTHROPIC_API_KEY").ok()
        .filter(|k| !k.trim().is_empty())
        .ok_or("ANTHROPIC_API_KEY is not set — export it to run the real-API eval (key is never printed)")?;

    let mut corpus = load_corpus(&fixtures).map_err(|e| format!("cannot load corpus: {e}"))?;
    if !only.is_empty() {
        corpus.retain(|s| only.contains(&s.id));
        if corpus.is_empty() { return Err("no scenarios matched --scenario".into()); }
    }

    let provider = Arc::new(AnthropicProvider::new(api_key, &model));
    let mut reports = Vec::new();
    for scenario in &corpus {
        eprintln!("running {} ...", scenario.id);
        let report = run_scenario(scenario, provider.clone(), &model).await
            .map_err(|e| format!("{}: {e}", scenario.id))?;
        reports.push(report);
    }
    let suite = SuiteReport::assemble(&model, reports);

    let json = serde_json::to_string_pretty(&suite).map_err(|e| e.to_string())?;
    match &out {
        Some(path) => std::fs::write(path, &json).map_err(|e| format!("cannot write {path}: {e}"))?,
        None => println!("{json}"),
    }
    eprintln!("\n{}", render_table(&suite));
    Ok(())
}
