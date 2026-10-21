"""Filesystem activation of an already verified Still update.

Version checks and downloads belong to updater.py. This module owns the
directory swap and its rollback; callers must first ensure Firefox is stopped.
The backup is kept until the new installation, runtime and policies are ready.
"""
from contextlib import contextmanager
import fcntl
from pathlib import Path
import shutil
import tempfile


class BusyError(Exception):
    """Another process is downloading or activating this installation's update."""


@contextmanager
def locked(root: Path):
    """Serialize update writers using an inode outside the renamed directories."""
    path = root.with_name("." + root.name + ".update.lock")
    # Keep this file: unlinking it would let two processes lock different inodes.
    with path.open("a") as lock:
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise BusyError from error
        try:
            yield
        finally:
            fcntl.flock(lock, fcntl.LOCK_UN)


POLICY_BACKUP = ".still-policy.previous"
POLICY_ABSENT = ".still-policy.absent"


def _prepare_policies(folder: Path) -> Path:
    """Finish allocating/writing the new policy before changing installation."""
    content = (folder / "policies.json").read_bytes()
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=folder, prefix=".still-policy-", delete=False) as output:
            temporary = Path(output.name)
            output.write(content)
        return temporary
    except (OSError, KeyboardInterrupt):
        if temporary is not None:
            temporary.unlink(missing_ok=True)
        raise


def recover(root: Path, staged: Path, previous: Path) -> None:
    """Finish cleanup after a commit, or restore an uncommitted installation.

    Also used for exceptions in activate(). If the retained Firefox was moved
    into staging, return it before restoring the old directory and its policy.
    Recovery itself can be retried if the filesystem remains unavailable.
    """
    if not previous.exists():
        return
    if root.exists():
        # The final rename committed a complete new installation.
        shutil.rmtree(previous, ignore_errors=True)
        return
    runtime = previous / "runtime"
    if not runtime.exists() and (staged / "runtime").exists():
        (staged / "runtime").rename(runtime)
    policy = runtime / "firefox" / "distribution" / "policies.json"
    backup = previous / POLICY_BACKUP
    absent = previous / POLICY_ABSENT
    if backup.exists():
        # Rename the original file back: rollback needs no file allocation or
        # write, and preserves policies modified since the last install.
        backup.replace(policy)
    elif absent.exists():
        policy.unlink(missing_ok=True)
    absent.unlink(missing_ok=True)
    previous.rename(root)


def activate(root: Path, staged: Path, previous: Path, *, keep_runtime: bool) -> None:
    """Commit staging, rolling back recoverable errors and keyboard interrupts.

    No copy of the large Firefox runtime is needed. A failed update may leave
    staging without its bundled runtime; retrying can retain the restored one.
    """
    pending = None
    try:
        if keep_runtime:
            pending = _prepare_policies(staged)
        root.rename(previous)
        if keep_runtime:
            if (staged / "runtime").exists():
                shutil.rmtree(staged / "runtime")
            (previous / "runtime").rename(staged / "runtime")
            policy = staged / "runtime" / "firefox" / "distribution" / "policies.json"
            policy.parent.mkdir(parents=True, exist_ok=True)
            if policy.exists():
                policy.rename(previous / POLICY_BACKUP)
            else:
                (previous / POLICY_ABSENT).touch(exist_ok=False)
            pending.replace(policy)
        staged.rename(root)
    except (OSError, KeyboardInterrupt):
        recover(root, staged, previous)
        raise
    finally:
        if pending is not None:
            pending.unlink(missing_ok=True)
    shutil.rmtree(previous, ignore_errors=True)
