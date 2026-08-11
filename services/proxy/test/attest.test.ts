import { beforeAll, describe, expect, it, vi, beforeEach, afterEach } from 'vitest';

import { authenticate } from '../src/auth';
import { decodeCbor, CborError } from '../src/cbor';
import { DerError, ecdsaDerToRaw, parseCertificate, pemToDer } from '../src/der';
import {
  AttestError,
  claimChallenge,
  isPlausibleKeyId,
  issueChallenge,
  mintToken,
  parseAuthenticatorData,
  saveAttestedKey,
  toBase64Url,
  verifyAssertion,
  verifyAttestation,
  verifyToken,
} from '../src/attest';
import worker, { attestMode, type Env } from '../src/index';
import {
  basicConstraintsExtension,
  buildWorld,
  encodeCbor,
  generateKeyPair,
  makeCertificate,
  sha256,
  TEST_APP_ID,
  toPem,
  type AttestWorld,
} from './appattest-fixtures';

const APP_SECRET = 'test-app-secret';
/** The one the app never sees. Distinct from APP_SECRET in every test. */
const TOKEN_SECRET = 'server-only-token-secret';
const INSTALL = 'install-abc-123';

let world: AttestWorld;
beforeAll(async () => {
  world = await buildWorld();
}, 30_000);

function config(overrides: Partial<{ appId: string; allowDevelopment: boolean }> = {}) {
  return {
    appId: overrides.appId ?? TEST_APP_ID,
    rootCaPem: world.rootPem,
    allowDevelopment: overrides.allowDevelopment ?? false,
  };
}

const CHALLENGE = 'a-test-challenge-value';
const challengeBytes = () => new TextEncoder().encode(CHALLENGE);

/** Asserts the failure is ours and carries the code we expect, not a stray crash. */
async function expectCode(promise: Promise<unknown>, code: string) {
  await expect(promise).rejects.toThrow(AttestError);
  await promise.catch((error: unknown) => {
    expect((error as AttestError).code).toBe(code);
  });
}

// ------------------------------------------------------------------- CBOR

describe('cbor', () => {
  it('round-trips the shapes App Attest actually uses', () => {
    const encoded = encodeCbor({
      fmt: 'apple-appattest',
      attStmt: { x5c: [new Uint8Array([1, 2]), new Uint8Array([3])], n: 7 },
      authData: new Uint8Array(300), // forces a 2-byte length header
    });
    const decoded = decodeCbor(encoded) as Map<string, unknown>;
    expect(decoded.get('fmt')).toBe('apple-appattest');
    expect((decoded.get('authData') as Uint8Array).length).toBe(300);
    const attStmt = decoded.get('attStmt') as Map<string, unknown>;
    expect((attStmt.get('x5c') as Uint8Array[])[0]).toEqual(new Uint8Array([1, 2]));
    expect(attStmt.get('n')).toBe(7);
  });

  it('rejects trailing bytes, so a valid prefix cannot smuggle a second reading', () => {
    const encoded = encodeCbor({ a: 1 });
    const padded = new Uint8Array(encoded.length + 1);
    padded.set(encoded);
    expect(() => decodeCbor(padded)).toThrow(CborError);
  });

  it('rejects truncation rather than reading past the end', () => {
    const encoded = encodeCbor({ authData: new Uint8Array(64) });
    expect(() => decodeCbor(encoded.subarray(0, encoded.length - 10))).toThrow(CborError);
  });

  it('rejects duplicate map keys — a classic parser-differential trick', () => {
    // {"a": 1, "a": 2} hand-encoded, since the encoder cannot produce it.
    const blob = new Uint8Array([0xa2, 0x61, 0x61, 0x01, 0x61, 0x61, 0x02]);
    expect(() => decodeCbor(blob)).toThrow(/duplicate/);
  });

  it('rejects indefinite-length items', () => {
    expect(() => decodeCbor(new Uint8Array([0x5f, 0xff]))).toThrow(CborError);
  });
});

// -------------------------------------------------------------------- DER

