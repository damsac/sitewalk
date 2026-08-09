/**
 * App Attest — the server half of Phase 2.
 *
 * ## What problem this actually solves
 *
 * `auth.ts` is honest that Phase 1 is identification with a speed bump: the
 * shared app secret ships inside the IPA, so anyone who unzips a build can mint
 * a valid-looking credential and spend against the cap. That is tolerable while
 * the only installs are testers we invited by email. It stops being tolerable
 * the day the App Store listing goes public, because the binary becomes
 * downloadable by anyone.
 *
 * App Attest replaces "knows a secret baked into the app" with "holds a private
 * key the Secure Enclave generated, on real Apple hardware, in a genuine,
 * unmodified copy of *this* app". The key is non-exportable — extracting it
 * from a device is not a matter of effort.
 *
 * ## The shape of the exchange
 *
 *   1. app  → `POST /v1/attest/challenge`   (Phase 1 credential)
 *      srv  → a one-time 32-byte challenge
 *   2. app  generates a Secure Enclave key, attests it over the challenge
 *      app  → `POST /v1/attest/attest`      { keyId, attestation }
 *      srv  verifies the chain to Apple's root, stores the public key
 *   3. app  → `POST /v1/attest/assert`      { keyId, assertion, challenge }
 *      srv  → a short-lived bearer token
 *   4. app  → `POST /v1/messages` carrying that token
 *
 * Steps 1–3 happen once per token lifetime (an hour), not once per message.
 * Asserting on every request would double the round trips on a cellular link
 * at a job site, which is the exact condition this app runs in.
 *
 * ## Why the token, rather than an assertion per message
 *
 * Also a plumbing constraint. The Rust provider sends `config.api_key` as the
 * only caller-controlled header, so `/v1/messages` has nowhere to put an
 * assertion (see `auth.ts`). A compact token fits in the credential string; a
 * CBOR assertion does not.
 *
 * ## Deliberately NOT implemented
 *
 * The attestation `receipt` is parsed out but not sent to Apple's server for
 * risk metrics or key-refresh checks. That is a separate Apple endpoint with
 * its own auth, it is optional, and shipping it half-done would be worse than
 * not shipping it. Tracked as a follow-up.
 */

import {
  bytesField,
  decodeCbor,
  mapField,
  textField,
  type CborValue,
} from './cbor';
import {
  componentSize,
  content,
  children,
  decodeOid,
  ecdsaDerToRaw,
  parseCertificate,
  parseTlv,
  pemToDer,
  type Certificate,
} from './der';

export class AttestError extends Error {
  constructor(
    message: string,
    /** Stable, non-identifying reason code. Safe to log; safe to return. */
    readonly code: string
  ) {
    super(message);
  }
}

/** Apple's certificate extension carrying the attestation nonce. */
const NONCE_EXTENSION_OID = '1.2.840.113635.100.8.2';

/** The 16-byte aaguid Apple stamps into attested credential data. */
const AAGUID_PRODUCTION = 'appattest\0\0\0\0\0\0\0';
const AAGUID_DEVELOPMENT = 'appattestdevelop';

/** Attestations larger than this are not real. Bounds the parser's work. */
const MAX_ATTESTATION_BYTES = 64 * 1024;
const MAX_ASSERTION_BYTES = 8 * 1024;

/** How long a challenge stays claimable. Also KV's floor is 60s. */
export const CHALLENGE_TTL_SECONDS = 300;
/** How long an attestation token is good for before the app must re-assert. */
export const TOKEN_TTL_SECONDS = 60 * 60;
/** Attested keys outlive tokens by a lot — re-attesting is the expensive step. */
const KEY_TTL_SECONDS = 60 * 60 * 24 * 400;

// ------------------------------------------------------------------ bytes

