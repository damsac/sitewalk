/**
 * A deliberately tiny CBOR decoder (RFC 8949).
 *
 * App Attest hands us two CBOR blobs — the attestation object and each
 * assertion — and Workers ship no CBOR primitive. Rather than take a
 * dependency for two call sites, this decodes only the subset Apple actually
 * emits: unsigned ints, byte strings, text strings, arrays and maps.
 *
 * Everything else THROWS rather than being tolerated. That is the point. A
 * decoder that shrugs at input it doesn't understand is exactly how a parser
 * becomes an attack surface, and the only writer we accept input from is
 * Apple's attestation framework, whose output shape is fixed. If this ever
 * throws on a real device, the right fix is to widen it deliberately — not to
 * make it lenient.
 */

export class CborError extends Error {}

export type CborValue =
  | number
  | string
  | Uint8Array
  | CborValue[]
  | Map<string | number, CborValue>;

interface Cursor {
  buf: Uint8Array;
  view: DataView;
  pos: number;
}

/** Bounds check. Every read goes through this — a truncated blob must throw, not read garbage. */
function need(c: Cursor, n: number): void {
  if (c.pos + n > c.buf.length) {
    throw new CborError(`truncated: needed ${n} bytes at ${c.pos}, have ${c.buf.length - c.pos}`);
  }
}

/**
 * Bounds-checked byte read.
 *
 * `need` has already proven the bound at every call site; this exists because
 * `noUncheckedIndexedAccess` is on and silencing it with `!` would make the
 * next edit to `need` fail silently instead of loudly.
 */
function byteAt(c: Cursor, index: number): number {
  const value = c.buf[index];
  if (value === undefined) throw new CborError('read past the end of the buffer');
  return value;
}

/**
 * Reads an initial byte and its argument.
 *
 * Lengths are capped at `Number.MAX_SAFE_INTEGER` semantics by construction —
 * a 64-bit argument is assembled as a float64, which is fine because anything
 * over ~8MB is rejected by the caller's size limit long before precision
 * matters.
 */
function readHead(c: Cursor): { major: number; arg: number } {
  need(c, 1);
  const initial = byteAt(c, c.pos++);
  const major = initial >> 5;
  const info = initial & 0x1f;

  if (info < 24) return { major, arg: info };
  if (info === 24) {
    need(c, 1);
    return { major, arg: byteAt(c, c.pos++) };
  }
  if (info === 25) {
    need(c, 2);
    const arg = c.view.getUint16(c.pos);
    c.pos += 2;
    return { major, arg };
  }
  if (info === 26) {
    need(c, 4);
    const arg = c.view.getUint32(c.pos);
    c.pos += 4;
    return { major, arg };
  }
  if (info === 27) {
    need(c, 8);
    const hi = c.view.getUint32(c.pos);
    const lo = c.view.getUint32(c.pos + 4);
    c.pos += 8;
    return { major, arg: hi * 2 ** 32 + lo };
  }
  // 28-30 are reserved; 31 is indefinite length, which Apple never emits and
  // which is the classic source of unbounded-allocation bugs.
  throw new CborError(`unsupported additional info ${info}`);
}

function readValue(c: Cursor, depth: number): CborValue {
  // Nesting is 3 deep in the real payloads. The cap stops a hostile blob from
  // recursing the stack to death before any length check can fire.
  if (depth > 16) throw new CborError('nesting too deep');

  const { major, arg } = readHead(c);

  switch (major) {
    case 0: // unsigned int
      return arg;
    case 1: // negative int
      return -1 - arg;
    case 2: {
      // byte string
      need(c, arg);
      const bytes = c.buf.subarray(c.pos, c.pos + arg);
      c.pos += arg;
      // Copy: subarray aliases the input, and callers hold these past the
      // decode. Aliasing would make a later mutation spooky at a distance.
      return new Uint8Array(bytes);
    }
    case 3: {
      // text string
      need(c, arg);
      const text = new TextDecoder('utf-8', { fatal: true, ignoreBOM: false }).decode(
        c.buf.subarray(c.pos, c.pos + arg)
      );
      c.pos += arg;
      return text;
    }
    case 4: {
      // array
      const out: CborValue[] = [];
      for (let i = 0; i < arg; i++) out.push(readValue(c, depth + 1));
      return out;
    }
    case 5: {
      // map
      const out = new Map<string | number, CborValue>();
      for (let i = 0; i < arg; i++) {
        const key = readValue(c, depth + 1);
        if (typeof key !== 'string' && typeof key !== 'number') {
          throw new CborError('map keys must be text or integer');
        }
        // Duplicate keys are malformed CBOR and are a known parser-differential
        // trick — two verifiers disagreeing on which value wins.
        if (out.has(key)) throw new CborError(`duplicate map key ${String(key)}`);
        out.set(key, readValue(c, depth + 1));
      }
      return out;
    }
    default:
      // 6 = tags, 7 = floats/simple. Neither appears in App Attest payloads.
      throw new CborError(`unsupported major type ${major}`);
  }
}

/**
 * Decodes one CBOR item and REQUIRES it to consume the whole buffer.
 *
 * Trailing bytes are rejected on purpose: "valid prefix, then junk" is how you
 * smuggle a second interpretation of the same blob past a verifier.
 */
export function decodeCbor(bytes: Uint8Array): CborValue {
  const cursor: Cursor = {
    buf: bytes,
    view: new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength),
    pos: 0,
  };
  const value = readValue(cursor, 0);
  if (cursor.pos !== bytes.length) {
    throw new CborError(`${bytes.length - cursor.pos} trailing bytes after the CBOR item`);
  }
  return value;
}

// -------------------------------------------------------------- accessors

/** Typed map lookup that names the field in its error, so failures are debuggable. */
export function mapField(map: CborValue, key: string): CborValue {
  if (!(map instanceof Map)) throw new CborError('expected a CBOR map');
  const value = map.get(key);
  if (value === undefined) throw new CborError(`missing field "${key}"`);
  return value;
}

export function bytesField(map: CborValue, key: string): Uint8Array {
  const value = mapField(map, key);
  if (!(value instanceof Uint8Array)) throw new CborError(`field "${key}" is not a byte string`);
  return value;
}

export function textField(map: CborValue, key: string): string {
  const value = mapField(map, key);
  if (typeof value !== 'string') throw new CborError(`field "${key}" is not text`);
  return value;
}