describe('der', () => {
  it('parses a certificate it just built, including the curve and extensions', async () => {
    const ca = await generateKeyPair('P-384');
    const leaf = await generateKeyPair('P-256');
    const der = await makeCertificate({
      subject: 'leaf',
      issuer: 'ca',
      subjectKey: leaf,
      issuerKey: ca,
      extensions: [['1.2.840.113635.100.8.2', new Uint8Array([0x30, 0x00])]],
    });
    const cert = parseCertificate(der);
    expect(cert.curve).toBe('P-256');
    expect(cert.signatureHash).toBe('SHA-384'); // named by the ISSUER's curve
    expect(cert.extensions.has('1.2.840.113635.100.8.2')).toBe(true);
    expect(cert.publicKeyPoint).toEqual(leaf.point);
    expect(cert.notAfter.getTime()).toBeGreaterThan(Date.now());
  });

  it('rejects trailing bytes after a certificate', async () => {
    const ca = await generateKeyPair('P-256');
    const der = await makeCertificate({ subject: 'a', issuer: 'a', subjectKey: ca, issuerKey: ca });
    const padded = new Uint8Array(der.length + 1);
    padded.set(der);
    expect(() => parseCertificate(padded)).toThrow(DerError);
  });

  it('pads short ECDSA components to full width', () => {
    // r = 0x01 (one byte), s = 0x00ff (leading zero stripped by DER rules).
    const derSig = new Uint8Array([0x30, 0x08, 0x02, 0x01, 0x01, 0x02, 0x03, 0x00, 0xff, 0x00]);
    // Rebuild cleanly: SEQUENCE { INTEGER 1, INTEGER 0x00ff }
    const clean = new Uint8Array([0x30, 0x08, 0x02, 0x01, 0x01, 0x02, 0x03, 0x00, 0x00, 0xff]);
    void derSig;
    const raw = ecdsaDerToRaw(clean, 32);
    expect(raw.length).toBe(64);
    expect(raw[31]).toBe(1);
    expect(raw.subarray(0, 31).every((b) => b === 0)).toBe(true);
    expect(raw[63]).toBe(0xff);
  });

  it('round-trips PEM', async () => {
    const ca = await generateKeyPair('P-256');
    const der = await makeCertificate({ subject: 'a', issuer: 'a', subjectKey: ca, issuerKey: ca });
    expect(pemToDer(toPem(der))).toEqual(der);
  });
});

describe('authenticator data', () => {
  it('rejects anything shorter than the fixed header', () => {
    expect(() => parseAuthenticatorData(new Uint8Array(36))).toThrow(AttestError);
  });

  it('rejects a credential id length that runs past the buffer', () => {
    const bytes = new Uint8Array(55);
    new DataView(bytes.buffer).setUint16(53, 9999);
    expect(() => parseAuthenticatorData(bytes)).toThrow(/past the buffer/);
  });
});

// ------------------------------------------------------------- attestation

