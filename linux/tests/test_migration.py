"""Exercise real LZ4 matches, including small matches with a large offset."""
import struct
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from profile_migration import MAGIC, decode_session


def matched_session(offset=65535, matches=10000):
    prefix = b'{"value":"' + b'a' * offset
    block = bytearray([min(len(prefix), 15) << 4])
    if len(prefix) >= 15:
        remaining = len(prefix) - 15
        while remaining >= 255:
            block.append(255)
            remaining -= 255
        block.append(remaining)
    block.extend(prefix)
    block.extend(struct.pack('<H', offset))
    block.extend((b'\0' + struct.pack('<H', offset)) * (matches - 1))
    block.extend(b'\x80aaaaaa"}')
    return MAGIC + struct.pack('<I', len(prefix) + 4 * matches + 8) + block


class Matches(unittest.TestCase):
    def test_small_matches_with_large_offsets(self):
        self.assertEqual(decode_session(matched_session())["value"], "a" * (65535 + 40000 + 6))

    def test_overlapping_matches(self):
        for offset in (1, 2, 3, 4, 5):
            with self.subTest(offset=offset):
                self.assertEqual(decode_session(matched_session(offset, 50))["value"], "a" * (offset + 200 + 6))

    def test_declared_size_is_still_enforced(self):
        data = bytearray(matched_session(3, 5))
        struct.pack_into('<I', data, 8, 1)
        with self.assertRaises(ValueError):
            decode_session(data)

    def test_truncated_match_is_rejected(self):
        with self.assertRaises(ValueError):
            decode_session(matched_session()[:-4])


if __name__ == '__main__':
    unittest.main(verbosity=2)
