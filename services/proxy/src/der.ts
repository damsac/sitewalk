/**
 * A tiny DER / X.509 reader — only what App Attest chain verification needs.
 *
 * Workers have no `X509Certificate` (that is `node:crypto`, and this worker
 * runs without `nodejs_compat`), and Web Crypto will import an SPKI but will
 * not parse a certificate. So the certificate walking is done by hand here and
 * the actual cryptography is left entirely to Web Crypto. That division is
 * deliberate: hand-rolled parsing is survivable, hand-rolled ECDSA is not.
 *
 * Same posture as the CBOR decoder — unknown or malformed input throws.
 */

export class DerError extends Error {}

export interface Tlv {
  /** The identifier octet. 0x30 SEQUENCE, 0x02 INTEGER, 0x03 BIT STRING, 0x04 OCTET STRING, 0x06 OID. */
  tag: number;
  /** Offset of the identifier octet, i.e. where the whole TLV starts. */
  start: number;
  /** Offset of the first content byte. */
  contentStart: number;
  /** Number of content bytes. */
  length: number;
  /** Offset one past the last content byte. */
  end: number;
}

/**
 * Bounds-checked byte read.
 *
 * Every caller has already length-checked; this turns `noUncheckedIndexedAccess`
 * into a real runtime guard rather than a `!` that hides one.
 */
function byteAt(buf: Uint8Array, index: number): number {
  const value = buf[index];
  if (value === undefined) throw new DerError('read past the end of the buffer');
  return value;
}

/** Asserts a structurally required element is present. */
function required<T>(value: T | undefined, what: string): T {
  if (value === undefined) throw new DerError(`missing ${what}`);
  return value;
}

export function parseTlv(buf: Uint8Array, pos: number): Tlv {
  if (pos + 2 > buf.length) throw new DerError('truncated TLV header');
  const start = pos;
  const tag = byteAt(buf, pos++);

  // High-tag-number form (0x1f) never appears in the certificates Apple issues.
  if ((tag & 0x1f) === 0x1f) throw new DerError('multi-byte tags are not supported');

  const first = byteAt(buf, pos++);
  let length: number;
  if (first < 0x80) {
    length = first;
  } else {
    const count = first & 0x7f;
    // 0x80 is BER indefinite length — invalid in DER, and a classic way to
    // desynchronise two parsers looking at the same bytes.
    if (count === 0) throw new DerError('indefinite length is not valid DER');
    // >4 length bytes means a value larger than any certificate we will ever
    // see; refusing keeps `length` inside exact integer range.
    if (count > 4) throw new DerError('length field too large');
    if (pos + count > buf.length) throw new DerError('truncated length field');
    length = 0;
    for (let i = 0; i < count; i++) length = length * 256 + byteAt(buf, pos++);
  }

  const contentStart = pos;
  const end = contentStart + length;
  if (end > buf.length) throw new DerError('TLV content runs past the buffer');
  return { tag, start, contentStart, length, end };
}

/** Every direct child of a constructed TLV. */
export function children(buf: Uint8Array, parent: Tlv): Tlv[] {
  const out: Tlv[] = [];
  let pos = parent.contentStart;
  while (pos < parent.end) {
    const child = parseTlv(buf, pos);
    out.push(child);
    pos = child.end;
  }
  if (pos !== parent.end) throw new DerError('children do not tile the parent exactly');
  return out;
}

export function content(buf: Uint8Array, tlv: Tlv): Uint8Array {
  return buf.subarray(tlv.contentStart, tlv.end);
}

/** The complete TLV including its header — what a signature is computed over. */
export function raw(buf: Uint8Array, tlv: Tlv): Uint8Array {
  return buf.subarray(tlv.start, tlv.end);
}

/** Decodes an OBJECT IDENTIFIER's content octets to dotted-decimal form. */
export function decodeOid(bytes: Uint8Array): string {
  if (bytes.length === 0) throw new DerError('empty OID');
  const parts: number[] = [];
  // The first octet packs two arcs: 40*first + second.
  const first = byteAt(bytes, 0);
  parts.push(Math.floor(first / 40), first % 40);

  let value = 0;
  for (let i = 1; i < bytes.length; i++) {
    const byte = byteAt(bytes, i);
    value = value * 128 + (byte & 0x7f);
    if ((byte & 0x80) === 0) {
      parts.push(value);
      value = 0;
    }
  }
  if (value !== 0) throw new DerError('OID ends mid-arc');
  return parts.join('.');
}

// ------------------------------------------------------------ certificates

/** Named curve OIDs, mapped to the labels Web Crypto expects. */
const CURVE_BY_OID: Record<string, string> = {
  '1.2.840.10045.3.1.7': 'P-256',
  '1.3.132.0.34': 'P-384',
  '1.3.132.0.35': 'P-521',
};

