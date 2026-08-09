/**
 * Phase 1 caller identification.
 *
 * READ THIS BEFORE TRUSTING IT: this is identification with a speed bump, not
 * authentication. The shared app secret ships inside the iOS binary, so anyone
 * willing to unzip an IPA can extract it and mint valid-looking credentials.
 *
 * That is a deliberate, bounded tradeoff. What Phase 1 buys:
 *   - the real `sk-ant-` key is off every device (the actual launch blocker),
 *   - abuse is metered, attributable to an install id, and revocable,
 *   - worst-case loss is bounded by the spend cap rather than by an
 *     attacker's appetite.
 *
 * What it does NOT buy: proof the caller is a genuine, unmodified copy of the
 * app on real Apple hardware. That is App Attest — Phase 2, and it matters
 * most the day the App Store listing goes public, because that is when the
 * binary becomes downloadable by anyone rather than by testers you invited.
 * See services/proxy/README.md and the v1 design doc §4.5.
 */

/**
 * Credential wire format: `jefe.<installId>.<appSecret>`.
 *
 * It rides in as the "API key" because the Rust provider sends
 * `config.api_key` as both `x-api-key` and `Authorization: Bearer` and offers
 * no custom-header seam. Packing both values into that one field is what keeps
 * this a config change instead of an FFI change.
 */
const PREFIX = 'jefe.';

/**
 * Phase 2 wire format: `jefeA.<installId>.<attestToken>~<appSecret>`.
 *
 * Same one-field constraint, one more value to carry. The `~` separator is
 * deliberate rather than another dot: the token is itself dotted
 * (`jat.<payload>.<mac>`), so a pure-dot format would need the parser to know
 * the token's internal shape to find the secret. `~` appears in neither
 * base64url nor an install id, so the boundary is unambiguous no matter how
 * the token is later restructured.
 *
 * `jefe.` credentials keep working. Attestation is additive — a build that
 * predates it must not stop being able to talk to the proxy.
 */
const PREFIX_ATTESTED = 'jefeA.';

/** UUID-ish: hex and dashes, bounded so a huge value can't be used as a KV-key DoS. */
const INSTALL_ID_RE = /^[a-zA-Z0-9-]{8,64}$/;

/** Bounds the token before any crypto touches it. */
const MAX_TOKEN_LENGTH = 512;

export interface Credential {
  installId: string;
  /** Present only on `jefeA.` credentials. Unverified at this layer. */
  attestToken?: string;
}

export type AuthFailure = 'missing' | 'malformed' | 'bad_secret';

export type AuthResult =
  | { ok: true; credential: Credential }
  | { ok: false; reason: AuthFailure };

/** Reads the credential out of either header the Rust provider sends. */
export function readRawCredential(headers: Headers): string | null {
  const xApiKey = headers.get('x-api-key');
  if (xApiKey) return xApiKey;
  const auth = headers.get('authorization');
  if (auth?.startsWith('Bearer ')) return auth.slice('Bearer '.length);
  return null;
}

/**
 * Constant-time string comparison.
 *
 * A plain `===` short-circuits on the first differing byte, which leaks the
 * secret one character at a time to anyone who can measure response latency.
 * Length is compared non-secretly first (it isn't sensitive), then every byte
 * is mixed so the loop cost doesn't depend on where the mismatch is.
 */
function timingSafeEqual(a: string, b: string): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) {
    diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  }
  return diff === 0;
}

export function authenticate(headers: Headers, appSecret: string): AuthResult {
  const raw = readRawCredential(headers);
  if (!raw) return { ok: false, reason: 'missing' };

  // Check the longer prefix first — `jefeA.` also starts with `jefe`, though
  // not with `jefe.`, so order is belt-and-braces rather than load-bearing.
  const attested = raw.startsWith(PREFIX_ATTESTED);
  if (!attested && !raw.startsWith(PREFIX)) return { ok: false, reason: 'malformed' };

  // Split off the prefix, then take the FIRST separator as the id boundary —
  // the secret itself may legitimately contain dots.
  const rest = raw.slice((attested ? PREFIX_ATTESTED : PREFIX).length);
  const sep = rest.indexOf('.');
  if (sep <= 0) return { ok: false, reason: 'malformed' };

  const installId = rest.slice(0, sep);
  let remainder = rest.slice(sep + 1);
  if (!INSTALL_ID_RE.test(installId)) return { ok: false, reason: 'malformed' };

  let attestToken: string | undefined;
  if (attested) {
    const tilde = remainder.indexOf('~');
    if (tilde <= 0) return { ok: false, reason: 'malformed' };
    attestToken = remainder.slice(0, tilde);
    remainder = remainder.slice(tilde + 1);
    if (attestToken.length > MAX_TOKEN_LENGTH) return { ok: false, reason: 'malformed' };
  }

  if (remainder.length === 0) return { ok: false, reason: 'malformed' };
  // The secret is still checked on attested credentials. Attestation adds a
  // layer; it does not replace the one underneath it.
  if (!timingSafeEqual(remainder, appSecret)) return { ok: false, reason: 'bad_secret' };

  return { ok: true, credential: { installId, attestToken } };
}
