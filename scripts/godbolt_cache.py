#!/usr/bin/env python3
"""One persistent Compiler Explorer cache, shared by every oracle tool.

WHY THIS EXISTS
---------------
Four tools in this tree ask Compiler Explorer for the same
(compiler, flags, source) tuples:

    scripts/codegen_oracle.py        static instruction survey (``--rank``)
    scripts/encdiff.py               encoding diff
    scripts/oracle_asm.py            offline per-function asm dump
    tools/oracle/godbolt_oracle.py   execution oracle (semantics + size)

Each grew its own memoisation, and the two designs disagreed:

  * ``codegen_oracle`` and ``encdiff`` persist to ``.godbolt-cache/`` on disk,
    so a second run is offline.
  * ``tools/oracle/godbolt_oracle.py`` memoised in a process-wide ``dict``, so
    its cache evaporated when the process exited and every sweep re-issued
    every request.

That second point is not a stylistic wart. Compiler Explorer rate-limits hard
(HTTP 429), and the execution oracle issues TWO requests per compiler per
program -- an asm pass and an execute pass, because the API returns assembly
only from a non-executing request. A full sweep is therefore the most
rate-limit-expensive job in the tree, and it was the one job with no
persistence: it could not be resumed after a 429, could not be re-run offline
to reproduce an already-published table, and could not run in CI at all.

So: one cache, one policy, four callers.

CONTRACT
--------
NAMESPACED.  Every record is keyed by ``(namespace, *parts)``. The namespace
is a *format contract*, not a label: ``att-v2`` exists because a pre-v2 cache
stored Intel syntax, which made AT&T load/store metrics silently read as zero
in a table that then looked like a triumph. When a record's shape changes,
bump the namespace -- that is how a format change invalidates old records
instead of poisoning new measurements.

ATOMIC.  Writers write a sibling temp file and ``os.replace`` it into
position, so a crashed, killed or Ctrl-C'd sweep can never leave a truncated
record that later reads as a hit.

CORRUPTION-TOLERANT.  An unreadable or unparseable record is a MISS, never an
exception. A cache that can abort a sweep is strictly worse than no cache.

CONCURRENT-SAFE.  ``--rank`` fans out over threads. Temp files are named per
(pid, thread) so two workers cannot collide, and the rename is the only
mutation visible to readers.

RECORD-SHAPED, NOT BLOB-SHAPED.  Text and JSON are separate entry points so a
caller cannot store assembly under a JSON namespace and get a silently
stringified record back.
"""
from __future__ import annotations

import hashlib
import json
import os
import threading
from pathlib import Path
from typing import Any

# Resolved identically by every importer: this file lives in <repo>/scripts/,
# so the cache sits at <repo>/.godbolt-cache/ no matter which tool imports it
# or what the process working directory is.
CACHE = Path(os.environ.get(
    "GODBOLT_CACHE",
    str(Path(__file__).resolve().parent.parent / ".godbolt-cache"),
))

# STALENESS, NOT VALIDITY
# -----------------------
# The key is (cid, flags, source, execute) and deliberately carries no compiler
# version.  That is the right trade for this cache: a published table must stay
# reproducible, and pinning a version into the key would force a probe request
# before every compile -- double the requests against an API that rate-limits.
#
# The cost is that a *versioned* upstream channel (ICX most of all) can move
# while a record stays "valid".  `tools/oracle/godbolt_oracle.py` closes that
# gap from the other side: it records the Compiler Explorer version string
# INSIDE the record and compares it against one cheap probe per compiler per
# process, treating a mismatch as a miss so the sweep re-measures and
# overwrites.  A record written before that field existed has `ce_version ==
# None` and is still honoured -- invalidating every unversioned record at once
# would be a far larger, and much less safe, change than the problem.
#
# To force a generation to be dropped wholesale after a format change, bump
# the namespace below; that is the sanctioned way, and it is the right tool
# when the KEY changes rather than when the compiler does.
#
# Namespaces currently in use. Adding one is the sanctioned way to invalidate
# a generation of records after a format change.
# v3/v2: the compiler's reported version (godbolt.compiler_fingerprint) is now
# part of every remote key. A CE id is a label, not a binary -- when `g162` is
# rebuilt against a newer GCC the id is unchanged but the code it emits is not,
# and a v2 record would then keep serving the old build's output under the new
# build's name. Bumping moves every pre-fingerprint record out of the reachable
# key space in one explicit step instead of leaving it to be mistaken for a
# current result. Bump these again on any future change to a key's meaning.
NS_ASM = "att-v3"          # AT&T-syntax assembly, text, one file per tuple
NS_ORACLE = "oracle-v1"    # execution-oracle records, JSON
NS_ENCDIFF = "encdiff-v2"  # raw CE /compile replies incl. binary opcodes, JSON