export function toBase64Url(bytes: Uint8Array): string {
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

export function fromBase64(text: string): Uint8Array {
  const normalised = text.replace(/-/g, '+').replace(/_/g, '/');
  let binary: string;
  try {
    binary = atob(normalised);
  } catch {
    throw new AttestError('value is not valid base64', 'malformed_base64');
  }
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
  return out;
}

function equalBytes(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  // `?? 0` never fires — lengths are equal — but keeps the loop branch-free
  // under `noUncheckedIndexedAccess`, which matters for a constant-time compare.
  for (let i = 0; i < a.length; i++) diff |= (a[i] ?? 0) ^ (b[i] ?? 0);
  return diff === 0;
}

/** Copies a view into a standalone buffer before handing it to Web Crypto. */
function copy(bytes: Uint8Array): Uint8Array {
  return new Uint8Array(bytes);
}

async function sha256(...parts: Uint8Array[]): Promise<Uint8Array> {
  const total = parts.reduce((n, p) => n + p.length, 0);
  const joined = new Uint8Array(total);
  let offset = 0;
  for (const part of parts) {
    joined.set(part, offset);
    offset += part.length;
  }
  return new Uint8Array(await crypto.subtle.digest('SHA-256', joined));
}

// ------------------------------------------------------------- signatures

async function importVerifyKey(spki: Uint8Array, curve: string): Promise<CryptoKey> {
  return crypto.subtle.importKey(
    'spki',
    copy(spki),
    { name: 'ECDSA', namedCurve: curve },
    false,
    ['verify']
  );
}

/**
 * Verifies an ECDSA signature over `message`.
 *
 * Note the message is passed UNHASHED — Web Crypto applies the digest itself.
 * Apple's documentation describes the signed value as the SHA-256 "nonce", but
 * that nonce *is* the digest ECDSA computes, so hashing first here would sign
 * the wrong thing (a double hash) and nothing would ever verify.
 */
async function verifyEcdsa(
  key: CryptoKey,
  derSignature: Uint8Array,
  message: Uint8Array,
  hash: string,
  curve: string
): Promise<boolean> {
  const rawSignature = ecdsaDerToRaw(derSignature, componentSize(curve));
  return crypto.subtle.verify(
    { name: 'ECDSA', hash },
    key,
    copy(rawSignature),
    copy(message)
  );
}

// --------------------------------------------------------- authenticator data

export interface AuthenticatorData {
  rpIdHash: Uint8Array;
  flags: number;
  signCount: number;
  aaguid?: Uint8Array;
  credentialId?: Uint8Array;
}

/**
 * Parses WebAuthn authenticator data.
 *
 * Layout: 32-byte rpIdHash, 1 flag byte, 4-byte big-endian counter, then — on
 * attestation only — 16-byte aaguid, a 2-byte credential id length, and the
 * credential id.
 */
export function parseAuthenticatorData(bytes: Uint8Array): AuthenticatorData {
  if (bytes.length < 37) {
    throw new AttestError('authenticator data is too short', 'bad_auth_data');
  }
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const base: AuthenticatorData = {
    rpIdHash: bytes.subarray(0, 32),
    flags: bytes[32] ?? 0,
    signCount: view.getUint32(33),
  };
  if (bytes.length === 37) return base;

  if (bytes.length < 55) {
    throw new AttestError('attested credential data is truncated', 'bad_auth_data');
  }
  const credentialIdLength = view.getUint16(53);
  if (bytes.length < 55 + credentialIdLength) {
    throw new AttestError('credential id runs past the buffer', 'bad_auth_data');
  }
  return {
    ...base,
    aaguid: bytes.subarray(37, 53),
    credentialId: bytes.subarray(55, 55 + credentialIdLength),
  };
}

// ------------------------------------------------------------ chain checks

/** Pulls the attestation nonce out of Apple's certificate extension. */
function nonceFromCertificate(cert: Certificate): Uint8Array {
  const extension = cert.extensions.get(NONCE_EXTENSION_OID);
  if (!extension) {
    throw new AttestError('credential certificate has no nonce extension', 'no_nonce_extension');
  }
  // SEQUENCE { [1] EXPLICIT { OCTET STRING nonce } }
  const outer = parseTlv(extension, 0);
  const tagged = children(extension, outer)[0];
  if (!tagged) throw new AttestError('nonce extension is empty', 'bad_nonce_extension');
  const octets = children(extension, tagged)[0];
  if (!octets || octets.tag !== 0x04) {
    throw new AttestError('nonce extension does not hold an OCTET STRING', 'bad_nonce_extension');
  }
  return content(extension, octets);
}

/**
 * Verifies `leaf → … → root`, where `root` is the pinned Apple certificate.
 *
 * Each certificate must be signed by the next, its issuer name must match that
 * signer's subject, and it must be inside its validity window. The root is
 * trusted because it was configured, not because it is self-signed — checking
 * a self-signature proves nothing about who issued it.
 */
async function verifyChain(chain: Certificate[], root: Certificate, now: Date): Promise<void> {
  const path = [...chain, root];

  for (const cert of path) {
    if (now < cert.notBefore || now > cert.notAfter) {
      throw new AttestError('a certificate in the chain is outside its validity window', 'cert_expired');
    }
  }

  for (let i = 0; i < path.length - 1; i++) {
    const subject = path[i];
    const issuer = path[i + 1];
    if (!subject || !issuer) throw new AttestError('certificate chain is malformed', 'chain_broken');

    if (!equalBytes(subject.issuer, issuer.subject)) {
      throw new AttestError('certificate chain does not link', 'chain_broken');
    }

    const key = await importVerifyKey(issuer.spki, issuer.curve);
    const ok = await verifyEcdsa(
      key,
      subject.signature,
      subject.tbs,
      subject.signatureHash,
      issuer.curve
    );
    if (!ok) {
      throw new AttestError('certificate signature does not verify', 'chain_signature_invalid');
    }
  }
}

// ------------------------------------------------------------- attestation

export interface AttestationConfig {
  /** `TEAMID.bundle.id` — what the device hashed into `rpIdHash`. */
  appId: string;
  /** Apple's App Attest root, PEM encoded. */
  rootCaPem: string;
  /** Whether to accept the development aaguid. False in production. */
  allowDevelopment: boolean;
}

export interface AttestedKey {
  /** SPKI of the attested key, base64. */
  spki: string;
  curve: string;
  /** Highest sign count seen. Replay protection. */
  counter: number;
  /** The install this key belongs to. A key is not transferable. */
  installId: string;
  attestedAt: number;
}

/**
 * Verifies an attestation object and returns the key to remember.
 *
 * Follows Apple's documented order. Every step is a real check — none of them
 * are ceremony, and skipping any one of them makes the rest decorative:
 *
 *  1. chain to Apple's root      — proves Apple issued this key's certificate
 *  2. nonce binds the challenge  — proves this attestation is fresh, not replayed
 *  3. key id is the key's hash   — proves the client named the key it attested
 *  4. rpIdHash matches our appId — proves it is *our* app, not another one
 *  5. counter is zero            — proves the key is brand new
 *  6. aaguid is Apple's          — proves the environment (prod vs development)
 */
export async function verifyAttestation(
  attestationObject: Uint8Array,
  keyId: Uint8Array,
  challenge: Uint8Array,
  config: AttestationConfig,
  now: Date = new Date()
): Promise<{ spki: Uint8Array; curve: string; receipt?: Uint8Array }> {
  if (attestationObject.length > MAX_ATTESTATION_BYTES) {
    throw new AttestError('attestation object is implausibly large', 'too_large');
  }

  let decoded: CborValue;
  try {
    decoded = decodeCbor(attestationObject);
  } catch (error) {
    throw new AttestError(`attestation is not valid CBOR: ${error}`, 'malformed_cbor');
  }

  const fmt = textField(decoded, 'fmt');
  if (fmt !== 'apple-appattest') {
    throw new AttestError(`unexpected attestation format "${fmt}"`, 'bad_format');
  }

  const authData = bytesField(decoded, 'authData');
  const attStmt = mapField(decoded, 'attStmt');
  const x5cRaw = mapField(attStmt, 'x5c');
  if (!Array.isArray(x5cRaw) || x5cRaw.length < 2) {
    throw new AttestError('attestation statement has no certificate chain', 'no_chain');
  }

  let chain: Certificate[];
  let root: Certificate;
  try {
    chain = x5cRaw.map((entry) => {
      if (!(entry instanceof Uint8Array)) throw new Error('x5c entry is not a byte string');
      return parseCertificate(entry);
    });
    root = parseCertificate(pemToDer(config.rootCaPem));
  } catch (error) {
    throw new AttestError(`certificate parse failed: ${error}`, 'bad_certificate');
  }

  // 1. chain
  await verifyChain(chain, root, now);
  const credCert = chain[0];
  if (!credCert) throw new AttestError('attestation has no leaf certificate', 'no_chain');

  // 2. nonce — SHA256(authData ‖ SHA256(challenge)), inside Apple's extension
  const clientDataHash = await sha256(challenge);
  const expectedNonce = await sha256(authData, clientDataHash);
  if (!equalBytes(nonceFromCertificate(credCert), expectedNonce)) {
    throw new AttestError('attestation nonce does not match the challenge', 'nonce_mismatch');
  }

  // 3. key id is the hash of the attested public key
  const publicKeyHash = await sha256(credCert.publicKeyPoint);
  if (!equalBytes(publicKeyHash, keyId)) {
    throw new AttestError('key id is not the hash of the attested key', 'key_id_mismatch');
  }

  const parsed = parseAuthenticatorData(authData);

  // 4. this is our app
  const expectedRpId = await sha256(new TextEncoder().encode(config.appId));
  if (!equalBytes(parsed.rpIdHash, expectedRpId)) {
    throw new AttestError('attestation is for a different app id', 'app_id_mismatch');
  }

  // 5. a freshly generated key has never signed anything
  if (parsed.signCount !== 0) {
    throw new AttestError('attested key has a non-zero counter', 'counter_not_zero');
  }

  // 6. environment
  const aaguid = new TextDecoder('utf-8').decode(parsed.aaguid ?? new Uint8Array());
  const acceptable = config.allowDevelopment
    ? [AAGUID_PRODUCTION, AAGUID_DEVELOPMENT]
    : [AAGUID_PRODUCTION];
  if (!acceptable.includes(aaguid)) {
    // Rejecting the development aaguid in production is the whole reason this
    // is configurable: a development attestation can be produced from a build
    // signed with a development profile, which is a much lower bar.
    throw new AttestError('unexpected attestation environment', 'bad_aaguid');
  }

  if (!parsed.credentialId || !equalBytes(parsed.credentialId, keyId)) {
    throw new AttestError('credential id does not match the key id', 'credential_id_mismatch');
  }

  const receipt = attStmt instanceof Map ? attStmt.get('receipt') : undefined;
  return {
    spki: credCert.spki,
    curve: credCert.curve,
    receipt: receipt instanceof Uint8Array ? receipt : undefined,
  };
}

// --------------------------------------------------------------- assertion

/**
 * Verifies an assertion against a previously attested key.
 *
 * Returns the new counter, which the caller must persist. The counter is the
 * only replay defence once a challenge has been consumed: Secure Enclave
 * increments it on every signature, so a captured assertion replayed later
 * carries a counter that is no longer strictly greater.
 */
export async function verifyAssertion(
  assertionObject: Uint8Array,
  challenge: Uint8Array,
  key: { spki: Uint8Array; curve: string; counter: number },
  appId: string
): Promise<{ counter: number }> {
  if (assertionObject.length > MAX_ASSERTION_BYTES) {
    throw new AttestError('assertion is implausibly large', 'too_large');
  }

  let decoded: CborValue;
  try {
    decoded = decodeCbor(assertionObject);
  } catch (error) {
    throw new AttestError(`assertion is not valid CBOR: ${error}`, 'malformed_cbor');
  }

  const signature = bytesField(decoded, 'signature');
  const authData = bytesField(decoded, 'authenticatorData');

  const clientDataHash = await sha256(challenge);
  const message = new Uint8Array(authData.length + clientDataHash.length);
  message.set(authData, 0);
  message.set(clientDataHash, authData.length);

  const verifyKey = await importVerifyKey(key.spki, key.curve);
  const ok = await verifyEcdsa(verifyKey, signature, message, 'SHA-256', key.curve);
  if (!ok) throw new AttestError('assertion signature does not verify', 'assertion_invalid');

  const parsed = parseAuthenticatorData(authData);

  const expectedRpId = await sha256(new TextEncoder().encode(appId));
  if (!equalBytes(parsed.rpIdHash, expectedRpId)) {
    throw new AttestError('assertion is for a different app id', 'app_id_mismatch');
  }

  if (parsed.signCount <= key.counter) {
    throw new AttestError('assertion counter did not advance', 'counter_replay');
  }

  return { counter: parsed.signCount };
}

// ------------------------------------------------------------------ token

/**
 * A short-lived bearer token, minted after a successful assertion.
 *
 * `jat.<payload>.<mac>`, both base64url. The payload names the install, the
 * key and an expiry; the MAC is HMAC-SHA256 under the app secret. Note this
 * means the token's unforgeability rests on a secret that also ships in the
 * binary — so a token is NOT stronger than Phase 1 on its own. What it is, is
 * a receipt that a real attestation happened within the last hour, which an
 * extracted secret alone cannot produce.
 */
interface TokenPayload {
  i: string;
  k: string;
  e: number;
}

async function macKey(secret: string): Promise<CryptoKey> {
  return crypto.subtle.importKey(
    'raw',
    new TextEncoder().encode(secret),
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['sign', 'verify']
  );
}

export async function mintToken(
  secret: string,
  installId: string,
  keyId: string,
  now: Date = new Date()
): Promise<string> {
  const payload: TokenPayload = {
    i: installId,
    k: keyId,
    e: Math.floor(now.getTime() / 1000) + TOKEN_TTL_SECONDS,
  };
  const encoded = toBase64Url(new TextEncoder().encode(JSON.stringify(payload)));
  const mac = await crypto.subtle.sign('HMAC', await macKey(secret), new TextEncoder().encode(encoded));
  return `jat.${encoded}.${toBase64Url(new Uint8Array(mac))}`;
}

export type TokenFailure =
  | 'malformed'
  | 'bad_signature'
  | 'expired'
  | 'wrong_install';

export type TokenResult =
  | { ok: true; keyId: string }
  | { ok: false; reason: TokenFailure };

export async function verifyToken(
  secret: string,
  token: string,
  installId: string,
  now: Date = new Date()
): Promise<TokenResult> {
  const parts = token.split('.');
  if (parts.length !== 3 || parts[0] !== 'jat') return { ok: false, reason: 'malformed' };
  const encoded = parts[1];
  const mac = parts[2];
  if (!encoded || !mac) return { ok: false, reason: 'malformed' };

  // `crypto.subtle.verify` is constant time, which matters: a `===` here would
  // leak the MAC one byte at a time to anyone who can measure latency.
  let valid: boolean;
  try {
    valid = await crypto.subtle.verify(
      'HMAC',
      await macKey(secret),
      copy(fromBase64(mac)),
      new TextEncoder().encode(encoded)
    );
  } catch {
    return { ok: false, reason: 'malformed' };
  }
  if (!valid) return { ok: false, reason: 'bad_signature' };

  let payload: TokenPayload;
  try {
    payload = JSON.parse(new TextDecoder().decode(fromBase64(encoded)));
  } catch {
    return { ok: false, reason: 'malformed' };
  }
  if (typeof payload.e !== 'number' || typeof payload.i !== 'string') {
    return { ok: false, reason: 'malformed' };
  }
  if (payload.e * 1000 <= now.getTime()) return { ok: false, reason: 'expired' };
  // A token is bound to the install that earned it, so a leaked token cannot
  // be pasted into someone else's credential to dodge their spend cap.
  if (payload.i !== installId) return { ok: false, reason: 'wrong_install' };

  return { ok: true, keyId: payload.k };
}

// --------------------------------------------------------------- KV access

export function challengeKey(challenge: string): string {
  return `attest:chal:${challenge}`;
}

export function attestedKeyKey(keyId: string): string {
  return `attest:key:${keyId}`;
}

export async function issueChallenge(kv: KVNamespace, installId: string): Promise<string> {
  const challenge = toBase64Url(crypto.getRandomValues(new Uint8Array(32)));
  await kv.put(challengeKey(challenge), installId, { expirationTtl: CHALLENGE_TTL_SECONDS });
  return challenge;
}

/**
 * Claims a challenge, which must have been issued to this same install.
 *
 * KV is eventually consistent, so "delete on use" is best-effort rather than a
 * hard single-use guarantee — two requests racing the same challenge inside the
 * replication window can both succeed. That is acceptable here because the
 * challenge is only a freshness bound: replaying an attestation twice in five
 * minutes gets an attacker a second copy of a token they already had. Durable
 * Objects would make it exact, and are the upgrade path if that ever matters.
 */
export async function claimChallenge(
  kv: KVNamespace,
  challenge: string,
  installId: string
): Promise<boolean> {
  const owner = await kv.get(challengeKey(challenge));
  if (owner !== installId) return false;
  await kv.delete(challengeKey(challenge));
  return true;
}

export async function loadAttestedKey(
  kv: KVNamespace,
  keyId: string
): Promise<AttestedKey | null> {
  const raw = await kv.get(attestedKeyKey(keyId));
  if (!raw) return null;
  try {
    return JSON.parse(raw) as AttestedKey;
  } catch {
    return null;
  }
}

export async function saveAttestedKey(
  kv: KVNamespace,
  keyId: string,
  key: AttestedKey
): Promise<void> {
  await kv.put(attestedKeyKey(keyId), JSON.stringify(key), { expirationTtl: KEY_TTL_SECONDS });
}

/** Key ids are base64 of a SHA-256; anything else is not one. */
export function isPlausibleKeyId(keyId: string): boolean {
  return /^[A-Za-z0-9+/_-]{43,44}={0,2}$/.test(keyId);
}

/** Re-exported so the router can decode without importing der.ts. */
export { decodeOid };