/** ECDSA signature-algorithm OIDs, mapped to their digest. */
const HASH_BY_SIG_OID: Record<string, string> = {
  '1.2.840.10045.4.3.2': 'SHA-256',
  '1.2.840.10045.4.3.3': 'SHA-384',
  '1.2.840.10045.4.3.4': 'SHA-512',
};

export interface Certificate {
  /** The exact `tbsCertificate` bytes, header included — the signed message. */
  tbs: Uint8Array;
  /** Digest named by the outer signatureAlgorithm, e.g. `SHA-256`. */
  signatureHash: string;
  /** Signature value, still in DER `SEQUENCE { r, s }` form. */
  signature: Uint8Array;
  /** The full SubjectPublicKeyInfo, ready to hand to `importKey('spki', …)`. */
  spki: Uint8Array;
  /** This certificate's own curve, e.g. `P-384`. */
  curve: string;
  /** The raw uncompressed EC point (`0x04 ‖ X ‖ Y`) — what App Attest hashes into a key id. */
  publicKeyPoint: Uint8Array;
  /** DER-encoded issuer and subject names, for chain linking. */
  issuer: Uint8Array;
  subject: Uint8Array;
  notBefore: Date;
  notAfter: Date;
  /** Extensions by OID. Values are the extnValue OCTET STRING contents. */
  extensions: Map<string, Uint8Array>;
}

/** ASN.1 UTCTime / GeneralizedTime → Date. */
function parseTime(tag: number, text: string): Date {
  const m =
    tag === 0x17
      ? /^(\d{2})(\d{2})(\d{2})(\d{2})(\d{2})(\d{2})Z$/.exec(text)
      : /^(\d{4})(\d{2})(\d{2})(\d{2})(\d{2})(\d{2})Z$/.exec(text);
  if (!m) throw new DerError(`unparseable validity time "${text}"`);
  let year = Number(m[1]);
  // RFC 5280: two-digit years >= 50 are 19xx, below are 20xx.
  if (tag === 0x17) year += year >= 50 ? 1900 : 2000;
  return new Date(
    Date.UTC(year, Number(m[2]) - 1, Number(m[3]), Number(m[4]), Number(m[5]), Number(m[6]))
  );
}

