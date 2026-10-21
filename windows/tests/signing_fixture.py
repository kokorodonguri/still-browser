"""Throwaway minisign keys that sign like `tauri signer sign`, for tests only.

Never touches the real update key. Without the `cryptography` package the
signatures are structurally valid but carry random bytes; release_set then
only checks structure, key ID and file name, as it does in production.
"""
import base64
import hashlib
import os
from pathlib import Path
import tempfile
import time

try:
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
except ImportError:
    Ed25519PrivateKey = None

CRYPTOGRAPHY = Ed25519PrivateKey is not None


def _b64(data: bytes) -> str:
    return base64.b64encode(data).decode()


class Signer:
    def __init__(self, key_id: bytes | None = None):
        self.key_id = key_id or os.urandom(8)
        if CRYPTOGRAPHY:
            self.private = Ed25519PrivateKey.generate()
            raw = self.private.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
        else:
            self.private, raw = None, os.urandom(32)
        text = (f"untrusted comment: minisign public key: {self.key_id[::-1].hex().upper()}\n"
                f"{_b64(b'Ed' + self.key_id + raw)}\n")
        self.public_key = _b64(text.encode())

    def _sign(self, message: bytes) -> bytes:
        return self.private.sign(message) if self.private else os.urandom(64)

    def sign(self, data: bytes, name: str) -> str:
        """What `tauri signer sign` writes to <name>.sig."""
        signature = self._sign(hashlib.blake2b(data).digest())
        trusted = f"timestamp:{int(time.time())}\tfile:{name}"
        text = (f"untrusted comment: signature from tauri secret key\n{_b64(b'ED' + self.key_id + signature)}\n"
                f"trusted comment: {trusted}\n{_b64(self._sign(signature + trusted.encode()))}\n")
        return _b64(text.encode())

    def build(self, folder: Path, name: str, data: bytes) -> Path:
        """A signed installer as windows/package.py leaves it before commit."""
        built = folder / name
        built.write_bytes(data)
        built.with_name(name + ".sig").write_text(self.sign(data, name))
        return built


def fast_temporary_directory():
    """tmpfs when available: the publication code fsyncs every file it commits."""
    shm = Path("/dev/shm")
    return tempfile.TemporaryDirectory(dir=shm if shm.is_dir() and os.access(shm, os.W_OK) else None)
