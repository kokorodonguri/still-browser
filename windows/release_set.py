"""Signed Windows release sets: an installer, its minisign signature, and the
record that binds the two.

`tauri signer sign` writes `<installer>.sig`: base64 of a minisign signature
file. A reader that opens the installer and the `.sig` separately can pair a
new installer with an old signature while a rebuild replaces them, because two
renames are never one atomic step. The record `<installer>.release.json`
closes that gap:

    {"file": "Still_0.6.0_x64-setup.exe", "size": 123, "sha256": "...", "signature": "..."}

windows/package.py writes the record last, with one atomic rename, after the
installer and `.sig` it describes are in place. The record is the commit point
of a set; it carries the signature itself. site/build.py snapshots the
installer, hashes the snapshot, and only accepts a record whose size and
SHA-256 match those bytes, so the signature it publishes belongs to the bytes
it publishes, whatever a concurrent rebuild does.

Signatures are checked structurally (minisign layout, the update key's key ID,
the signed file name). When the `cryptography` package is importable they are
also verified with Ed25519, exactly as the launcher does.
"""
import base64
import binascii
import hashlib
import json
import os
from pathlib import Path
import tempfile

RECORD_SUFFIX = ".release.json"


class SignatureError(ValueError):
    """A signature that does not belong to the update key or the bytes."""


def record_path(installer: Path) -> Path:
    return installer.with_name(installer.name + RECORD_SUFFIX)


def signature_path(installer: Path) -> Path:
    return installer.with_name(installer.name + ".sig")


def file_digest(path: Path) -> tuple[int, str]:
    """(size, SHA-256) of the bytes read in one pass."""
    digest, size = hashlib.sha256(), 0
    with path.open("rb") as file:
        for chunk in iter(lambda: file.read(1 << 20), b""):
            size += len(chunk)
            digest.update(chunk)
    return size, digest.hexdigest()


def fsync_dir(directory: Path) -> None:
    """Persist renames in `directory` (best effort where directories cannot be opened)."""
    try:
        fd = os.open(directory, os.O_RDONLY)
    except OSError:
        return
    try:
        os.fsync(fd)
    except OSError:
        pass
    finally:
        os.close(fd)


def fsync_file(path: Path) -> None:
    fd = os.open(path, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def prepare(path: Path, data: bytes, mode: int = 0o644) -> Path:
    """A complete, synced temporary file beside `path`, ready to be renamed onto it."""
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=path.parent, prefix=f".{path.name}.", delete=False) as output:
            temporary = Path(output.name)
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
            os.fchmod(output.fileno(), mode)
    except BaseException:
        if temporary is not None:
            temporary.unlink(missing_ok=True)
        raise
    return temporary


def write_atomic(path: Path, data: bytes, mode: int = 0o644) -> None:
    """Replace `path` with complete, synced `data`; readers see the old or new file only."""
    temporary = prepare(path, data, mode)
    try:
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)
    fsync_dir(path.parent)


def _b64(text: str | bytes) -> bytes:
    try:
        return base64.b64decode(text, validate=True)
    except (binascii.Error, ValueError) as error:
        raise SignatureError("not base64") from error


def public_key_id(public_key: str) -> tuple[bytes, bytes]:
    """(key ID, Ed25519 key) of a Tauri updater public key (base64 of a minisign .pub)."""
    lines = _b64(public_key.strip()).decode("utf-8", "replace").splitlines()
    if len(lines) < 2:
        raise SignatureError("public key: not a minisign public key")
    raw = _b64(lines[1].strip())
    if len(raw) != 42 or raw[:2] != b"Ed":
        raise SignatureError("public key: not a minisign Ed25519 public key")
    return raw[2:10], raw[10:]


def parse_signature(signature: str) -> dict:
    """The parts of a Tauri signature (base64 of a minisign signature file)."""
    try:
        text = _b64(signature.strip()).decode("utf-8")
    except UnicodeDecodeError as error:
        raise SignatureError("signature: not text") from error
    lines = text.splitlines()
    if len(lines) < 4 or not lines[0].startswith("untrusted comment:") or not lines[2].startswith("trusted comment: "):
        raise SignatureError("signature: not a minisign signature file")
    raw = _b64(lines[1].strip())
    if len(raw) != 74 or raw[:2] not in (b"ED", b"Ed"):
        raise SignatureError("signature: unknown minisign algorithm")
    trusted = lines[2][len("trusted comment: "):]
    global_signature = _b64(lines[3].strip())
    if len(global_signature) != 64:
        raise SignatureError("signature: truncated trusted comment signature")
    fields = dict(part.split(":", 1) for part in trusted.split("\t") if ":" in part)
    return {"algorithm": raw[:2], "key_id": raw[2:10], "signature": raw[10:],
            "trusted": trusted, "global": global_signature, "fields": fields}