def key(namespace: str, *parts: Any) -> str:
    """Stable digest for ``(namespace, *parts)``.

    Parts are joined with NUL rather than a printable separator so that no
    choice of source text or flags can forge a boundary: ``("a,b", "c")`` and
    ``("a", "b,c")`` must not collide, and with a comma separator they would.
    """
    h = hashlib.sha256()
    h.update(namespace.encode("utf-8"))
    for part in parts:
        h.update(b"\0")
        h.update(str(part).encode("utf-8"))
    return h.hexdigest()[:32]


def path_for(namespace: str, *parts: Any) -> Path:
    """On-disk path for a record, without touching the filesystem."""
    return CACHE / namespace / (key(namespace, *parts) + ".json")


def _atomic_write(path: Path, data: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(
        f"{path.stem}.tmp.{os.getpid()}.{threading.get_ident()}{path.suffix}"
    )
    tmp.write_text(data, encoding="utf-8")
    os.replace(tmp, path)


# --------------------------------------------------------------------------
# JSON records (execution oracle results, compiler metadata, ...)
# --------------------------------------------------------------------------

def load_json(namespace: str, *parts: Any) -> Any | None:
    """Return the cached JSON value, or ``None`` on miss or corruption.

    Corruption is a miss on purpose. A truncated file from a killed sweep is
    the common case, and raising here would turn a recoverable cache miss into
    a dead sweep.
    """
    path = path_for(namespace, *parts)
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return None
    except (OSError, ValueError, UnicodeDecodeError):
        return None


def store_json(namespace: str, value: Any, *parts: Any) -> None:
    """Write ``value`` as JSON, atomically."""
    _atomic_write(path_for(namespace, *parts), json.dumps(value))


# --------------------------------------------------------------------------
# Text records (assembly bodies)
# --------------------------------------------------------------------------

def load_text(namespace: str, *parts: Any) -> str | None:
    """Return the cached text, or ``None`` on miss or corruption.

    Decoding is STRICT, exactly like :func:`load_json`. ``errors="replace"`
    would turn a record corrupted by anything other than truncation into
    U+FFFD mush and report it as a *hit* -- a silent wrong answer from a
    cache whose whole contract is that corruption is a miss. Atomic rename
    makes truncation impossible, but hand-edits, disk rot and partial
    network-sync are neither truncation nor impossible.
    """
    path = path_for(namespace, *parts)
    try:
        return path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return None
    except (OSError, UnicodeDecodeError):
        return None


def store_text(namespace: str, text: str, *parts: Any) -> None:
    """Write ``text`` atomically."""
    _atomic_write(path_for(namespace, *parts), text)


def load_lines(namespace: str, *parts: Any) -> list[str] | None:
    """Cached text split into lines, or ``None`` on miss. Never ``[""]``."""
    text = load_text(namespace, *parts)
    return None if text is None else text.splitlines()


def store_lines(namespace: str, lines: list[str], *parts: Any) -> None:
    """Write ``lines`` joined by newlines, atomically."""
    store_text(namespace, "\n".join(lines), *parts)


# --------------------------------------------------------------------------
# Introspection
# --------------------------------------------------------------------------

def _is_record(f: Path) -> bool:
    """True for a committed record, false for an in-flight temp file.

    ``_atomic_write`` names temps ``<stem>.tmp.<pid>.<tid><suffix>`` -- they keep
    the record's own suffix, so filtering on ``endswith(".tmp")`` matched none
    of them and counted every half-written file as a record.  The marker is the
    infix, so match that.
    """
    return f.is_file() and ".tmp." not in f.name


def stats() -> dict[str, int]:
    """Record count per namespace. Used by the self-test and by ``--cache``."""
    out: dict[str, int] = {}
    if not CACHE.is_dir():
        return out
    for ns in sorted(p.name for p in CACHE.iterdir() if p.is_dir()):
        out[ns] = sum(1 for f in (CACHE / ns).iterdir() if _is_record(f))
    return out
