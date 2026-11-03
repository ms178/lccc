"""Unconditional DejaGnu option directives in GCC's C torture corpus.

DejaGnu spells ``dg-options`` in two shapes, and both occur in
``gcc.c-torture``::

    /* { dg-options "-O2 -fwrapv" } */
    /* { dg-options { "-fwrapv" } } */

The second is a Tcl *list*.  A parser that only accepts a quoted string
silently drops every flag in it.  That is not a cosmetic omission:
``gcc.c-torture/execute/930529-1.c`` uses the brace form for ``-fwrapv``,
which makes its signed-overflow loop defined.  Without the flag GCC 16.2
folds the loop to ``ud2`` and the reference binary dies with SIGILL; the
differential runner then reports ``reference-fail`` for BOTH x86 targets --
an oracle failure that looks like a compiler defect, on a case LCCC passes.

This module is the single source of truth for that extraction.  It is
deliberately conservative:

* a directive carrying a ``{ target ... }`` clause is skipped in full (target
  expressions are Tcl, and guessing them is worse than using the suite
  default);
* only tokens that begin with ``-`` are returned, so a stray Tcl word can
  never become a compiler argument;
* an unterminated group or an unparseable quoted string yields no tokens for
  that directive instead of a partially interpreted one.

The runner is still a documented native differential subset of DejaGnu, not
an implementation of it: ``dg-do``, nested target expressions, LTO and
diagnostic suites remain out of scope.  This parser only widens the option
*spelling* the subset accepts.
"""
from __future__ import annotations

import re
import shlex
from typing import Sequence

# dg-options / dg-additional-options, followed by whitespace.  The rest of the
# directive is parsed structurally below, so this only has to find the start.
_START = re.compile(r"\{\s*dg-(?:additional-)?options\s+")
# A target qualifier anywhere inside the directive makes it conditional.
_TARGET = re.compile(r"\{\s*target\b")
# Quoted strings, including backslash-escaped quotes (rare, but legal Tcl).
_QUOTED = re.compile(r'"((?:[^"\\]|\\.)*)"')
# Bare -flag tokens outside quotes (e.g. `{ dg-options -Wa,--no-verify }`).
_BARE_FLAG = re.compile(r"(?<![\w.-])(--?[A-Za-z0-9][^\s{}\"']*)")


def _balanced(text: str, start: int) -> tuple[str, int] | None:
    """Return (body, index-after-close) of the brace group at ``start``.

    ``start`` must point at ``{``.  ``None`` means the group is unterminated,
    in which case the caller must not interpret any of its contents.
    """
    depth = 0
    for index in range(start, len(text)):
        char = text[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return text[start + 1:index], index + 1
    return None


def _quoted_tokens(body: str) -> list[str]:
    tokens: list[str] = []
    for match in _QUOTED.finditer(body):
        try:
            tokens.extend(shlex.split(match.group(1)))
        except ValueError:
            # An unbalanced quote inside the Tcl string: do not guess.
            return []
    return tokens


def extract_option_flags(text: str, *, limit: int = 8192) -> tuple[str, ...]:
    """Return the unconditional option flags in the first ``limit`` bytes.

    Both spellings are accepted, in any order, and multiple directives
    accumulate in source order.  A directive whose brace group is
    unterminated contributes nothing (a partial read must never turn into a
    silently different compiler invocation).
    """
    flags: list[str] = []
    for match in _START.finditer(text[:limit]):
        # Find the directive's closing brace from the opening `{` of the match,
        # so a target qualifier that follows the flags is still visible.
        opening = match.start()
        group = _balanced(text, opening)
        if group is None:
            continue
        body, _end = group
        if _TARGET.search(body):
            continue
        # ``body`` starts one character after ``opening`` (the `{`), so the
        # match end measured from ``opening`` must lose that one character.
        payload = body[match.end() - opening - 1:]
        tokens: list[str] = _quoted_tokens(payload)
        if not tokens and not _QUOTED.search(payload):
            # A Tcl list can also spell ONE flag without quotes, e.g.
            # `{ dg-options -Wa,--no-verify }`.  Only accept bare tokens when
            # the payload has no quoted strings at all: a mixed/unknown Tcl
            # list must not leak its stray words into the compiler argv.
            tokens.extend(m.group(0) for m in _BARE_FLAG.finditer(payload))
        flags.extend(token for token in tokens if token.startswith("-"))
    return tuple(flags)


def flags_for_source(path, *, limit: int = 8192) -> tuple[str, ...]:
    """Convenience wrapper for a source file (errors are the caller's)."""
    from pathlib import Path

    return extract_option_flags(Path(path).read_text(errors="replace"), limit=limit)


def with_flags(command: Sequence[str], flags: Sequence[str]) -> list[str]:
    """Insert flags before positional source operands (identity helper).

    Kept here so callers that assemble argv do not each have to re-decide
    where directives belong; the native runners place them with the other
    compile-only flags, ahead of the source path.
    """
    return [*command, *flags]


__all__ = ["extract_option_flags", "flags_for_source", "with_flags"]
