/**
 * Token -> dollars, so the spend cap can be expressed in the unit that
 * actually matters.
 *
 * Rates are USD per million tokens. The three input classes bill differently
 * — full rate, ~1.25x for a cache write, ~0.1x for a cache read — which is
 * exactly why `llm_usage` tracks them separately core-side (migration v8).
 * Collapsing them into one number here would defeat that.
 */

export interface Rate {
  /** USD per 1M input tokens. */
  input: number;
  /** USD per 1M output tokens. */
  output: number;
}

/**
 * Known model rates. Keys are matched by PREFIX, so a dated snapshot like
 * `claude-haiku-4-5-20251001` resolves to the `claude-haiku-4-5` entry.
 */
const RATES: ReadonlyArray<readonly [string, Rate]> = [
  ['claude-haiku-4-5', { input: 1, output: 5 }],
  ['claude-sonnet-4-5', { input: 3, output: 15 }],
  ['claude-sonnet-4-6', { input: 3, output: 15 }],
  ['claude-sonnet-5', { input: 3, output: 15 }],
  ['claude-opus-4-8', { input: 5, output: 25 }],
  ['claude-opus-5', { input: 5, output: 25 }],
];

/**
 * The rate applied when the model string matches nothing known.
 *
 * Deliberately the most expensive entry: an unrecognized model must
 * OVERestimate. Guessing low would let a model we haven't priced slip past the
 * spend cap, which is the one thing the cap exists to prevent. Over-charging a
 * legitimate call only trips the cap early, which is visible and fixable.
 */
const UNKNOWN_MODEL_RATE: Rate = { input: 5, output: 25 };

export function rateFor(model: string): Rate {
  for (const [prefix, rate] of RATES) {
    if (model.startsWith(prefix)) return rate;
  }
  return UNKNOWN_MODEL_RATE;
}

/** The four token classes as the Anthropic API reports them. */
export interface Usage {
  input_tokens?: number;
  output_tokens?: number;
  cache_creation_input_tokens?: number;
  cache_read_input_tokens?: number;
}

const PER_MILLION = 1_000_000;

/** Cache writes bill at ~1.25x the input rate. */
const CACHE_WRITE_MULTIPLIER = 1.25;
/** Cache reads bill at ~0.1x the input rate. */
const CACHE_READ_MULTIPLIER = 0.1;

/**
 * USD cost of one call. Missing fields count as zero — a non-caching response
 * omits the two cache fields entirely, and that must not read as NaN and
 * silently poison the running total.
 */
export function costUsd(model: string, usage: Usage): number {
  const rate = rateFor(model);
  const input = usage.input_tokens ?? 0;
  const output = usage.output_tokens ?? 0;
  const cacheWrite = usage.cache_creation_input_tokens ?? 0;
  const cacheRead = usage.cache_read_input_tokens ?? 0;

  const dollars =
    (input * rate.input +
      cacheWrite * rate.input * CACHE_WRITE_MULTIPLIER +
      cacheRead * rate.input * CACHE_READ_MULTIPLIER +
      output * rate.output) /
    PER_MILLION;

  // Guard against a malformed usage object producing NaN/Infinity, which would
  // make every later cap comparison false and disable the cap entirely.
  return Number.isFinite(dollars) && dollars > 0 ? dollars : 0;
}
