"""Bounded process evidence with independent pipe and process lifecycles.

No shell, closed stdin, deterministic locale, bounded combined stdout/stderr.
A child closing its streams does not shorten the requested deadline. On failure
we kill the process group and drain for a fixed grace period, independent of the
original timeout. Escaped sessions are outside process-group containment; their
inherited pipes cannot hold this reader indefinitely.
"""
from __future__ import annotations

import hashlib
import math
import os
import selectors
import signal
import subprocess
import time

LIMIT = 256 * 1024
DRAIN_GRACE = 0.5


def run(argv, timeout, *, env=None, output_limit=LIMIT):
    if (type(timeout) not in (int, float) or not math.isfinite(timeout)
            or timeout <= 0 or type(output_limit) is not int or output_limit < 1):
        raise ValueError('positive finite process timeout/output budget required')
    started = time.monotonic()
    deadline = started + timeout
    environment = dict(os.environ, LC_ALL='C', LANG='C', GCC_COLORS='', NO_COLOR='1')
    if env is not None:
        environment.update(env)
    out, err = bytearray(), bytearray()
    reason = None
    failed_at = None
    proc = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                            stderr=subprocess.PIPE, env=environment,
                            start_new_session=True)

    def terminate():
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass

    def fail(kind):
        nonlocal reason, failed_at
        if reason is None:
            reason, failed_at = kind, time.monotonic()
            terminate()

    try:
        with selectors.DefaultSelector() as selector:
            for stream, target in ((proc.stdout, out), (proc.stderr, err)):
                os.set_blocking(stream.fileno(), False)
                selector.register(stream, selectors.EVENT_READ, target)
            while selector.get_map() or proc.poll() is None:
                now = time.monotonic()
                if reason is None and now >= deadline:
                    fail('timeout')
                if failed_at is not None and now >= failed_at + DRAIN_GRACE:
                    break
                wait = min(.05, max(0., (failed_at + DRAIN_GRACE
                                        if failed_at is not None else deadline) - now))
                if not selector.get_map():
                    # EOF on both streams is not process completion. Wait only
                    # to our deadline, not an unrelated fixed two-second cap.
                    try:
                        proc.wait(timeout=wait)
                    except subprocess.TimeoutExpired:
                        pass
                    continue
                for key, _ in selector.select(wait):
                    try:
                        block = os.read(key.fd, 65536)
                    except BlockingIOError:
                        continue
                    if not block:
                        selector.unregister(key.fileobj)
                        continue
                    available = max(0, output_limit - len(out) - len(err))
                    key.data.extend(block[:available])
                    if len(block) > available:
                        fail('output-limit')
        if proc.poll() is None:
            terminate()
        proc.wait(timeout=2)
    except BaseException:
        terminate()
        proc.wait(timeout=2)
        raise
    finally:
        proc.stdout.close()
        proc.stderr.close()
    return dict(returncode=proc.returncode, stdout=bytes(out), stderr=bytes(err),
                failure=reason, elapsed=time.monotonic() - started,
                stdout_sha256=hashlib.sha256(out).hexdigest(),
                stderr_sha256=hashlib.sha256(err).hexdigest(),
                retained_bytes=len(out) + len(err))


def normalize(result):
    """Validate captured evidence; an injected malformed result is not a PASS."""
    if not isinstance(result, dict):
        try:
            rc, out, err = result
        except (TypeError, ValueError) as exc:
            raise ValueError('malformed process evidence tuple') from exc
        out = out.encode() if isinstance(out, str) else out
        err = err.encode() if isinstance(err, str) else err
        if not isinstance(out, bytes) or not isinstance(err, bytes):
            raise ValueError('process channel must retain raw bytes')
        result = dict(returncode=rc, stdout=out, stderr=err, failure=None, elapsed=0,
                      stdout_sha256=hashlib.sha256(out).hexdigest(),
                      stderr_sha256=hashlib.sha256(err).hexdigest())
    required = {'returncode', 'stdout', 'stderr', 'failure', 'elapsed',
                'stdout_sha256', 'stderr_sha256'}
    if not required <= result.keys():
        raise ValueError('incomplete process evidence')
    if result['returncode'] is not None and type(result['returncode']) is not int:
        raise ValueError('invalid raw process return code')
    if (type(result['elapsed']) not in (int, float) or not math.isfinite(result['elapsed'])
            or result['elapsed'] < 0):
        raise ValueError('invalid process elapsed evidence')
    if result['failure'] is not None and (not isinstance(result['failure'], str) or not result['failure']):
        raise ValueError('invalid process resource state')
    for channel in ('stdout', 'stderr'):
        if not isinstance(result[channel], bytes):
            raise ValueError('process channel must retain raw bytes')
        if result[channel + '_sha256'] != hashlib.sha256(result[channel]).hexdigest():
            raise ValueError('process channel hash does not identify retained bytes')
    return result
