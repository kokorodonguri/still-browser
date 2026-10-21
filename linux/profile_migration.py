"""One-time sidebar-only migration of Firefox's saved window state.

Mozilla's jsonlz4 format is an eight-byte magic, uint32 length, and an LZ4
block. Write a valid literal-only block to avoid an extra runtime dependency.
Firefox recompresses the session on its next save. Originals are backed up.
"""
import json
import struct

MAGIC = b"mozLz40\0"
LIMIT = 64 * 1024 * 1024


def decode_session(data):
    if data[:8] != MAGIC or len(data) < 12:
        raise ValueError("Invalid Firefox session header")
    size = struct.unpack_from("<I", data, 8)[0]
    if size > LIMIT:
        raise ValueError("Firefox session is too large to migrate safely")
    output = bytearray()
    position = 12

    def length(base):
        nonlocal position
        total = base
        if base == 15:
            while True:
                if position >= len(data):
                    raise ValueError("Truncated session length")
                byte = data[position]
                position += 1
                total += byte
                if total > LIMIT:
                    raise ValueError("Invalid session length")
                if byte != 255:
                    break
        return total

    while position < len(data):
        token = data[position]
        position += 1
        literals = length(token >> 4)
        if position + literals > len(data) or len(output) + literals > size:
            raise ValueError("Invalid session literals")
        output.extend(data[position:position + literals])
        position += literals
        if position == len(data):
            break
        if position + 2 > len(data):
            raise ValueError("Truncated session offset")
        offset = int.from_bytes(data[position:position + 2], "little")
        position += 2
        if not 0 < offset <= len(output):
            raise ValueError("Invalid session offset")
        count = length(token & 15) + 4
        if len(output) + count > size:
            raise ValueError("Session exceeds declared length")
        # Repeat the prior pattern to support overlapping LZ4 matches.
        start = len(output) - offset
        if count <= offset:
            # A four-byte match can have a 64 KiB offset. Copy only the match,
            # not the entire suffix on every such token.
            output.extend(output[start:start + count])
        else:
            pattern = output[start:]
            output.extend((pattern * ((count + offset - 1) // offset))[:count])
    if len(output) != size:
        raise ValueError("Session length mismatch")
    return json.loads(output)


def encode_session(value):
    raw = json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()
    if len(raw) > LIMIT:
        raise ValueError("Firefox session is too large to migrate safely")
    block = bytearray([min(len(raw), 15) << 4])
    if len(raw) >= 15:
        remaining = len(raw) - 15
        while remaining >= 255:
            block.append(255)
            remaining -= 255
        block.append(remaining)
    return MAGIC + struct.pack("<I", len(raw)) + block + raw


def stage_sessions(profile):
    staged = []
    candidates = [profile / "sessionstore.jsonlz4"]
    backup = profile / "sessionstore-backups"
    candidates += [backup / name for name in ("recovery.jsonlz4", "recovery.baklz4", "previous.jsonlz4")]
    for path in candidates:
        if not path.exists():
            continue
        original = path.read_bytes()
        state = decode_session(original)
        if not isinstance(state, dict) or not isinstance(state.get("windows", []), list):
            raise ValueError("Invalid Firefox session structure")
        for window in state.get("windows", []):
            sidebar = window.setdefault("sidebar", {})
            sidebar.update(launcherVisible=False, launcherExpanded=True, panelOpen=False)
            sidebar.pop("hidden", None)
        staged.append((path, original, encode_session(state)))
    return staged
