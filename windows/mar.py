"""Read Mozilla complete MAR archives without buffering their runtime files."""

import lzma
from pathlib import Path, PurePosixPath, PureWindowsPath
import struct
import tempfile


CHUNK_SIZE = 64 * 1024


class MarError(ValueError):
    """An archive has an invalid index, path or compressed member."""


def _read_exact(archive, size):
    data = archive.read(size)
    if len(data) != size:
        raise MarError("truncated archive")
    return data


def _index(archive):
    archive.seek(0, 2)
    archive_size = archive.tell()
    archive.seek(0)
    magic, index = struct.unpack(">4sI", _read_exact(archive, 8))
    if magic != b"MAR1":
        raise MarError("not a Mozilla update archive")
    if not 8 <= index <= archive_size - 4:
        raise MarError("index is outside the archive")
    archive.seek(index)
    size = struct.unpack(">I", _read_exact(archive, 4))[0]
    end = index + 4 + size
    if end > archive_size:
        raise MarError("truncated archive index")
    entries = []
    while archive.tell() < end:
        if end - archive.tell() < 13:
            raise MarError("truncated index entry")
        offset, length, _mode = struct.unpack(">III", _read_exact(archive, 12))
        name = bytearray()
        while archive.tell() < end:
            byte = _read_exact(archive, 1)
            if byte == b"\0":
                break
            name.extend(byte)
            if len(name) > 4096:
                raise MarError("archive path is too long")
        else:
            raise MarError("unterminated archive path")
        try:
            name = name.decode("utf-8")
        except UnicodeDecodeError as error:
            raise MarError("archive path is not UTF-8") from error
        if (not name or "\\" in name or ":" in name or
                PurePosixPath(name).is_absolute() or PureWindowsPath(name).drive or
                any(part in ("", ".", "..") for part in name.split("/"))):
            raise MarError(f"unexpected archive path: {name!r}")
        if offset < 8 or length == 0 or offset + length > index:
            raise MarError(f"member is outside the archive data: {name}")
        entries.append((name, offset, length))
    return entries


def _extract_member(archive, output, offset, length):
    archive.seek(offset)
    decoder = lzma.LZMADecompressor()
    remaining = length
    try:
        while remaining:
            compressed = _read_exact(archive, min(CHUNK_SIZE, remaining))
            remaining -= len(compressed)
            while True:
                output.write(decoder.decompress(compressed, max_length=CHUNK_SIZE))
                compressed = b""
                if decoder.eof:
                    if decoder.unused_data or remaining:
                        raise MarError("unexpected data after compressed member")
                    return
                if decoder.needs_input:
                    break
    except lzma.LZMAError as error:
        raise MarError("invalid compressed member") from error
    raise MarError("truncated compressed member")


def extract_mar(mar: Path, destination: Path):
    """Extract the indexed XZ members, checking bounds and paths before writing."""
    root = destination.resolve()
    with mar.open("rb") as archive:
        entries = _index(archive)
        targets = []
        for name, offset, length in entries:
            target = (root / name).resolve()
            if root not in target.parents:
                raise MarError(f"unexpected archive path: {name!r}")
            targets.append((target, offset, length))
        for target, offset, length in targets:
            target.parent.mkdir(parents=True, exist_ok=True)
            temporary = None
            try:
                with tempfile.NamedTemporaryFile(dir=target.parent, prefix=".still-mar-", delete=False) as output:
                    temporary = Path(output.name)
                    _extract_member(archive, output, offset, length)
                temporary.replace(target)
            finally:
                if temporary is not None:
                    temporary.unlink(missing_ok=True)
