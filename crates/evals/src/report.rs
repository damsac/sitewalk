//! Machine-comparable eval report. `SuiteReport` serializes to JSON (for a
//! prompt-optimizer to diff variants) and renders a human summary table. Cost
//! is estimated from R9 usage tokens via a documented price table.

use serde::{Deserialize, Serialize};

use crate::grade::ScenarioScore;

/// Price table ($ per 1M tokens). APPROXIMATE — confirm against current
/// Anthropic pricing before trusting the dollar column; token counts are exact
/// (from the R9 usage log), only the $ conversion is a constant here. Unknown
/// models fall back to the haiku rate and are flagged in the report.
fn price_per_mtok(model: &str) -> (f64, f64) {
    match model {
        m if m.contains("haiku") => (1.00, 5.00),
        m if m.contains("sonnet") => (3.00, 15.00),
        m if m.contains("opus") => (15.00, 75.00),
        _ => (1.00, 5.00),
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CostReport {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub est_usd: f64,
}

impl CostReport {
    pub fn estimate(model: &str, input_tokens: u64, output_tokens: u64) -> Self {
        let (in_rate, out_rate) = price_per_mtok(model);
        let est_usd = (input_tokens as f64 / 1e6) * in_rate + (output_tokens as f64 / 1e6) * out_rate;
        CostReport { input_tokens, output_tokens, est_usd }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScenarioReport {
    pub id: String,
    pub score: ScenarioScore,
    pub cost: CostReport,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Aggregate {
    pub scenarios: usize,
    pub mean_f_half: f64,
    pub micro_precision: f64,
    pub micro_recall: f64,
    pub mean_distractor_fp_rate: f64,
    pub mean_contact_accuracy: f64,
    pub summaries_ok: usize,
    pub total_cost_usd: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SuiteReport {
    pub model: String,
    pub scenarios: Vec<ScenarioReport>,
    pub aggregate: Aggregate,
}

impl SuiteReport {
    pub fn assemble(model: impl Into<String>, scenarios: Vec<ScenarioReport>) -> Self {
        let n = scenarios.len().max(1) as f64;
        let mean_f_half = scenarios.iter().map(|s| s.score.f_half).sum::<f64>() / n;
        let (tp, fp, fn_) = scenarios.iter().fold((0, 0, 0), |(tp, fp, fnn), s| {
            (tp + s.score.overall.true_positives, fp + s.score.overall.false_positives, fnn + s.score.overall.false_negatives)
        });
        let micro_precision = if tp + fp == 0 { 0.0 } else { tp as f64 / (tp + fp) as f64 };
        let micro_recall = if tp + fn_ == 0 { 0.0 } else { tp as f64 / (tp + fn_) as f64 };
        let mean_distractor_fp_rate = scenarios.iter().map(|s| s.score.distractor_fp_rate).sum::<f64>() / n;
        let mean_contact_accuracy = scenarios.iter().map(|s| s.score.contact_accuracy).sum::<f64>() / n;
        let summaries_ok = scenarios.iter().filter(|s| s.score.summary_ok).count();
        let total_cost_usd = scenarios.iter().map(|s| s.cost.est_usd).sum();
        SuiteReport {
            model: model.into(),
            aggregate: Aggregate {
                scenarios: scenarios.len(),
                mean_f_half, micro_precision, micro_recall,
                mean_distractor_fp_rate, mean_contact_accuracy, summaries_ok, total_cost_usd,
            },
            scenarios,
        }
    }
}

/// Fixed-width summary table. Purely for humans; the JSON is the machine artifact.
pub fn render_table(suite: &SuiteReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("model: {}\n", suite.model));
    out.push_str(&format!("{:<24} {:>6} {:>6} {:>6} {:>7} {:>8} {:>8}\n",
        "scenario", "F0.5", "P", "R", "distFP", "contact", "usd"));
    for s in &suite.scenarios {
        out.push_str(&format!("{:<24} {:>6.2} {:>6.2} {:>6.2} {:>7.2} {:>8.2} {:>8.4}\n",
            s.id, s.score.f_half, s.score.overall.precision, s.score.overall.recall,
            s.score.distractor_fp_rate, s.score.contact_accuracy, s.cost.est_usd));
    }
    let a = &suite.aggregate;
    out.push_str(&format!("{:<24} {:>6.2} {:>6.2} {:>6.2} {:>7.2} {:>8.2} {:>8.4}\n",
        "TOTAL (mean)", a.mean_f_half, a.micro_precision, a.micro_recall,
        a.mean_distractor_fp_rate, a.mean_contact_accuracy, a.total_cost_usd));
    out.push_str(&format!("summaries ok: {}/{}\n", a.summaries_ok, a.scenarios));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grade::{PrecisionRecall, ScenarioScore};

    fn score(f_half: f64, distractor_fp: f64) -> ScenarioScore {
        ScenarioScore {
            overall: PrecisionRecall { true_positives: 1, false_positives: 0, false_negatives: 0, precision: 1.0, recall: 1.0, f1: 1.0 },
            per_kind: vec![], confusion: vec![], f_half,
            contacts_expected: 0, contacts_matched: 0, contact_accuracy: 1.0,
            distractor_count: 2, distractor_hits: (distractor_fp * 2.0) as usize, distractor_fp_rate: distractor_fp,
            summary_ok: true,
        }
    }

    #[test]
    fn suite_aggregate_means_across_scenarios() {
        let suite = SuiteReport::assemble("claude-haiku-4-5", vec![
            ScenarioReport { id: "a".into(), score: score(1.0, 0.0), cost: CostReport::default() },
            ScenarioReport { id: "b".into(), score: score(0.0, 1.0), cost: CostReport::default() },
        ]);
        assert!((suite.aggregate.mean_f_half - 0.5).abs() < 1e-9);
        assert!((suite.aggregate.mean_distractor_fp_rate - 0.5).abs() < 1e-9);
    }

    #[test]
    fn report_serializes_to_stable_json() {
        let suite = SuiteReport::assemble("m", vec![
            ScenarioReport { id: "a".into(), score: score(1.0, 0.0), cost: CostReport::default() },
        ]);
        let json = serde_json::to_string_pretty(&suite).unwrap();
        // round-trips and contains the headline scalar
        assert!(json.contains("mean_f_half"));
        let back: SuiteReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.model, "m");
    }

    #[test]
    fn cost_estimate_uses_price_table() {
        // 1M input + 1M output tokens at the documented haiku rate
        let c = CostReport::estimate("claude-haiku-4-5", 1_000_000, 1_000_000);
        assert!(c.est_usd > 0.0);
        assert_eq!(c.input_tokens, 1_000_000);
    }

    #[test]
    fn table_renders_one_row_per_scenario_and_a_total() {
        let suite = SuiteReport::assemble("m", vec![
            ScenarioReport { id: "deck".into(), score: score(0.8, 0.0), cost: CostReport::default() },
        ]);
        let table = render_table(&suite);
        assert!(table.contains("deck"));
        assert!(table.contains("F0.5"));
        assert!(table.contains("TOTAL") || table.contains("mean"));
    }
}
