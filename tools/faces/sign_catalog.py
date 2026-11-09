#!/usr/bin/env python3
"""Signs the faces catalog (docs/faces/CATALOG.md, "Verify"; PLAN.md step 6) with ed25519.

    python3 tools/faces/sign_catalog.py keygen KEYFILE        # new key: private -> KEYFILE (0600),
                                                              # prints the public key for faces.CATALOG_KEYS
    python3 tools/faces/sign_catalog.py sign site/faces/catalog.json      # key from $FACES_SIGNING_KEY
    python3 tools/faces/sign_catalog.py verify site/faces/catalog.json    # against the agent's CATALOG_KEYS

The signature is catalog.json.sig next to the catalog: base64 of the raw 64-byte ed25519
signature of catalog.json's exact bytes. Keys are base64 of the raw 32 bytes (the private key
is its seed). The private key lives only in the repo's Actions secret FACES_SIGNING_KEY; the
agent accepts any key in mac-agent/stick_agent/faces.py CATALOG_KEYS, so a key can be rotated.

Needs the `cryptography` package (the agent has it; CI installs it). The other tools in this
folder are standard library only.
"""
from __future__ import annotations

import argparse
import base64
import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
AGENT_KEYS = ROOT / "mac-agent" / "stick_agent" / "faces.py"


def _ed25519():
    from cryptography.hazmat.primitives import serialization
    from cryptography.hazmat.primitives.asymmetric import ed25519
    return ed25519, serialization


def agent_keys(path=AGENT_KEYS):
    """The public keys in the agent's CATALOG_KEYS list."""
    src = Path(path).read_text()
    block = re.search(r"CATALOG_KEYS\s*=\s*\[(.*?)\]", src, re.S)
    return re.findall(r'"([A-Za-z0-9+/=]{40,48})"', block.group(1)) if block else []


def keygen(out: Path):
    ed, ser = _ed25519()
    key = ed.Ed25519PrivateKey.generate()
    seed = key.private_bytes(ser.Encoding.Raw, ser.PrivateFormat.Raw, ser.NoEncryption())
    pub = key.public_key().public_bytes(ser.Encoding.Raw, ser.PublicFormat.Raw)
    fd = os.open(out, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as f:
        f.write(base64.b64encode(seed).decode() + "\n")
    return base64.b64encode(pub).decode()


def sign(catalog: Path, private_b64: str) -> Path:
    ed, _ = _ed25519()
    key = ed.Ed25519PrivateKey.from_private_bytes(base64.b64decode(private_b64.strip(), validate=True))
    sig = Path(str(catalog) + ".sig")
    sig.write_text(base64.b64encode(key.sign(catalog.read_bytes())).decode() + "\n")
    return sig


def verify(catalog: Path, keys) -> bool:
    ed, _ = _ed25519()
    from cryptography.exceptions import InvalidSignature
    sig = base64.b64decode(Path(str(catalog) + ".sig").read_text().strip(), validate=True)
    data = catalog.read_bytes()
    for k in keys:
        try:
            ed.Ed25519PublicKey.from_public_bytes(base64.b64decode(k)).verify(sig, data)
            return True
        except (InvalidSignature, ValueError):
            continue
    return False


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    k = sub.add_parser("keygen", help="make a key pair: the private key into a new file, the public key printed")
    k.add_argument("keyfile", type=Path)
    s = sub.add_parser("sign", help="write <catalog>.sig with the key in $FACES_SIGNING_KEY")
    s.add_argument("catalog", type=Path)
    s.add_argument("--key-env", default="FACES_SIGNING_KEY")
    v = sub.add_parser("verify", help="check <catalog>.sig against the agent's CATALOG_KEYS (or --key)")
    v.add_argument("catalog", type=Path)
    v.add_argument("--key", action="append", help="a public key (base64); repeatable")
    args = ap.parse_args(argv)
    if args.cmd == "keygen":
        pub = keygen(args.keyfile)
        print(f"private key: {args.keyfile} (keep it secret: it goes into the Actions secret FACES_SIGNING_KEY)")
        print(f"public key:  {pub}   (add it to CATALOG_KEYS in mac-agent/stick_agent/faces.py)")
        return 0
    if args.cmd == "sign":
        secret = os.environ.get(args.key_env, "")
        if not secret.strip():
            print(f"${args.key_env} is empty: set the Actions secret (see docs/faces/CATALOG.md)", file=sys.stderr)
            return 1
        print(f"wrote {sign(args.catalog, secret)}")
        return 0
    keys = args.key or agent_keys()
    if not keys:
        print("no public keys to check against", file=sys.stderr)
        return 1
    ok = verify(args.catalog, keys)
    print(f"{args.catalog}: signature {'ok' if ok else 'does NOT match any accepted key'}")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
