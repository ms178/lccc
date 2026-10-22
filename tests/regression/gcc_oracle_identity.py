"""Classify exact GCC 16.2 evidence versus an explicitly allowed GCC 16 snapshot."""
import re


_EXACT_GCC_16_2 = re.compile(r"16\.2(?:\.0)?")
_GCC_16_SNAPSHOT = re.compile(r"16\.\d+(?:\.\d+)?")


def gcc_oracle_identity(version: str, allow_snapshot: bool = False) -> bool:
    """Return whether ``version`` is normative GCC 16.2 evidence.

    Exact-version gates accept GCC 16.2 by default. A rolling GCC 16 package
    (for example, 16.0.1) is accepted only when its caller explicitly opts in;
    its result remains supplemental and this function returns ``False``. The
    check is deliberately on the numeric ``-dumpfullversion`` value, not a
    banner substring that could accidentally match a different compiler.
    """
    version = version.strip()
    normative = _EXACT_GCC_16_2.fullmatch(version) is not None
    if normative:
        return True
    if allow_snapshot and _GCC_16_SNAPSHOT.fullmatch(version):
        return False
    raise ValueError(f"GCC 16.2 oracle required, found {version!r}")