def verify_signature(installer: Path, signature: str, public_key: str, name: str | None = None) -> bool:
    """Raises SignatureError unless `signature` is the update key's signature of `installer`.

    Returns True when the Ed25519 signatures were verified, False when only the
    structure, key ID and signed file name could be checked (no `cryptography`).
    """
    parts = parse_signature(signature)
    key_id, key = public_key_id(public_key)
    if parts["key_id"] != key_id:
        raise SignatureError(f"{installer.name}: signed with key {parts['key_id'][::-1].hex().upper()}, "
                             f"not the update key {key_id[::-1].hex().upper()}")
    signed_name = parts["fields"].get("file")
    if signed_name is not None and signed_name != (name or installer.name):
        raise SignatureError(f"{name or installer.name}: the signature is for {signed_name}")
    try:
        from cryptography.exceptions import InvalidSignature
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
    except ImportError:
        return False
    if parts["algorithm"] == b"ED":
        digest = hashlib.blake2b()
        with installer.open("rb") as file:
            for chunk in iter(lambda: file.read(1 << 20), b""):
                digest.update(chunk)
        message = digest.digest()
    else:
        message = installer.read_bytes()
    verifier = Ed25519PublicKey.from_public_bytes(key)
    try:
        verifier.verify(parts["signature"], message)
        verifier.verify(parts["global"], parts["signature"] + parts["trusted"].encode())
    except InvalidSignature as error:
        raise SignatureError(f"{name or installer.name}: the signature does not match these bytes") from error
    return True


def read_record(installer: Path) -> dict | None:
    """The committed set for `installer`, or None when there is none."""
    try:
        record = json.loads(record_path(installer).read_text(encoding="utf-8"))
    except FileNotFoundError:
        return None
    except (OSError, ValueError) as error:
        raise SignatureError(f"{record_path(installer).name}: unreadable ({error})") from error
    if not (isinstance(record, dict) and record.get("file") == installer.name
            and isinstance(record.get("size"), int) and isinstance(record.get("sha256"), str)
            and isinstance(record.get("signature"), str) and record["signature"].strip()):
        raise SignatureError(f"{record_path(installer).name}: incomplete release record")
    return record


def record_bytes(name: str, size: int, sha256: str, signature: str) -> bytes:
    return (json.dumps({"file": name, "size": size, "sha256": sha256, "signature": signature.strip()},
                       indent=2) + "\n").encode()


def commit(built: Path, destination: Path, public_key: str) -> dict:
    """Moves a signed installer (`built` and `built.sig`, in a temporary
    directory on the same filesystem) into `destination` as one release set.

    Order, each step a single rename of a complete, synced file:
      1. remove the old `.sig`: a reader of the plain pair now sees "unsigned",
         never the old signature with the new installer, nor the new
         signature with the old installer;
      2. installer;  3. `.sig`;
      4. the record, last: the commit point. Until it lands the old record
         names the old SHA-256, so record readers see a mismatch and retry.
    A failure before step 4 leaves the previous record, and readers keep
    refusing the half-replaced pair instead of publishing it.
    """
    built_sig = signature_path(built)
    signature = built_sig.read_text(encoding="utf-8").strip()
    verify_signature(built, signature, public_key)
    size, digest = file_digest(built)
    os.chmod(built, 0o644)
    os.chmod(built_sig, 0o644)
    fsync_file(built)
    fsync_file(built_sig)
    final = destination / built.name
    pending_record = built.with_name(built.name + RECORD_SUFFIX)
    write_atomic(pending_record, record_bytes(final.name, size, digest, signature))
    signature_path(final).unlink(missing_ok=True)
    fsync_dir(destination)
    os.replace(built, final)
    os.replace(built_sig, signature_path(final))
    os.replace(pending_record, record_path(final))
    fsync_dir(destination)
    return {"file": final.name, "size": size, "sha256": digest, "signature": signature}


def seal(installer: Path, public_key: str) -> dict:
    """Writes the record for an existing installer and `.sig` pair (built before records existed)."""
    signature = signature_path(installer).read_text(encoding="utf-8").strip()
    size, digest = file_digest(installer)
    verified = verify_signature(installer, signature, public_key)
    if (file_digest(installer) != (size, digest)
            or signature_path(installer).read_text(encoding="utf-8").strip() != signature):
        raise SignatureError(f"{installer.name}: the installer or signature changed while sealing; retry")
    write_atomic(record_path(installer), record_bytes(installer.name, size, digest, signature))
    return {"file": installer.name, "size": size, "sha256": digest, "verified": verified}


def updater_public_key(root: Path) -> str:
    """`plugins.updater.pubkey` of the Tauri build: the key the launcher also embeds."""
    config = json.loads((root / "desktop" / "src-tauri" / "tauri.conf.json").read_text(encoding="utf-8"))
    return config["plugins"]["updater"]["pubkey"]