describe('verifyAttestation', () => {
  it('accepts a well-formed attestation over the right challenge', async () => {
    const attestation = await world.attestationFor(CHALLENGE);
    const result = await verifyAttestation(attestation, world.keyId, challengeBytes(), config());
    expect(result.curve).toBe('P-256');
    expect(result.spki).toEqual(world.deviceKey.spki);
    expect(result.receipt).toEqual(new Uint8Array([1, 2, 3]));
  });

  it('rejects an attestation for a DIFFERENT challenge — the replay defence', async () => {
    const attestation = await world.attestationFor('some-other-challenge');
    await expectCode(
      verifyAttestation(attestation, world.keyId, challengeBytes(), config()),
      'nonce_mismatch'
    );
  });

  it('rejects a nonce that does not cover authData', async () => {
    // Right challenge, but the nonce is not SHA256(authData ‖ clientDataHash),
    // so authData itself is unauthenticated — this is what lets an attacker
    // swap the counter or aaguid if the check is missing.
    const attestation = await world.attestationFor(CHALLENGE, {
      nonce: await sha256(new TextEncoder().encode(CHALLENGE)),
    });
    await expectCode(
      verifyAttestation(attestation, world.keyId, challengeBytes(), config()),
      'nonce_mismatch'
    );
  });

  it('rejects a chain that does not reach the pinned root', async () => {
    const attestation = await world.attestationFor(CHALLENGE, { rogueIssuer: true });
    await expectCode(
      verifyAttestation(attestation, world.keyId, challengeBytes(), config()),
      'chain_signature_invalid'
    );
  });

  it('rejects an expired certificate', async () => {
    const attestation = await world.attestationFor(CHALLENGE, {
      notAfter: new Date(Date.now() - 60_000),
    });
    await expectCode(
      verifyAttestation(attestation, world.keyId, challengeBytes(), config()),
      'cert_expired'
    );
  });

  it('rejects an attestation for another app id', async () => {
    const attestation = await world.attestationFor(CHALLENGE, { appId: 'ZZZZZ99999.com.someone.else' });
    await expectCode(
      verifyAttestation(attestation, world.keyId, challengeBytes(), config()),
      'app_id_mismatch'
    );
  });

  it('rejects a key that has already signed something', async () => {
    const attestation = await world.attestationFor(CHALLENGE, { counter: 5 });
    await expectCode(
      verifyAttestation(attestation, world.keyId, challengeBytes(), config()),
      'counter_not_zero'
    );
  });

  it('rejects a development attestation in production', async () => {
    const attestation = await world.attestationFor(CHALLENGE, { aaguid: 'appattestdevelop' });
    await expectCode(
      verifyAttestation(attestation, world.keyId, challengeBytes(), config()),
      'bad_aaguid'
    );
  });

  it('accepts a development attestation only when explicitly allowed', async () => {
    const attestation = await world.attestationFor(CHALLENGE, { aaguid: 'appattestdevelop' });
    const result = await verifyAttestation(
      attestation,
      world.keyId,
      challengeBytes(),
      config({ allowDevelopment: true })
    );
    expect(result.curve).toBe('P-256');
  });

  it('rejects a key id that is not the hash of the attested key', async () => {
    const attestation = await world.attestationFor(CHALLENGE);
    await expectCode(
      verifyAttestation(attestation, new Uint8Array(32), challengeBytes(), config()),
      'key_id_mismatch'
    );
  });

  it('rejects a credential id that disagrees with the key id', async () => {
    const attestation = await world.attestationFor(CHALLENGE, {
      credentialId: new Uint8Array(32).fill(7),
    });
    await expectCode(
      verifyAttestation(attestation, world.keyId, challengeBytes(), config()),
      'credential_id_mismatch'
    );
  });

  it('rejects a non-App-Attest format', async () => {
    const blob = encodeCbor({
      fmt: 'packed',
      attStmt: { x5c: [new Uint8Array([1]), new Uint8Array([2])] },
      authData: new Uint8Array(37),
    });
    await expectCode(verifyAttestation(blob, world.keyId, challengeBytes(), config()), 'bad_format');
  });

  it('rejects a chain with no intermediate', async () => {
    const blob = encodeCbor({
      fmt: 'apple-appattest',
      attStmt: { x5c: [new Uint8Array([1])] },
      authData: new Uint8Array(37),
    });
    await expectCode(verifyAttestation(blob, world.keyId, challengeBytes(), config()), 'no_chain');
  });

  it('refuses an implausibly large attestation before parsing it', async () => {
    await expectCode(
      verifyAttestation(new Uint8Array(70_000), world.keyId, challengeBytes(), config()),
      'too_large'
    );
  });

  it('refuses a chain longer than any real one, before parsing a certificate', async () => {
    // Apple sends two. Without a bound, a hostile client picks the number of
    // DER parses and ECDSA verifies the worker performs on its behalf.
    const attestation = await world.attestationFor(CHALLENGE, { padChainTo: 5 });
    await expectCode(
      verifyAttestation(attestation, world.keyId, challengeBytes(), config()),
      'chain_too_long'
    );
  });

  it('lets a chain AT the bound through the length gate', async () => {
    // Four is allowed, so this one gets past the length check and dies later on
    // the padding's broken issuer/subject linkage instead. That distinction is
    // the point: it proves `chain_too_long` above is a bound at 4 and not a
    // blanket refusal of anything the fixture pads.
    const attestation = await world.attestationFor(CHALLENGE, { padChainTo: 4 });
    await expectCode(
      verifyAttestation(attestation, world.keyId, challengeBytes(), config()),
      'chain_broken'
    );
  });

  it('refuses an intermediate that does not claim to be a CA', async () => {
    // Same key, same name, same valid signature over the leaf — the ONLY
    // difference is basicConstraints cA. Without the check, any end-entity
    // certificate Apple ever issued is a usable signer.
    const attestation = await world.attestationFor(CHALLENGE, { nonCaIntermediate: true });
    await expectCode(
      verifyAttestation(attestation, world.keyId, challengeBytes(), config()),
      'issuer_not_ca'
    );
  });
});

// --------------------------------------------------------------- assertion

