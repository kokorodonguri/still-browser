"""Exercise real MAR parsing and bounded decompression with offline fixtures."""

import hashlib
import importlib.util
import io
import lzma
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("still_mar_test", ROOT / "windows/mar.py")
mar = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mar)


def archive_bytes(members):
    data = bytearray(b"MAR1\0\0\0\0")
    index = bytearray()
    for name, content in members:
        compressed = lzma.compress(content)
        index.extend(struct.pack(">III", len(data), len(compressed), 0o644))
        index.extend(name.encode() + b"\0")
        data.extend(compressed)
    struct.pack_into(">I", data, 4, len(data))
    return data + struct.pack(">I", len(index)) + index


class MarChecks(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.archive = self.root / "fixture.mar"
        self.destination = self.root / "firefox"

    def extract(self, data):
        self.archive.write_bytes(data)
        mar.extract_mar(self.archive, self.destination)

    def test_extracts_original_bytes_without_whole_archive_or_member_helpers(self):
        members = [("firefox.exe", bytes(range(256)) * 1000), ("browser/omni.ja", b"resources\0\xff")]
        data = archive_bytes(members)
        with patch.object(Path, "read_bytes", side_effect=AssertionError("whole file read")), \
                patch.object(lzma, "decompress", side_effect=AssertionError("whole member decompression")):
            self.extract(data)
        for name, content in members:
            self.assertEqual((self.destination / name).read_bytes(), content)
        self.assertFalse(list(self.destination.rglob(".still-mar-*")))

    def test_large_member_uses_bounded_reads_and_output_chunks(self):
        # A small archive expands to 32 MiB, making an unbounded decoder visible.
        block = bytes(range(256)) * 256
        compressor = lzma.LZMACompressor()
        compressed = bytearray()
        expected = hashlib.sha256()
        for _ in range(512):
            compressed.extend(compressor.compress(block))
            expected.update(block)
        compressed.extend(compressor.flush())

        class BoundedInput(io.BytesIO):
            def read(inner, size=-1):
                self.assertGreater(size, 0)
                self.assertLessEqual(size, mar.CHUNK_SIZE)
                return super().read(size)

        class DigestOutput:
            count = 0
            digest = hashlib.sha256()

            def write(inner, data):
                self.assertLessEqual(len(data), mar.CHUNK_SIZE)
                inner.count += len(data)
                inner.digest.update(data)

        output = DigestOutput()
        mar._extract_member(BoundedInput(compressed), output, 0, len(compressed))
        self.assertEqual(output.count, 32 * 1024 * 1024)
        self.assertEqual(output.digest.digest(), expected.digest())

    def test_rejects_truncated_header_and_invalid_magic(self):
        for data in (b"", b"MAR1", b"BAD!\0\0\0\x08\0\0\0\0"):
            with self.subTest(data=data), self.assertRaises(mar.MarError):
                self.extract(data)

    def test_rejects_index_outside_archive(self):
        for position in (0, 7, 0xFFFFFFFF):
            data = archive_bytes([("file", b"content")])
            struct.pack_into(">I", data, 4, position)
            with self.subTest(position=position), self.assertRaises(mar.MarError):
                self.extract(data)

    def test_rejects_truncated_index(self):
        data = archive_bytes([("file", b"content")])
        with self.assertRaises(mar.MarError):
            self.extract(data[:-1])

    def test_index_name_must_terminate_inside_index(self):
        data = archive_bytes([("file", b"content")])
        data[-1] = ord("x")
        with self.assertRaises(mar.MarError):
            self.extract(data)

    def test_rejects_short_index_entry(self):
        data = b"MAR1" + struct.pack(">II", 8, 12) + bytes(12)
        with self.assertRaises(mar.MarError):
            self.extract(data)

    def test_rejects_member_bounds_before_creating_files(self):
        for offset, length in ((0, 8), (8, 0), (8, 0xFFFFFFFF)):
            data = archive_bytes([("file", b"content")])
            index = struct.unpack_from(">I", data, 4)[0]
            struct.pack_into(">II", data, index + 4, offset, length)
            with self.subTest(offset=offset, length=length), self.assertRaises(mar.MarError):
                self.extract(data)
        self.assertFalse(self.destination.exists())

    def test_rejects_paths_on_both_unix_and_windows(self):
        for name in ("", "../outside", "/absolute", "C:/outside", "C:outside", "a\\..\\outside",
                     "a/../outside", "a//file", "./file", "file:stream"):
            with self.subTest(name=name), self.assertRaises(mar.MarError):
                self.extract(archive_bytes([("good", b"valid"), (name, b"invalid")]))
            self.assertFalse(self.destination.exists())
        self.assertFalse((self.root / "outside").exists())

    def test_rejects_symlink_escape(self):
        self.destination.mkdir()
        (self.destination / "linked").symlink_to(self.root, target_is_directory=True)
        with self.assertRaises(mar.MarError):
            self.extract(archive_bytes([("linked/outside", b"invalid")]))
        self.assertFalse((self.root / "outside").exists())

    def test_truncated_compressed_member_leaves_existing_file_unchanged(self):
        data = archive_bytes([("file", b"content" * 1000)])
        index = struct.unpack_from(">I", data, 4)[0]
        length = struct.unpack_from(">I", data, index + 8)[0]
        struct.pack_into(">I", data, index + 8, length - 1)
        self.destination.mkdir()
        (self.destination / "file").write_bytes(b"existing")
        with self.assertRaises(mar.MarError):
            self.extract(data)
        self.assertEqual((self.destination / "file").read_bytes(), b"existing")
        self.assertFalse(list(self.destination.glob(".still-mar-*")))

    def test_rejects_corrupt_compressed_data(self):
        data = archive_bytes([("file", b"content")])
        data[8:14] = b"broken"
        with self.assertRaises(mar.MarError):
            self.extract(data)
        self.assertFalse((self.destination / "file").exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)
