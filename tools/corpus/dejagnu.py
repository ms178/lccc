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

# dg-options / dg-additional-options, followed by whitespace.  The rest of the
# directive is parsed structurally below, so this only has to find the start.
_START = re.compile(r"\{\s*dg-(?:additional-)?options\s+")
# A target qualifier anywhere inside the directive makes it conditional.
_TARGET = re.compile(r"\{\s*target\b")
# Quoted strings, including backslash-escaped quotes (rare, but legal Tcl).
_QUOTED = re.compile(r'"((?:[^"\\]|\\.)*)"')
# Bare -flag tokens outside quotes (e.g. `{ dg-options -Wa,--no-verify }`).
_BARE_FLAG = re.compile(r"(?<![\w.-])(--?[A-Za-z0-9][^\s{}\"']*)")
# `--param NAME=N` is spelled as TWO argv entries.  A Tcl brace-list is a list
# of flags, not an arbitrary argv, so a non-flag token is kept ONLY as the
# value of that one option; everything else (`;`, paths, bare words) is dropped.
_PARAM_VALUE = re.compile(r"[A-Za-z_][A-Za-z0-9_.-]*=[^\s;{}'\"]+")


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


def _quoted_tokens(body: str) -> tuple[list[str], bool]:
    """Return (tokens, unsupported).

    ``unsupported`` is set for backslash escapes or unbalanced quotes: a
    Tcl string we cannot decode exactly must not become a silently different
    compiler invocation.  The caller skips the whole directive in that case.
    """
    tokens: list[str] = []
    for match in _QUOTED.finditer(body):
        raw = match.group(1)
        if "\\" in raw:
            return [], True
        try:
            tokens.extend(shlex.split(raw))
        except ValueError:
            return [], True
    return tokens, False


def _brace_list_tokens(tokens: list[str]) -> list[str]:
    """A brace list is a list of FLAGS: keep flags and --param values only."""
    kept: list[str] = []
    previous = ""
    for token in tokens:
        if token.startswith("-"):
            kept.append(token)
        elif previous == "--param" and _PARAM_VALUE.fullmatch(token):
            kept.append(token)
        previous = token
    return kept


def extract_option_flags(text: str, *, limit: int = 8192) -> tuple[str, ...]:
    """Return the unconditional option flags in the first ``limit`` bytes.

    Both spellings are accepted, in any order, and multiple directives
    accumulate in source order.  A directive whose brace group is
    unterminated contributes nothing (a partial read must never turn into a
    silently different compiler invocation).
    """
    if limit < 0:
        raise ValueError("limit must be non-negative")
    # ONE bounded window for both the search and the structural parse.  Passing
    # the full text to _balanced would let an unterminated directive near the
    # boundary find a stray `}` in the C source *after* the window and turn a
    # truncated input into a plausible-looking option list.
    window = text[:limit]
    flags: list[str] = []
    for match in _START.finditer(window):
        # Find the directive's closing brace from the opening `{` of the match,
        # so a target qualifier that follows the flags is still visible.
        opening = match.start()
        group = _balanced(window, opening)
        if group is None:
            continue
        body, _end = group
        if _TARGET.search(body):
            continue
        # ``body`` starts one character after ``opening`` (the `{`), so the
        # match end measured from ``opening`` must lose that one character.
        # The offset can never exceed ``len(body)``: the match itself is part
        # of the balanced group that produced ``body``.
        payload = body[match.end() - opening - 1:]
        tokens, unsupported = _quoted_tokens(payload)
        if unsupported:
            continue
        if _QUOTED.search(payload):
            # Quoted spelling: retain the exact argv the old runner passed
            # (including separated `--param NAME=N` values).  A brace list is
            # a list of *flags*, so filter it; the single quoted string is a
            # literal option string and is preserved verbatim.
            is_brace_list = not payload.lstrip().startswith('"')
            flags.extend(_brace_list_tokens(tokens) if is_brace_list else tokens)
        else:
            # A Tcl list can also spell ONE flag without quotes, e.g.
            # `{ dg-options -Wa,--no-verify }`.  Only accept bare tokens when
            # the payload has no quoted strings at all: a mixed/unknown Tcl
            # list must not leak its stray words into the compiler argv.
            flags.extend(m.group(0) for m in _BARE_FLAG.finditer(payload))
    return tuple(flags)


__all__ = ["extract_option_flags"]