describe('verifyAssertion', () => {
  const stored = () => ({ spki: world.deviceKey.spki, curve: 'P-256', counter: 0 });

  it('accepts an assertion signed by the attested key', async () => {
    const assertion = await world.assertionFor(CHALLENGE, 1);
    const { counter } = await verifyAssertion(assertion, challengeBytes(), stored(), TEST_APP_ID);
    expect(counter).toBe(1);
  });

  it('rejects an assertion signed by a different key', async () => {
    const other = await generateKeyPair('P-256');
    const assertion = await world.assertionFor(CHALLENGE, 1, { wrongKey: other });
    await expectCode(
      verifyAssertion(assertion, challengeBytes(), stored(), TEST_APP_ID),
      'assertion_invalid'
    );
  });

  it('rejects an assertion over a different challenge', async () => {
    const assertion = await world.assertionFor('another-challenge', 1);
    await expectCode(
      verifyAssertion(assertion, challengeBytes(), stored(), TEST_APP_ID),
      'assertion_invalid'
    );
  });

  it('rejects a replayed assertion whose counter did not advance', async () => {
    const assertion = await world.assertionFor(CHALLENGE, 4);
    const first = await verifyAssertion(assertion, challengeBytes(), stored(), TEST_APP_ID);
    expect(first.counter).toBe(4);
    // Same assertion again, now that the stored counter has caught up.
    await expectCode(
      verifyAssertion(assertion, challengeBytes(), { ...stored(), counter: 4 }, TEST_APP_ID),
      'counter_replay'
    );
  });

  it('rejects an assertion for a different app id', async () => {
    const assertion = await world.assertionFor(CHALLENGE, 1, { appId: 'ZZZZZ99999.com.other' });
    await expectCode(
      verifyAssertion(assertion, challengeBytes(), stored(), TEST_APP_ID),
      'app_id_mismatch'
    );
  });
});

// ------------------------------------------------------------------ token

describe('attestation token', () => {
  const KEY_ID = toBase64Url(new Uint8Array(32).fill(3));
  const OTHER_KEY_ID = toBase64Url(new Uint8Array(32).fill(9));

  /** KV holding one attested key, as enrolment would have left it. */
  async function kvWithKey(installId = INSTALL, keyId = KEY_ID) {
    const kv = fakeKv();
    await saveAttestedKey(kv, keyId, {
      spki: '',
      curve: 'P-256',
      counter: 0,
      installId,
      attestedAt: Date.now(),
    });
    return kv;
  }

  it('round-trips', async () => {
    const kv = await kvWithKey();
    const token = await mintToken(TOKEN_SECRET, INSTALL, KEY_ID);
    expect(await verifyToken(kv, TOKEN_SECRET, token, INSTALL)).toEqual({
      ok: true,
      keyId: KEY_ID,
    });
  });

  it('rejects a token minted under a different secret', async () => {
    const kv = await kvWithKey();
    const token = await mintToken('some-other-secret', INSTALL, KEY_ID);
    expect(await verifyToken(kv, TOKEN_SECRET, token, INSTALL)).toEqual({
      ok: false,
      reason: 'bad_signature',
    });
  });

  it('rejects a token signed with the secret that ships in the app', async () => {
    // The finding this binding exists for. APP_SECRET is extractable from any
    // IPA, so a token signed with it is a token anyone can mint — and it would
    // have sailed through, making every attestation check above unreachable.
    const kv = await kvWithKey();
    const token = await mintToken(APP_SECRET, INSTALL, KEY_ID);
    expect(await verifyToken(kv, TOKEN_SECRET, token, INSTALL)).toEqual({
      ok: false,
      reason: 'bad_signature',
    });
  });

  it('rejects a token naming a key that was never attested', async () => {
    // Correctly signed, correct install, correct expiry — and `k` names
    // nothing. Before the lookup this was a valid token.
    const kv = await kvWithKey();
    const token = await mintToken(TOKEN_SECRET, INSTALL, OTHER_KEY_ID);
    expect(await verifyToken(kv, TOKEN_SECRET, token, INSTALL)).toEqual({
      ok: false,
      reason: 'unknown_key',
    });
  });

  it('rejects a token whose key was attested by a different install', async () => {
    const kv = await kvWithKey('some-other-install');
    const token = await mintToken(TOKEN_SECRET, INSTALL, KEY_ID);
    expect(await verifyToken(kv, TOKEN_SECRET, token, INSTALL)).toEqual({
      ok: false,
      reason: 'key_install_mismatch',
    });
  });

  it('rejects a tampered payload', async () => {
    const kv = await kvWithKey();
    const token = await mintToken(TOKEN_SECRET, INSTALL, KEY_ID);
    const parts = token.split('.');
    const forged = toBase64Url(
      new TextEncoder().encode(JSON.stringify({ i: 'someone-else', k: KEY_ID, e: 2 ** 40 }))
    );
    expect(await verifyToken(kv, TOKEN_SECRET, `jat.${forged}.${parts[2]}`, 'someone-else')).toEqual(
      { ok: false, reason: 'bad_signature' }
    );
  });

  it('rejects an expired token', async () => {
    const kv = await kvWithKey();
    const token = await mintToken(TOKEN_SECRET, INSTALL, KEY_ID, new Date(Date.now() - 7200_000));
    expect(await verifyToken(kv, TOKEN_SECRET, token, INSTALL)).toEqual({
      ok: false,
      reason: 'expired',
    });
  });

  it('refuses a valid token presented by another install', async () => {
    // Without this, one attested device could mint tokens and hand them to
    // every other install id, spreading its spend across everyone's caps.
    const kv = await kvWithKey();
    const token = await mintToken(TOKEN_SECRET, INSTALL, KEY_ID);
    expect(await verifyToken(kv, TOKEN_SECRET, token, 'different-install')).toEqual({
      ok: false,
      reason: 'wrong_install',
    });
  });

  it('rejects structurally broken tokens without throwing', async () => {
    const kv = await kvWithKey();
    for (const bad of ['', 'nope', 'jat.only-two', 'jat.a.b.c', 'xxx.a.b', 'jat.!!!.###']) {
      const result = await verifyToken(kv, TOKEN_SECRET, bad, INSTALL);
      expect(result.ok, bad).toBe(false);
    }
  });

  it('never touches KV for a token it cannot authenticate', async () => {
    // Order matters: the MAC gates the lookup, so an unauthenticated caller
    // cannot use token verification to probe or hammer the keyspace.
    const kv = (await kvWithKey()) as KVNamespace & { store: Map<string, string> };
    const gets = (kv.get as unknown as { mock: { calls: unknown[] } }).mock.calls.length;
    await verifyToken(kv, TOKEN_SECRET, await mintToken(APP_SECRET, INSTALL, KEY_ID), INSTALL);
    expect((kv.get as unknown as { mock: { calls: unknown[] } }).mock.calls.length).toBe(gets);
  });
});

