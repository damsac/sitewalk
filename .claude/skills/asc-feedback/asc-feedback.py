#!/usr/bin/env python3
"""Minimal App Store Connect API client. Prints API responses only; never prints key material."""
import base64, json, sys, time, urllib.request
from pathlib import Path
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives.asymmetric.utils import decode_dss_signature

SEC = Path.home() / "secrets" / "apple"
KEY_ID = (SEC / "asc_key_id").read_text().strip()
ISSUER = (SEC / "asc_issuer_id").read_text().strip()
PKEY = serialization.load_pem_private_key((SEC / "AuthKey.p8").read_bytes(), password=None)

def b64u(b): return base64.urlsafe_b64encode(b).rstrip(b"=").decode()

def token():
    now = int(time.time())
    header = b64u(json.dumps({"alg": "ES256", "kid": KEY_ID, "typ": "JWT"}).encode())
    payload = b64u(json.dumps({"iss": ISSUER, "iat": now, "exp": now + 900,
                               "aud": "appstoreconnect-v1"}).encode())
    signing_input = f"{header}.{payload}".encode()
    der = PKEY.sign(signing_input, ec.ECDSA(hashes.SHA256()))
    r, s = decode_dss_signature(der)
    sig = b64u(r.to_bytes(32, "big") + s.to_bytes(32, "big"))
    return f"{header}.{payload}.{sig}"

def get(path, raw=False):
    url = path if path.startswith("http") else f"https://api.appstoreconnect.apple.com{path}"
    req = urllib.request.Request(url, headers={"Authorization": f"Bearer {token()}"})
    try:
        with urllib.request.urlopen(req) as r:
            body = r.read()
            return body if raw else json.loads(body)
    except urllib.error.HTTPError as e:
        print(f"HTTP {e.code} for {url}\n{e.read().decode()[:2000]}", file=sys.stderr)
        sys.exit(1)

if __name__ == "__main__":
    out = get(sys.argv[1], raw="--raw" in sys.argv)
    if isinstance(out, bytes):
        sys.stdout.buffer.write(out)
    else:
        print(json.dumps(out, indent=1))
