/**
 * Builds synthetic App Attest material — a real certificate chain, real ECDSA
 * signatures, real CBOR.
 *
 * ## Why this exists rather than a captured fixture
 *
 * A verifier you cannot make FAIL is not tested. A recorded attestation from a
 * real device proves one happy path and nothing else: you cannot corrupt one
 * field of it and re-sign, so every negative test degenerates into "garbage in,
 * error out", which would also pass against a verifier that rejects everything.
 *
 * Holding the private keys means each test can change exactly one thing — the
 * challenge, the counter, the aaguid, the signing CA — and assert that this
 * specific check is what fires. That is the difference between testing the
 * checks and testing that the parser doesn't crash.
 *
 * The cost is that these certificates are ours, not Apple's, so this cannot
 * prove we accept a genuine Apple chain — only that we accept a well-formed
 * one and reject the malformations. Rejecting Apple's real root is the residual
 * risk, and it is the one thing that must be checked on a device.
 */

// ------------------------------------------------------------ DER encoding

function encodeLength(n: number): Uint8Array {
  if (n < 128) return Uint8Array.of(n);
  const bytes: number[] = [];
  let value = n;
  while (value > 0) {
    bytes.unshift(value & 0xff);
    value = Math.floor(value / 256);
  }
  return Uint8Array.of(0x80 | bytes.length, ...bytes);
}

export function cat(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}

export function tlv(tag: number, body: Uint8Array): Uint8Array {
  return cat(Uint8Array.of(tag), encodeLength(body.length), body);
}

const seq = (...parts: Uint8Array[]) => tlv(0x30, cat(...parts));
const set = (...parts: Uint8Array[]) => tlv(0x31, cat(...parts));
const octets = (body: Uint8Array) => tlv(0x04, body);
const explicit = (n: number, body: Uint8Array) => tlv(0xa0 | n, body);

/** DER INTEGER from a non-negative JS number. */
function integer(value: number): Uint8Array {
  const bytes: number[] = [];
  let v = value;
  do {
    bytes.unshift(v & 0xff);
    v = Math.floor(v / 256);
  } while (v > 0);
  // DER INTEGERs are signed; a leading high bit would read as negative.
  if ((bytes[0] ?? 0) & 0x80) bytes.unshift(0);
  return tlv(0x02, Uint8Array.from(bytes));
}

/** DER INTEGER from a big-endian magnitude, used for ECDSA r and s. */
function integerFromBytes(magnitude: Uint8Array): Uint8Array {
  let from = 0;
  while (from < magnitude.length - 1 && magnitude[from] === 0) from++;
  let bytes = Array.from(magnitude.subarray(from));
  if ((bytes[0] ?? 0) & 0x80) bytes = [0, ...bytes];
  return tlv(0x02, Uint8Array.from(bytes));
}

export function oid(dotted: string): Uint8Array {
  const arcs = dotted.split('.').map(Number);
  const bytes: number[] = [40 * (arcs[0] ?? 0) + (arcs[1] ?? 0)];
  for (const arc of arcs.slice(2)) {
    const chunk: number[] = [arc & 0x7f];
    let v = Math.floor(arc / 128);
    while (v > 0) {
      chunk.unshift((v & 0x7f) | 0x80);
      v = Math.floor(v / 128);
    }
    bytes.push(...chunk);
  }
  return tlv(0x06, Uint8Array.from(bytes));
}

function bitString(body: Uint8Array): Uint8Array {
  return tlv(0x03, cat(Uint8Array.of(0), body));
}

function utcTime(date: Date): Uint8Array {
  const pad = (n: number) => String(n).padStart(2, '0');
  const text =
    pad(date.getUTCFullYear() % 100) +
    pad(date.getUTCMonth() + 1) +
    pad(date.getUTCDate()) +
    pad(date.getUTCHours()) +
    pad(date.getUTCMinutes()) +
    pad(date.getUTCSeconds()) +
    'Z';
  return tlv(0x17, new TextEncoder().encode(text));
}

/** `CN=<name>` as an RFC 5280 Name. */
function name(common: string): Uint8Array {
  return seq(set(seq(oid('2.5.4.3'), tlv(0x13, new TextEncoder().encode(common)))));
}

const ECDSA_SHA256 = seq(oid('1.2.840.10045.4.3.2'));
const ECDSA_SHA384 = seq(oid('1.2.840.10045.4.3.3'));

// -------------------------------------------------------------- signing

function componentSize(curve: string): number {
  return curve === 'P-384' ? 48 : 32;
}

/** Web Crypto emits `r ‖ s`; X.509 wants `SEQUENCE { INTEGER r, INTEGER s }`. */
function rawSignatureToDer(raw: Uint8Array, curve: string): Uint8Array {
  const size = componentSize(curve);
  return seq(
    integerFromBytes(raw.subarray(0, size)),
    integerFromBytes(raw.subarray(size, size * 2))
  );
}