describe('key ids', () => {
  it('accepts base64 of a SHA-256 and rejects the rest', () => {
    expect(isPlausibleKeyId(toBase64Url(new Uint8Array(32)))).toBe(true);
    expect(isPlausibleKeyId('short')).toBe(false);
    expect(isPlausibleKeyId('a'.repeat(500))).toBe(false);
    expect(isPlausibleKeyId('has spaces and $')).toBe(false);
  });
});

// ----------------------------------------------------------- credentials

describe('attested credential format', () => {
  it('parses `jefeA.<install>.<token>~<secret>`', () => {
    const raw = `jefeA.${INSTALL}.jat.abc.def~${APP_SECRET}`;
    const result = authenticate(new Headers({ 'x-api-key': raw }), APP_SECRET);
    expect(result).toEqual({
      ok: true,
      credential: { installId: INSTALL, attestToken: 'jat.abc.def' },
    });
  });

  it('keeps accepting plain `jefe.` credentials', () => {
    // Builds already in testers' hands predate attestation and must keep working.
    const result = authenticate(
      new Headers({ 'x-api-key': `jefe.${INSTALL}.${APP_SECRET}` }),
      APP_SECRET
    );
    expect(result).toEqual({ ok: true, credential: { installId: INSTALL, attestToken: undefined } });
  });

  it('still checks the app secret on an attested credential', () => {
    const raw = `jefeA.${INSTALL}.jat.abc.def~wrong-secret`;
    expect(authenticate(new Headers({ 'x-api-key': raw }), APP_SECRET)).toEqual({
      ok: false,
      reason: 'bad_secret',
    });
  });

  it('rejects an attested credential with no separator', () => {
    const raw = `jefeA.${INSTALL}.${APP_SECRET}`;
    expect(authenticate(new Headers({ 'x-api-key': raw }), APP_SECRET).ok).toBe(false);
  });

  it('rejects an oversized token before any crypto runs', () => {
    const raw = `jefeA.${INSTALL}.${'a'.repeat(600)}~${APP_SECRET}`;
    expect(authenticate(new Headers({ 'x-api-key': raw }), APP_SECRET).ok).toBe(false);
  });
});

// --------------------------------------------------------------- the worker

/** In-memory KV with the `delete` the challenge flow needs. */
function fakeKv() {
  const store = new Map<string, string>();
  return {
    store,
    get: vi.fn(async (k: string) => store.get(k) ?? null),
    put: vi.fn(async (k: string, v: string) => {
      store.set(k, v);
    }),
    delete: vi.fn(async (k: string) => {
      store.delete(k);
    }),
  } as unknown as KVNamespace & { store: Map<string, string> };
}

