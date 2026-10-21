"""Verify updater exclusion across processes and installation renames."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import update_swap


class UpdateLockChecks(unittest.TestCase):
    def test_second_process_cannot_enter_during_a_directory_swap(self):
        script = """
import sys
from pathlib import Path
import update_swap
try:
    with update_swap.locked(Path(sys.argv[1])):
        print('acquired')
except update_swap.BusyError:
    print('busy')
"""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / "Still"
            previous = Path(directory) / "Still.previous"
            root.mkdir()

            def child():
                return subprocess.run(
                    [sys.executable, "-c", script, str(root)],
                    cwd=Path(update_swap.__file__).parent,
                    capture_output=True, text=True, check=True,
                ).stdout.strip()

            with update_swap.locked(root):
                self.assertEqual(child(), "busy")
                root.rename(previous)
                self.assertEqual(child(), "busy")
                previous.rename(root)
            self.assertEqual(child(), "acquired")


if __name__ == "__main__":
    unittest.main()