export interface KeyPairInfo {
  publicKey: CryptoKey;
  privateKey: CryptoKey;
  curve: string;
  /** Full SubjectPublicKeyInfo DER. */
  spki: Uint8Array;
  /** Uncompressed EC point, `0x04 ‖ X ‖ Y`. */
  point: Uint8Array;
}

export async function generateKeyPair(curve: 'P-256' | 'P-384'): Promise<KeyPairInfo> {
  const pair = (await crypto.subtle.generateKey({ name: 'ECDSA', namedCurve: curve }, true, [
    'sign',
    'verify',
  ])) as CryptoKeyPair;
  return {
    publicKey: pair.publicKey,
    privateKey: pair.privateKey,
    curve,
    // Cast: workers-types narrows exportKey's return by format, and the
    // ArrayBuffer overloads aren't selected for these literals under `strict`.
    spki: new Uint8Array((await crypto.subtle.exportKey('spki', pair.publicKey)) as ArrayBuffer),
    point: new Uint8Array((await crypto.subtle.exportKey('raw', pair.publicKey)) as ArrayBuffer),
  };
}

export interface CertOptions {
  subject: string;
  issuer: string;
  subjectKey: KeyPairInfo;
  issuerKey: KeyPairInfo;
  /** Extra extensions as `[oid, extnValue content]`. */
  extensions?: Array<[string, Uint8Array]>;
  notBefore?: Date;
  notAfter?: Date;
  serial?: number;
}

export async function makeCertificate(options: CertOptions): Promise<Uint8Array> {
  const hash = options.issuerKey.curve === 'P-384' ? 'SHA-384' : 'SHA-256';
  const algorithm = hash === 'SHA-384' ? ECDSA_SHA384 : ECDSA_SHA256;

  const extensionBlocks = (options.extensions ?? []).map(([extOid, value]) =>
    seq(oid(extOid), octets(value))
  );

  const tbs = seq(
    explicit(0, integer(2)), // v3
    integer(options.serial ?? 1),
    algorithm,
    name(options.issuer),
    seq(
      utcTime(options.notBefore ?? new Date(Date.now() - 86_400_000)),
      utcTime(options.notAfter ?? new Date(Date.now() + 86_400_000))
    ),
    name(options.subject),
    options.subjectKey.spki,
    ...(extensionBlocks.length ? [tlv(0xa3, seq(...extensionBlocks))] : [])
  );

  const raw = new Uint8Array(
    await crypto.subtle.sign({ name: 'ECDSA', hash }, options.issuerKey.privateKey, tbs)
  );
  return seq(tbs, algorithm, bitString(rawSignatureToDer(raw, options.issuerKey.curve)));
}

export function toPem(der: Uint8Array): string {
  let binary = '';
  for (const byte of der) binary += String.fromCharCode(byte);
  const base64 = btoa(binary).replace(/(.{64})/g, '$1\n');
  return `-----BEGIN CERTIFICATE-----\n${base64}\n-----END CERTIFICATE-----\n`;
}

/** Apple wraps the attestation nonce in `SEQUENCE { [1] { OCTET STRING } }`. */
export function nonceExtensionValue(nonce: Uint8Array): Uint8Array {
  return seq(tlv(0xa1, octets(nonce)));
}

// ------------------------------------------------------------------ CBOR

function cborHead(major: number, arg: number): Uint8Array {
  if (arg < 24) return Uint8Array.of((major << 5) | arg);
  if (arg < 0x100) return Uint8Array.of((major << 5) | 24, arg);
  if (arg < 0x10000) return Uint8Array.of((major << 5) | 25, arg >> 8, arg & 0xff);
  return Uint8Array.of(
    (major << 5) | 26,
    (arg >>> 24) & 0xff,
    (arg >>> 16) & 0xff,
    (arg >>> 8) & 0xff,
    arg & 0xff
  );
}

/** An interface, not a type alias — a `Record` inside the union is circular. */
export interface CborMap {
  [key: string]: CborInput;
}
export type CborInput = number | string | Uint8Array | CborInput[] | CborMap;

export function encodeCbor(value: CborInput): Uint8Array {
  if (typeof value === 'number') return cborHead(0, value);
  if (value instanceof Uint8Array) return cat(cborHead(2, value.length), value);
  if (typeof value === 'string') {
    const bytes = new TextEncoder().encode(value);
    return cat(cborHead(3, bytes.length), bytes);
  }
  if (Array.isArray(value)) {
    return cat(cborHead(4, value.length), ...value.map(encodeCbor));
  }
  const entries = Object.entries(value);
  return cat(
    cborHead(5, entries.length),
    ...entries.flatMap(([k, v]) => [encodeCbor(k), encodeCbor(v)])
  );
}

// ------------------------------------------------------ authenticator data