function makeEnv(overrides: Partial<Env> = {}): Env {
  return {
    ANTHROPIC_API_KEY: 'sk-ant-real',
    APP_SECRET,
    METER: fakeKv(),
    UPSTREAM_BASE_URL: 'https://upstream.test',
    APP_ATTEST_APP_ID: TEST_APP_ID,
    APP_ATTEST_ROOT_CA: world.rootPem,
    ATTEST_TOKEN_SECRET: TOKEN_SECRET,
    ...overrides,
  } as Env;
}

const ctx = {
  waitUntil: () => {},
  passThroughOnException: () => {},
} as unknown as ExecutionContext;

function messagesRequest(credential: string): Request {
  return new Request('https://proxy.test/v1/messages', {
    method: 'POST',
    headers: { 'x-api-key': credential, 'content-type': 'application/json' },
    body: JSON.stringify({ model: 'claude-haiku-4-5', max_tokens: 8, messages: [] }),
  });
}

function attestRequest(path: string, body: unknown, credential?: string): Request {
  return new Request(`https://proxy.test${path}`, {
    method: 'POST',
    headers: {
      'x-api-key': credential ?? `jefe.${INSTALL}.${APP_SECRET}`,
      'content-type': 'application/json',
    },
    body: JSON.stringify(body),
  });
}

describe('attest mode parsing', () => {
  it('treats anything unrecognised as off, so a typo cannot lock the fleet out', () => {
    expect(attestMode({} as Env)).toBe('off');
    expect(attestMode({ ATTEST_MODE: '' } as Env)).toBe('off');
    expect(attestMode({ ATTEST_MODE: 'enfroce' } as Env)).toBe('off');
    expect(attestMode({ ATTEST_MODE: ' Monitor ' } as Env)).toBe('monitor');
    expect(attestMode({ ATTEST_MODE: 'enforce' } as Env)).toBe('enforce');
  });

  it('says so out loud when it does not recognise the value', () => {
    // `off` is also what a CORRECT deployment looks like, so falling back
    // silently means a typo is indistinguishable from working config.
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      expect(attestMode({ ATTEST_MODE: 'enfroce' } as Env)).toBe('off');
      expect(warn).toHaveBeenCalledTimes(1);
      const logged = JSON.parse(String(warn.mock.calls[0]?.[0])) as Record<string, unknown>;
      expect(logged.event).toBe('attest_mode_unrecognised');
      expect(logged.value).toBe('enfroce');
    } finally {
      warn.mockRestore();
    }
  });

  it('stays quiet for the three real modes and for no value at all', () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      for (const mode of ['off', 'monitor', 'enforce', ' Monitor ']) {
        attestMode({ ATTEST_MODE: mode } as Env);
      }
      attestMode({} as Env);
      attestMode({ ATTEST_MODE: '  ' } as Env);
      expect(warn).not.toHaveBeenCalled();
    } finally {
      warn.mockRestore();
    }
  });
});

