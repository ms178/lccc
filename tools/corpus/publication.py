"""Fail-closed publication with explicit visibility/durability commit points.

Rename/exchange is the visibility commit. A subsequent parent-directory fsync
failure cannot truthfully promise that the previous generation stayed live:
PublicationError(committed=True) records complete new visibility but uncertain
crash durability. Pre-commit errors preserve the previous live generation.
"""
from __future__ import annotations

from contextlib import contextmanager
import ctypes
import errno
import math
import os
from pathlib import Path
import stat
import tempfile
import time


class PublicationError(OSError):
    def __init__(self, message, *, committed):
        self.committed = committed
        state = 'new generation visible; durability unconfirmed' if committed else 'not committed'
        super().__init__(f'{message} ({state})')


@contextmanager
def lock(path: Path, *, timeout=30.):
    """Bounded same-user writer serialization, without following lock symlinks."""
    import fcntl
    if type(timeout) not in (int, float) or not math.isfinite(timeout) or timeout <= 0:
        raise ValueError('positive finite lock timeout required')
    path.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_CLOEXEC | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'a+b') as stream:
        if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
            raise ValueError('publication lock must be a regular file')
        deadline = time.monotonic() + timeout
        while True:
            try:
                fcntl.flock(stream.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                if time.monotonic() >= deadline:
                    raise TimeoutError('publication writer lock deadline exceeded')
                time.sleep(min(.05, max(0., deadline - time.monotonic())))
        try:
            yield
        finally:
            fcntl.flock(stream.fileno(), fcntl.LOCK_UN)


def sync_dir(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def _committed_sync(parent):
    try:
        sync_dir(parent)
    except OSError as exc:
        raise PublicationError(str(exc), committed=True) from exc


def atomic_bytes(destination: Path, data: bytes):
    if destination.is_symlink() or (destination.exists() and not destination.is_file()):
        raise ValueError('publication destination must be a regular non-symlink file')
    fd, name = tempfile.mkstemp(prefix='.' + destination.name + '-', suffix='.tmp',
                                dir=destination.parent)
    temporary = Path(name)
    try:
        with os.fdopen(fd, 'wb') as stream:
            os.fchmod(stream.fileno(), 0o644)
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())  # data AND published mode before visibility
        os.replace(temporary, destination)
        _committed_sync(destination.parent)
    finally:
        temporary.unlink(missing_ok=True)


def exchange(staged: Path, live: Path):
    libc = ctypes.CDLL(None, use_errno=True)
    try:
        call = libc.renameat2
    except AttributeError as exc:
        raise OSError(errno.ENOSYS, 'atomic directory exchange unavailable') from exc
    call.argtypes = [ctypes.c_int, ctypes.c_char_p, ctypes.c_int, ctypes.c_char_p, ctypes.c_uint]
    call.restype = ctypes.c_int
    if call(-100, os.fsencode(staged), -100, os.fsencode(live), 2) != 0:
        code = ctypes.get_errno()
        raise OSError(code, os.strerror(code))
    _committed_sync(live.parent)


def publish_directory(staged: Path, live: Path, *, force: bool):
    if live.is_symlink():
        raise ValueError('live corpus path must not be a symlink')
    if not staged.is_dir() or staged.is_symlink():
        raise ValueError('staged corpus must be a real directory')
    if staged.parent.resolve() != live.parent.resolve():
        raise ValueError('directory publication requires same-parent staging')
    if live.exists():
        if not live.is_dir():
            raise ValueError('live corpus must be a directory')
        if not force:
            raise FileExistsError(live)
        exchange(staged, live)  # old complete generation remains at staged
    else:
        os.replace(staged, live)
        _committed_sync(live.parent)