export function buildAuthData(options: {
  rpIdHash: Uint8Array;
  counter: number;
  aaguid?: string;
  credentialId?: Uint8Array;
}): Uint8Array {
  const header = new Uint8Array(37);
  header.set(options.rpIdHash, 0);
  header[32] = 0x40; // AT flag — attested credential data present
  new DataView(header.buffer).setUint32(33, options.counter);
  if (!options.aaguid || !options.credentialId) return header.subarray(0, 37);

  const aaguid = new Uint8Array(16);
  for (let i = 0; i < options.aaguid.length; i++) aaguid[i] = options.aaguid.charCodeAt(i);
  const length = new Uint8Array(2);
  new DataView(length.buffer).setUint16(0, options.credentialId.length);
  return cat(header, aaguid, length, options.credentialId);
}

export async function sha256(...parts: Uint8Array[]): Promise<Uint8Array> {
  return new Uint8Array(await crypto.subtle.digest('SHA-256', cat(...parts)));
}

// -------------------------------------------------------------- the world

export const TEST_APP_ID = 'ABCDE12345.com.isaacwm.sitewalk';

export interface AttestWorld {
  rootPem: string;
  /** The Secure Enclave key the device would hold. */
  deviceKey: KeyPairInfo;
  keyId: Uint8Array;
  /** Builds an attestation object over `challenge`, with optional sabotage. */
  attestationFor(
    challenge: string,
    overrides?: {
      counter?: number;
      aaguid?: string;
      appId?: string;
      credentialId?: Uint8Array;
      /** Sign the leaf with a CA that does not chain to the pinned root. */
      rogueIssuer?: boolean;
      /** Put a different nonce in the certificate extension. */
      nonce?: Uint8Array;
      notAfter?: Date;
    }
  ): Promise<Uint8Array>;
  /** Builds an assertion signed by the device key. */
  assertionFor(
    challenge: string,
    counter: number,
    overrides?: { appId?: string; wrongKey?: KeyPairInfo }
  ): Promise<Uint8Array>;
}

export async function buildWorld(): Promise<AttestWorld> {
  const root = await generateKeyPair('P-384');
  const intermediate = await generateKeyPair('P-384');
  const rogueRoot = await generateKeyPair('P-384');
  const deviceKey = await generateKeyPair('P-256');
  const keyId = await sha256(deviceKey.point);

  const rootDer = await makeCertificate({
    subject: 'Test App Attest Root',
    issuer: 'Test App Attest Root',
    subjectKey: root,
    issuerKey: root,
  });
  const intermediateDer = await makeCertificate({
    subject: 'Test App Attest CA 1',
    issuer: 'Test App Attest Root',
    subjectKey: intermediate,
    issuerKey: root,
  });
  const rogueIntermediateDer = await makeCertificate({
    subject: 'Test App Attest CA 1',
    issuer: 'Test App Attest Root',
    subjectKey: rogueRoot,
    issuerKey: rogueRoot,
  });

  return {
    rootPem: toPem(rootDer),
    deviceKey,
    keyId,

    async attestationFor(challenge, overrides = {}) {
      const rpIdHash = await sha256(
        new TextEncoder().encode(overrides.appId ?? TEST_APP_ID)
      );
      const authData = buildAuthData({
        rpIdHash,
        counter: overrides.counter ?? 0,
        aaguid: overrides.aaguid ?? 'appattest\0\0\0\0\0\0\0',
        credentialId: overrides.credentialId ?? keyId,
      });

      const clientDataHash = await sha256(new TextEncoder().encode(challenge));
      const nonce = overrides.nonce ?? (await sha256(authData, clientDataHash));

      const signer = overrides.rogueIssuer ? rogueRoot : intermediate;
      const leafDer = await makeCertificate({
        subject: 'device',
        issuer: 'Test App Attest CA 1',
        subjectKey: deviceKey,
        issuerKey: signer,
        extensions: [['1.2.840.113635.100.8.2', nonceExtensionValue(nonce)]],
        notAfter: overrides.notAfter,
      });

      return encodeCbor({
        fmt: 'apple-appattest',
        attStmt: {
          x5c: [leafDer, overrides.rogueIssuer ? rogueIntermediateDer : intermediateDer],
          receipt: new Uint8Array([1, 2, 3]),
        },
        authData,
      });
    },

    async assertionFor(challenge, counter, overrides = {}) {
      const rpIdHash = await sha256(
        new TextEncoder().encode(overrides.appId ?? TEST_APP_ID)
      );
      const authData = buildAuthData({ rpIdHash, counter });
      const clientDataHash = await sha256(new TextEncoder().encode(challenge));
      const key = overrides.wrongKey ?? deviceKey;
      const raw = new Uint8Array(
        await crypto.subtle.sign(
          { name: 'ECDSA', hash: 'SHA-256' },
          key.privateKey,
          cat(authData, clientDataHash)
        )
      );
      return encodeCbor({
        signature: rawSignatureToDer(raw, key.curve),
        authenticatorData: authData,
      });
    },
  };
}