describe('worker attestation flow', () => {
  beforeEach(() => {
    vi.stubGlobal(
      'fetch',
      vi.fn(
        async () =>
          new Response(JSON.stringify({ usage: { input_tokens: 1, output_tokens: 1 } }), {
            status: 200,
          })
      )
    );
  });
  afterEach(() => vi.unstubAllGlobals());

  /** Walks challenge → attest → assert and returns the minted token. */
  async function enrol(env: Env): Promise<string> {
    const challengeRes = await worker.fetch(
      attestRequest('/v1/attest/challenge', {}),
      env,
      ctx
    );
    expect(challengeRes.status).toBe(200);
    const { challenge } = (await challengeRes.json()) as { challenge: string };

    const keyId = toBase64Url(world.keyId);
    const attestation = await world.attestationFor(challenge);
    const attestRes = await worker.fetch(
      attestRequest('/v1/attest/attest', {
        key_id: keyId,
        challenge,
        attestation: toBase64Url(attestation),
      }),
      env,
      ctx
    );
    expect(await attestRes.text()).toContain('"ok":true');

    const second = await worker.fetch(attestRequest('/v1/attest/challenge', {}), env, ctx);
    const { challenge: challenge2 } = (await second.json()) as { challenge: string };
    const assertion = await world.assertionFor(challenge2, 1);
    const assertRes = await worker.fetch(
      attestRequest('/v1/attest/assert', {
        key_id: keyId,
        challenge: challenge2,
        assertion: toBase64Url(assertion),
      }),
      env,
      ctx
    );
    expect(assertRes.status).toBe(200);
    const { token } = (await assertRes.json()) as { token: string };
    return token;
  }

  it('runs the whole enrolment and then serves an enforced request', async () => {
    const env = makeEnv({ ATTEST_MODE: 'enforce' });
    const token = await enrol(env);
    const res = await worker.fetch(
      messagesRequest(`jefeA.${INSTALL}.${token}~${APP_SECRET}`),
      env,
      ctx
    );
    expect(res.status).toBe(200);
  });

  it('turns away an unattested caller in enforce mode', async () => {
    const env = makeEnv({ ATTEST_MODE: 'enforce' });
    const res = await worker.fetch(messagesRequest(`jefe.${INSTALL}.${APP_SECRET}`), env, ctx);
    expect(res.status).toBe(401);
    expect(await res.text()).toContain('attestation required');
  });

  it('lets an unattested caller through in monitor mode', async () => {
    const env = makeEnv({ ATTEST_MODE: 'monitor' });
    const res = await worker.fetch(messagesRequest(`jefe.${INSTALL}.${APP_SECRET}`), env, ctx);
    expect(res.status).toBe(200);
  });

  it('ignores attestation entirely when off — the deployed default', async () => {
    const env = makeEnv();
    const res = await worker.fetch(messagesRequest(`jefe.${INSTALL}.${APP_SECRET}`), env, ctx);
    expect(res.status).toBe(200);
  });

  it('fails closed when asked to enforce without config', async () => {
    const env = makeEnv({ ATTEST_MODE: 'enforce', APP_ATTEST_ROOT_CA: undefined });
    const res = await worker.fetch(messagesRequest(`jefe.${INSTALL}.${APP_SECRET}`), env, ctx);
    expect(res.status).toBe(500);
  });

  it('fails closed when the token-signing secret is missing', async () => {
    const env = makeEnv({ ATTEST_MODE: 'enforce', ATTEST_TOKEN_SECRET: undefined });
    const res = await worker.fetch(messagesRequest(`jefe.${INSTALL}.${APP_SECRET}`), env, ctx);
    expect(res.status).toBe(500);
  });

  it('fails closed when the token secret is the secret the app already holds', async () => {
    // Configured, complete, and worthless: signing the token with a value that
    // ships in the IPA means anyone can mint one. Refuse to run rather than
    // pretend to check.
    const env = makeEnv({ ATTEST_MODE: 'enforce', ATTEST_TOKEN_SECRET: APP_SECRET });
    const res = await worker.fetch(messagesRequest(`jefe.${INSTALL}.${APP_SECRET}`), env, ctx);
    expect(res.status).toBe(500);
    const enrolment = await worker.fetch(attestRequest('/v1/attest/challenge', {}), env, ctx);
    expect(enrolment.status).toBe(503);
  });

  it('refuses a token forged with the app secret', async () => {
    // The whole finding, end to end: an attacker with the extracted IPA secret
    // mints a well-formed token for an install id of their choosing and names
    // a key that WAS genuinely attested. It must not be enough.
    const env = makeEnv({ ATTEST_MODE: 'enforce' });
    await enrol(env);
    const forged = await mintToken(APP_SECRET, INSTALL, toBase64Url(world.keyId));
    const res = await worker.fetch(
      messagesRequest(`jefeA.${INSTALL}.${forged}~${APP_SECRET}`),
      env,
      ctx
    );
    expect(res.status).toBe(401);
  });

  it('refuses a token naming a key that was never attested', async () => {
    // Minted with the REAL signing secret — so this is what a compromised
    // server-side minting path or a stale token looks like — but `k` points at
    // nothing, which is what makes the attestation load-bearing.
    const env = makeEnv({ ATTEST_MODE: 'enforce' });
    await enrol(env);
    const unattested = toBase64Url(new Uint8Array(32).fill(1));
    const token = await mintToken(TOKEN_SECRET, INSTALL, unattested);
    const res = await worker.fetch(
      messagesRequest(`jefeA.${INSTALL}.${token}~${APP_SECRET}`),
      env,
      ctx
    );
    expect(res.status).toBe(401);
  });

  it('refuses a token for a key another install attested', async () => {
    const env = makeEnv({ ATTEST_MODE: 'enforce' });
    await enrol(env); // key belongs to INSTALL
    const keyId = toBase64Url(world.keyId);
    const token = await mintToken(TOKEN_SECRET, 'squatter-install', keyId);
    const res = await worker.fetch(
      messagesRequest(`jefeA.squatter-install.${token}~${APP_SECRET}`),
      env,
      ctx
    );
    expect(res.status).toBe(401);
  });

  it('refuses a token minted for a different install', async () => {
    const env = makeEnv({ ATTEST_MODE: 'enforce' });
    const token = await enrol(env);
    const res = await worker.fetch(
      messagesRequest(`jefeA.someone-elses-install.${token}~${APP_SECRET}`),
      env,
      ctx
    );
    expect(res.status).toBe(401);
  });

  it('spends a challenge exactly once', async () => {
    const env = makeEnv();
    const res = await worker.fetch(attestRequest('/v1/attest/challenge', {}), env, ctx);
    const { challenge } = (await res.json()) as { challenge: string };
    const keyId = toBase64Url(world.keyId);
    const attestation = toBase64Url(await world.attestationFor(challenge));

    const first = await worker.fetch(
      attestRequest('/v1/attest/attest', { key_id: keyId, challenge, attestation }),
      env,
      ctx
    );
    expect(first.status).toBe(200);

    const replay = await worker.fetch(
      attestRequest('/v1/attest/attest', { key_id: keyId, challenge, attestation }),
      env,
      ctx
    );
    expect(replay.status).toBe(403);
    expect(await replay.text()).toContain('bad_challenge');
  });

  it('refuses a challenge issued to a different install', async () => {
    const env = makeEnv();
    const res = await worker.fetch(attestRequest('/v1/attest/challenge', {}), env, ctx);
    const { challenge } = (await res.json()) as { challenge: string };

    const stolen = await worker.fetch(
      attestRequest(
        '/v1/attest/attest',
        {
          key_id: toBase64Url(world.keyId),
          challenge,
          attestation: toBase64Url(await world.attestationFor(challenge)),
        },
        `jefe.other-install-99.${APP_SECRET}`
      ),
      env,
      ctx
    );
    expect(stolen.status).toBe(403);
  });

  it('refuses to assert against a key another install attested', async () => {
    const env = makeEnv();
    await enrol(env);
    const res = await worker.fetch(
      attestRequest('/v1/attest/challenge', {}, `jefe.other-install-99.${APP_SECRET}`),
      env,
      ctx
    );
    const { challenge } = (await res.json()) as { challenge: string };
    const hijack = await worker.fetch(
      attestRequest(
        '/v1/attest/assert',
        {
          key_id: toBase64Url(world.keyId),
          challenge,
          assertion: toBase64Url(await world.assertionFor(challenge, 9)),
        },
        `jefe.other-install-99.${APP_SECRET}`
      ),
      env,
      ctx
    );
    expect(hijack.status).toBe(403);
    expect(await hijack.text()).toContain('key_install_mismatch');
  });

  it('requires a credential on the attestation routes', async () => {
    const env = makeEnv();
    const res = await worker.fetch(
      new Request('https://proxy.test/v1/attest/challenge', { method: 'POST', body: '{}' }),
      env,
      ctx
    );
    expect(res.status).toBe(401);
  });

  it('reports 503 rather than crashing when attestation is unconfigured', async () => {
    const env = makeEnv({ APP_ATTEST_APP_ID: undefined });
    const res = await worker.fetch(attestRequest('/v1/attest/challenge', {}), env, ctx);
    expect(res.status).toBe(503);
  });

  it('rejects a malformed body without a 500', async () => {
    const env = makeEnv();
    const res = await worker.fetch(
      new Request('https://proxy.test/v1/attest/attest', {
        method: 'POST',
        headers: { 'x-api-key': `jefe.${INSTALL}.${APP_SECRET}` },
        body: 'not json',
      }),
      env,
      ctx
    );
    expect(res.status).toBe(403);
  });
});

describe('challenge storage', () => {
  it('issues, claims once, and refuses a claim by the wrong install', async () => {
    const kv = fakeKv();
    const challenge = await issueChallenge(kv, INSTALL);
    expect(await claimChallenge(kv, challenge, 'someone-else')).toBe(false);
    expect(await claimChallenge(kv, challenge, INSTALL)).toBe(true);
    expect(await claimChallenge(kv, challenge, INSTALL)).toBe(false);
  });

  it('issues distinct challenges', async () => {
    const kv = fakeKv();
    const seen = new Set<string>();
    for (let i = 0; i < 50; i++) seen.add(await issueChallenge(kv, INSTALL));
    expect(seen.size).toBe(50);
  });
});