export function parseCertificate(der: Uint8Array): Certificate {
  const cert = parseTlv(der, 0);
  if (cert.tag !== 0x30) throw new DerError('certificate is not a SEQUENCE');
  if (cert.end !== der.length) throw new DerError('trailing bytes after the certificate');

  const [tbsTlv, sigAlgTlv, sigTlv] = children(der, cert);
  if (!tbsTlv || !sigAlgTlv || !sigTlv) throw new DerError('certificate has too few fields');

  // --- outer signature algorithm
  const sigAlgOid = children(der, sigAlgTlv)[0];
  if (!sigAlgOid || sigAlgOid.tag !== 0x06) throw new DerError('missing signature algorithm OID');
  const sigOidText = decodeOid(content(der, sigAlgOid));
  const signatureHash = HASH_BY_SIG_OID[sigOidText];
  if (!signatureHash) {
    // RSA-signed certificates would land here. App Attest is ECDSA end to end;
    // silently accepting anything else would widen what we trust.
    throw new DerError(`unsupported signature algorithm ${sigOidText}`);
  }

  // --- signature value (BIT STRING: first content byte is the unused-bit count)
  if (sigTlv.tag !== 0x03) throw new DerError('signature is not a BIT STRING');
  const sigBits = content(der, sigTlv);
  if (sigBits.length < 1 || sigBits[0] !== 0) throw new DerError('signature has unused bits');
  const signature = sigBits.subarray(1);

  // --- tbsCertificate fields
  const tbsFields = children(der, tbsTlv);
  let i = 0;
  // [0] EXPLICIT version, optional and always present in the v3 certs Apple issues.
  if (tbsFields[i]?.tag === 0xa0) i++;
  i++; // serialNumber
  i++; // inner signature AlgorithmIdentifier
  const issuerTlv = tbsFields[i++];
  const validityTlv = tbsFields[i++];
  const subjectTlv = tbsFields[i++];
  const spkiTlv = tbsFields[i++];
  if (!issuerTlv || !validityTlv || !subjectTlv || !spkiTlv) {
    throw new DerError('tbsCertificate is missing required fields');
  }

  const validityParts = children(der, validityTlv);
  const notBeforeTlv = required(validityParts[0], 'notBefore');
  const notAfterTlv = required(validityParts[1], 'notAfter');
  const decoder = new TextDecoder();
  const notBefore = parseTime(notBeforeTlv.tag, decoder.decode(content(der, notBeforeTlv)));
  const notAfter = parseTime(notAfterTlv.tag, decoder.decode(content(der, notAfterTlv)));

  // --- public key
  const spkiParts = children(der, spkiTlv);
  const algTlv = required(spkiParts[0], 'public key algorithm');
  const keyBitsTlv = required(spkiParts[1], 'public key bits');
  const algParts = children(der, algTlv);
  const algOid = decodeOid(content(der, required(algParts[0], 'algorithm OID')));
  if (algOid !== '1.2.840.10045.2.1') throw new DerError(`public key is not EC (${algOid})`);
  const curveParam = algParts[1];
  const curveOid = curveParam ? decodeOid(content(der, curveParam)) : '';
  const curve = CURVE_BY_OID[curveOid];
  if (!curve) throw new DerError(`unsupported curve ${curveOid}`);

  if (keyBitsTlv.tag !== 0x03) throw new DerError('public key is not a BIT STRING');
  const keyBits = content(der, keyBitsTlv);
  if (keyBits.length < 2 || keyBits[0] !== 0) throw new DerError('public key has unused bits');
  const publicKeyPoint = keyBits.subarray(1);
  if (byteAt(publicKeyPoint, 0) !== 0x04) {
    // Compressed points would need decompression before hashing, and Apple
    // does not issue them. Rejecting is safer than guessing.
    throw new DerError('public key point is not in uncompressed form');
  }

  // --- extensions, under [3] EXPLICIT
  const extensions = new Map<string, Uint8Array>();
  for (let j = i; j < tbsFields.length; j++) {
    const field = tbsFields[j];
    if (!field || field.tag !== 0xa3) continue;
    const seq = required(children(der, field)[0], 'extensions SEQUENCE');
    for (const ext of children(der, seq)) {
      const parts = children(der, ext);
      const oid = decodeOid(content(der, required(parts[0], 'extension OID')));
      // parts[1] may be the optional `critical` BOOLEAN; the value is the last.
      const valueTlv = required(parts[parts.length - 1], 'extension value');
      extensions.set(oid, content(der, valueTlv));
    }
  }

  return {
    tbs: raw(der, tbsTlv),
    signatureHash,
    signature,
    spki: raw(der, spkiTlv),
    curve,
    publicKeyPoint,
    issuer: raw(der, issuerTlv),
    subject: raw(der, subjectTlv),
    notBefore,
    notAfter,
    extensions,
  };
}

/**
 * Converts a DER `SEQUENCE { INTEGER r, INTEGER s }` signature into the fixed
 * width `r ‖ s` that Web Crypto requires.
 *
 * DER INTEGERs are signed and minimally encoded, so `r` may carry a leading
 * 0x00 (when its high bit is set) or be short (when it has leading zero bytes).
 * Both have to be normalised to exactly `size` bytes or verification fails for
 * roughly one signature in 256 — the kind of bug that looks like flaky
 * hardware.
 */
export function ecdsaDerToRaw(derSig: Uint8Array, size: number): Uint8Array {
  const seq = parseTlv(derSig, 0);
  if (seq.tag !== 0x30) throw new DerError('ECDSA signature is not a SEQUENCE');
  const [rTlv, sTlv] = children(derSig, seq);
  if (!rTlv || !sTlv || rTlv.tag !== 0x02 || sTlv.tag !== 0x02) {
    throw new DerError('ECDSA signature is not two INTEGERs');
  }

  const out = new Uint8Array(size * 2);
  const place = (tlv: Tlv, offset: number) => {
    let bytes = content(derSig, tlv);
    let from = 0;
    while (from < bytes.length - 1 && byteAt(bytes, from) === 0) from++;
    bytes = bytes.subarray(from);
    if (bytes.length > size) throw new DerError('ECDSA signature component is too large');
    out.set(bytes, offset + size - bytes.length);
  };
  place(rTlv, 0);
  place(sTlv, size);
  return out;
}

/** Bytes per ECDSA signature component, by curve. */
export function componentSize(curve: string): number {
  if (curve === 'P-256') return 32;
  if (curve === 'P-384') return 48;
  if (curve === 'P-521') return 66;
  throw new DerError(`unknown curve ${curve}`);
}

/** Decodes a PEM block (any label) to DER. */
export function pemToDer(pem: string): Uint8Array {
  const match = /-----BEGIN [^-]+-----([\s\S]*?)-----END [^-]+-----/.exec(pem);
  if (!match) throw new DerError('no PEM block found');
  const base64 = required(match[1], 'PEM body').replace(/\s+/g, '');
  const binary = atob(base64);
  const out = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) out[i] = binary.charCodeAt(i);
  return out;
}
