"""One definition of what a torture case *is*, shared by producer and consumer.

`scripts/x86_gcc_torture.py` writes the report and `scripts/torture_evidence.py`
judges it.  Both need to name a case, and the consumer's whole power to detect
a truncated or doctored report comes from comparing its own idea of the case
set against the producer's.  Two implementations of that identity is how a
drift becomes invisible: the producer would hash `a.c -O2` while the consumer
expected `a.c[-O2]`, and every report would then look corrupted -- or, worse,
the consumer would quietly stop checking at all.

So the key format and the digest live here.
"""
from __future__ import annotations

import hashlib
from typing import Iterable


def case_key(test: str, flags: str) -> str:
    """The identity of one executed configuration: a source and one flag set."""
    return f"{test}[{flags}]"


def case_set_digest(keys: Iterable[str]) -> str:
    """A stable hash over the *multiset-free* set of case keys a run owes.

    Sorted, so insertion order, thread scheduling and `--jobs` cannot change
    it; joined with NUL, so no pair of distinct key lists can collide by
    concatenation.
    """
    return hashlib.sha256("\0".join(sorted(keys)).encode()).hexdigest()
