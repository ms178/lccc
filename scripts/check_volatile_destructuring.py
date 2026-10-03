#!/usr/bin/env python3
"""Ratchet: a `volatile` binding destructured from an IR access is never dead.

WHAT THIS CATCHES
-----------------
`Instruction::Load`/`Store` carry a `volatile: bool` that means "this access
is an observable side effect" (C11 5.1.2.3): it must not be eliminated,
forwarded, CSE'd, hoisted or sunk.  Passes therefore destructure it and
branch on it.

The failure mode is a *refactor* one, and it is silent.  Rewriting

    if !*volatile && dominates && is_load_hoistable(..) { .. }

into

    if dominates { is_load_hoistable(..) } else { .. }

keeps every identifier in scope: `volatile` is still bound, still compiles,
and the guard it fed is simply gone.  The optimizer then hoists a volatile
MMIO load out of its spin loop, and the program hangs on real hardware.

WHY THE COMPILER DOES NOT FULLY CATCH IT
----------------------------------------
`src/lib.rs` no longer allows `unused_variables`, so rustc now fires on a dead
binding and this ratchet is a second, narrower net under it.  The lint alone is
still not sufficient for this field, for two reasons that are exactly the two
rules below.

  1. A dropped guard whose binding is still *mentioned* in the arm
     (`Instruction::Store { volatile: false, .. }` re-emits the field) is not an
     unused variable, so rustc is silent.
  2. Volatility also travels as a FUNCTION PARAMETER, and no lint can know that
     a parameter named `volatile` is load-bearing.  Renaming it to `_volatile`
     silences the diagnostic while leaving the flag dropped in the body -- which
     is what happened in `expr_assign.rs::store_bitfield_split`, where the rename
     hid a wrong-code bug for volatile bitfields.

THE PARAMETER RULE
------------------
A parameter whose NAME contains `volatile` is a volatility flag the function is
trusted to honour.  If the body never reads it, the flag is dropped, and the
compiler cannot tell "does not matter here" from "was forgotten": `_volatile` is
a legal, warning-free way to write the latter.  So a `_`-prefixed `volatile`
parameter is a violation, and any volatile parameter never read in its own body
is a violation.  Discarding it explicitly in the body (`let _ = volatile;`) is
allowed and is the reviewable way to record that the flag is irrelevant -- the
same escape hatch as `volatile: _`.

THE RULE
--------
For every `Instruction::Load { .. }` / `Instruction::Store { .. }`
destructuring pattern in `src/`, if the pattern binds `volatile` by value
(`volatile` or `volatile: <binding>`), the enclosing arm body must contain a
*use* of it.  Writing `volatile: _` is the explicit, reviewable way to say
"I looked at volatility and it does not matter here" and is always allowed.

Fail-closed: an unparseable file, or a body the scanner cannot delimit, is a
violation rather than a skip.  A rule that cannot read the code must not
report the code clean.

Self-test
---------
`--self-test` runs the detector over synthetic snippets -- including the
verbatim pre-fix shape of the LICM bug -- and fails if the detector does not
fire on them.  A detector that cannot be shown to catch the known bug is not a
detector; see tests/regression/check_env_test_hygiene.sh for the same
philosophy applied to the environment-read ratchet.

Usage:
    python3 scripts/check_volatile_destructuring.py [--root DIR] [--self-test]
Exit status 0 = clean, 1 = violation, 2 = self-test failure.
"""
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

# `Instruction::Load {` / `Instruction::Store {` -- the two access kinds whose
# `volatile` flag is an observable-access contract.  `AtomicLoad`/`AtomicRmw`
# are excluded on purpose: `_Atomic` accesses are already unremovable, so no
# pass needs to branch on their volatility.
_ACCESS_RE = re.compile(r"Instruction::(?P<kind>Load|Store)\s*\{")
# A USE of the binding, as opposed to a mention of the same word.
#
# `volatile_x` is a different identifier and `volatile:` is a FIELD NAME -- in
# a struct literal it names the field being built, and in a pattern it is the
# binding site itself.  Counting those as uses is the difference between a gate
# and a rubber stamp: the exact failure this rule must catch is
#
#     Instruction::Store { val, ptr, ty, seg_override, volatile } =>
#         Instruction::Store { volatile: false, .. },     // flag hardcoded off
#
# where the word `volatile` is still all over the construct, so a naive search
# finds a "use" and the dropped flag ships.  Only a bare mention counts, which
# is what `*volatile`, `!volatile`, `volatile &&` and `(&volatile)` all are.
_USE_RE = re.compile(r"\bvolatile\b(?!\s*:)")
# Tokens that put the following `Instruction::Load { .. }` in PATTERN position.
# `if let X { .. } = e` and `match e { X { .. } => .. }` both introduce a
# pattern; the `=` that follows the pattern is outside the window we look at,
# so it cannot be confused for a construction.
_PATTERN_INTRO_RE = re.compile(
    r"\bif\s+let\b|\bwhile\s+let\b|\bmatch\b|\blet\b|=>|\|")
# Tokens that put it in EXPRESSION position, i.e. a struct literal building a
# new instruction.  `Some(Instruction::Load { .. })`, `return Instruction::Load
# { .. }`, `= Instruction::Load { .. }`.
_CONSTRUCT_INTRO_RE = re.compile(r"=|\(|\[|,|\breturn\b|->")
# Pathological-input guard only.  The *semantic* bound is USE_DISTANCE_LINES
# below; the enclosing block merely has to be delimited so the scan has a stop.
# This cap exists to keep a malformed file from turning the scan quadratic, and
# is set far above any real site: the largest enclosing block in this tree is
# the ~350-line instruction-remapping `match` in inline.rs/outline_switch.rs,
# whose arms are constructions that legitimately re-emit `volatile: *volatile`.
MAX_SCAN_LINES = 5000
# How far below its own pattern a use of `volatile` still counts as that
# binding's use.  See `_use_window` for why the rule is a distance and not an
# attempt to re-derive the arm boundary.
USE_DISTANCE_LINES = 40

# Values that can appear on the right of `volatile:` in a struct LITERAL and
# are therefore never pattern bindings.  Anything in this set means we are
# looking at a construction, not a destructuring.
_LITERAL_RHS = {
    "true", "false", "self", "Self", "None", "Some", "Default",
}


def _is_char_literal(text: str, i: int) -> bool:
    """True when the `'` at ``i`` opens a char literal, not a lifetime.

    `'a'` and `'\n'` are char literals; `'a`, `&'static str` and `T: 'a` are
    lifetimes.  The two are distinguishable only by what follows, so that is
    the test: an escape, or exactly one character and a closing quote.

    Getting this wrong is not a cosmetic bug.  A lifetime misread as an
    unterminated literal makes the scanner run to the *next* apostrophe in the
    file and blank everything between, which silently removes braces and makes
    every enclosing block undelimitable -- the ratchet then reports the whole
    file rather than quietly passing it.  (It did exactly that on
    `src/backend/x86/codegen/isel.rs`, whose `'{'`/`'}'` byte-class literals
    unbalanced the stripped text by three.)
    """
    n = len(text)
    if i + 1 >= n or text[i + 1] == "'":
        return False                       # `''` is not Rust; treat as lifetime
    if text[i + 1] == "\\":
        return i + 2 < n and text[i + 2] == "'"
    return i + 2 < n and text[i + 2] == "'"


def _prev_token(src: str, idx: int) -> str:
    """The token immediately preceding ``idx``, ignoring whitespace.

    One word or one punctuation mark.  This is the discriminator that
    separates `push(Instruction::Load { .. })` (a literal -- the constructor
    call ends in `(`) from `match x { Instruction::Load { .. } => .. }` (a
    pattern -- the arm opens with `{`), which word-frequency heuristics cannot
    do because both sites sit inside functions full of `let`/`match`.
    """
    k = idx - 1
    while k >= 0 and src[k].isspace():
        k -= 1
    if k < 0:
        return ""
    if src[k].isalnum() or src[k] == "_":
        j = k
        while j >= 0 and (src[j].isalnum() or src[j] == "_"):
            j -= 1
        return src[j + 1:k + 1]
    return src[k]


def _is_pattern_position(src: str, idx: int, close_idx: int) -> bool:
    """Is the `Instruction::.. { .. }` at ``idx`` a PATTERN or a struct literal?

    The two spellings are identical apart from their neighbours, and this tree
    contains both, often in the same function.  `inline.rs` and
    `outline_switch.rs` remap every instruction with

        Instruction::Load { dest, ptr, ty, seg_override, volatile }
            => Instruction::Load { volatile: *volatile, dest: remap(..), .. },

    where the first is a pattern and the second a literal that merely re-emits
    the field name; `ir/lowering/*.rs` and `ir/mem2reg/promote.rs` build loads
    with a shorthand `Instruction::Load { volatile, dest, .. }`, which is
    textually identical to a pattern that binds `volatile`.  Getting this
    backwards is fatal in both directions: a literal reported as a dead binding
    cries wolf, and a pattern skipped as a literal misses the bug.

    The discriminator is the immediately adjacent token, which is exact for
    every shape here, and the direction of the arm's `=>`, which is exact for
    the rest:

        push(Instruction::Load {..})   literal -- constructor call, token `(`
        = Instruction::Store {..}     literal -- assigned, token `=`
        => Instruction::Store {..}    literal -- the arm's expression
        match x { Instruction::Load {..} => .. }   pattern -- token `{`
        | Instruction::Load {..} | ..  pattern -- token `|`
        let Instruction::Store {..} = e else {..}   pattern -- token `let`
    """
    # Decisive first: in `PAT => EXPR` the pattern is before the `=>` and the
    # expression after it.  `|` is the or-pattern separator, so a construct
    # followed by one is a pattern too.
    after = src[close_idx + 1:].lstrip()
    if after.startswith("=>") or after.startswith("|"):
        return True
    tok = _prev_token(src, idx)
    if tok in ("let", "match", "|"):
        return True
    if tok in ("(", "[", ",", "=", "->", "?", "!", ";", "{", "}", "return"):
        # `{` and `}` are literals here, not patterns.  A `{` is what follows
        # `match x`, but it is equally what opens a function body -- and a
        # construct in statement position is an expression.  The match-arm
        # case never reaches this line, because its `=>` was already caught by
        # the `after` test above; the same is true of an or-pattern.
        return False
    # Ambiguous introducer (a bare identifier, e.g. a helper taking the
    # instruction as its argument).  Fall back to the shape of the field list:
    # a pattern binds bare identifiers and `..`; a literal evaluates
    # expressions, and every field of Load/Store is a value or a type.
    field_list = src[src.index("{", idx) + 1:close_idx]
    if "*" in field_list or "(" in field_list:
        return False
    return ".." in field_list or _binds_volatile(field_list)


def _strip_comments(text: str) -> str:
    """Blank out comments and string/char literals, preserving offsets.

    Offsets matter: the caller reports `file:line` from the returned text, so
    every character has to survive.  Replaced spans become spaces.
    """
    out = list(text)
    i, n = 0, len(text)
    while i < n:
        two = text[i:i + 2]
        if two == "//":
            j = text.find("\n", i)
            j = n if j < 0 else j
            for k in range(i, j):
                out[k] = " "
            i = j
        elif two == "/*":
            depth, j = 1, i + 2
            while j < n and depth:
                if text[j:j + 2] == "/*":
                    depth += 1
                    j += 2
                elif text[j:j + 2] == "*/":
                    depth -= 1
                    j += 2
                else:
                    j += 1
            for k in range(i, min(j, n)):
                if out[k] != "\n":
                    out[k] = " "
            i = j
        elif text[i] == '"':
            j = i + 1
            while j < n:
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == '"':
                    j += 1
                    break
                j += 1
            for k in range(i, min(j, n)):
                if out[k] != "\n":
                    out[k] = " "
            i = j
        elif text[i] == "'" and _is_char_literal(text, i):
            # Rust lifetimes ('a) look exactly like an unterminated char
            # literal.  Treating them as literals makes the scanner run to the
            # NEXT apostrophe in the file and blank everything in between --
            # which silently unbalances the braces and makes every enclosing
            # block undelimitable.  A real char literal is at most four
            # characters: quote, (escape | any char), quote.
            j = i + 1
            if text[j] == "\\":
                j += 3
            else:
                j += 2
            for k in range(i, min(j, n)):
                if out[k] != "\n":
                    out[k] = " "
            i = j
        else:
            i += 1
    return "".join(out)


def _match_brace(text: str, open_idx: int) -> int:
    """Index of the `}` closing the `{` at ``open_idx``, or -1."""
    depth = 0
    for k in range(open_idx, len(text)):
        c = text[k]
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if depth == 0:
                return k
    return -1


def _binds_volatile(field_list: str) -> bool:
    """True if this pattern field-list binds `volatile` by value.

    `volatile: _` and `volatile: ref _` are explicit discards and are fine.
    A bare `volatile`, or `volatile: pat`, is a binding -- unless the
    right-hand side is a literal (`volatile: false` in a struct literal), which
    is a construction rather than a binding and is handled by the caller.
    """
    depth = 0
    for part in field_list.split(","):
        s = part.strip()
        if not s or depth:
            # Inside a nested generic/braced sub-pattern; conservatively do
            # not claim a binding we cannot see.
            depth += s.count("{") - s.count("}")
            continue
        if s == "volatile":
            return True
        if s.startswith("volatile:"):
            rhs = s[len("volatile:"):].strip()
            if not rhs or rhs in _LITERAL_RHS:
                continue
            if rhs in ("_", "ref _", "mut _", "box _"):
                continue
            # A bare literal (number, char, string) is an expression.
            if rhs[0] in "\"'0123456789-.":
                continue
            return True
    return False


def _enclosing_block(src: str, idx: int) -> tuple[int, int] | None:
    """The ``{ ... }`` block that lexically encloses offset ``idx``.

    Returns ``(open_idx, close_idx)``, or ``None`` when no enclosing block can
    be found or it is unterminated.

    Walking backwards with a depth counter and then matching the block forward
    is the only delimiting that works for every shape Rust uses here:

      * ``if let PAT {..} = e { BODY }``      -> BODY (and the else arm);
      * ``let PAT {..} = e else { .. };``     -> the statements that FOLLOW the
        `;`, because the binding's scope is the enclosing block, not the
        statement;
      * ``match e { PAT {..} => EXPR, .. }``  -> EXPR and its sibling arms.

    A `let ... else` whose use lives in the next statement is exactly the shape
    a naive "scan to the next `{`" gets wrong, and it is the dominant shape in
    this tree's passes (slp_vectorizer, bit_idioms and loop_carried_forward all
    use it) -- so the window is anchored on the enclosing block and then
    narrowed again in ``_use_window``.
    """
    depth = 0
    open_idx = -1
    for k in range(idx - 1, -1, -1):
        c = src[k]
        if c == "}":
            depth += 1
        elif c == "{":
            if depth == 0:
                open_idx = k
                break
            depth -= 1
    if open_idx < 0:
        return None
    close_idx = _match_brace(src, open_idx)
    if close_idx < 0:
        return None
    return open_idx, close_idx


def _use_window(src: str, pattern_close: int, block_close: int) -> int:
    """How far after the pattern a use of `volatile` still counts as its use.

    Every shape this tree uses -- `if let PAT {..} = e { .. }`, the `let PAT {..}
    = e else { .. };` used by slp_vectorizer/bit_idioms/loop_carried_forward,
    the `| PAT {..} | PAT2 {..} => (a, b, ..)` or-patterns in if_convert, and
    the instruction-remapping `match` arms in inline/outline_switch -- places
    the guard's use a handful of lines after the pattern closes, and inside
    different delimiters.  Trying to find the exact arm boundary means
    re-implementing enough of Rust's grammar to get `let ... else` and
    or-patterns right, and a gate that cries wolf on real code gets deleted.

    So the rule is deliberately blunt and stated as a bound instead: a use
    counts when it is within ``USE_DISTANCE_LINES`` lines after the pattern
    closes, anywhere in the enclosing block.  That is long enough for every
    real guard (the widest in this tree is a handful of lines) and short enough
    that an unrelated `volatile` elsewhere in the function cannot mask a dead
    binding.  The cost is a documented blind spot: a guard whose use sits more
    than that many lines below its own pattern would be missed.  Writing the
    check inline, as every site in this tree does, is what keeps the distance
    small -- and the behavioural gate in
    `tests/regression/check_volatile_pointer_subscript.sh` is the backstop for
    the case where this one is blind.
    """
    stop = src.find("\n", pattern_close)
    if stop < 0 or stop > block_close:
        stop = block_close
    for _ in range(USE_DISTANCE_LINES):
        if stop >= block_close:
            break
        stop = src.find("\n", stop + 1)
        if stop < 0:
            return block_close
    return min(stop, block_close)



# ---------------------------------------------------------------------------
# Rule 2: a volatility flag passed in as a parameter
# ---------------------------------------------------------------------------
# Deliberately not anchored to `pub`/`pub(super)`: visibility is irrelevant to
# whether a body honours its own parameter.
_FN_RE = re.compile(r"\bfn\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*(?:<[^<>]*>)?\s*\(")


def _match_paren(text: str, open_idx: int) -> int:
    """Index of the `)` closing the `(` at ``open_idx``, or -1."""
    depth = 0
    for k in range(open_idx, len(text)):
        c = text[k]
        if c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
            if depth == 0:
                return k
    return -1


def _split_top_level(text: str, sep: str = ",") -> list[str]:
    """Split on ``sep`` at bracket depth 0, so `FxHashMap<u8, u32>` survives."""
    parts, cur, depth = [], "", 0
    for ch in text:
        if ch in "(<[":
            depth += 1
        elif ch in ")>]":
            depth -= 1
        if ch == sep and depth == 0:
            parts.append(cur)
            cur = ""
        else:
            cur += ch
    if cur.strip():
        parts.append(cur)
    return parts


def _volatile_params(param_list: str) -> list[str]:
    """Parameter names claiming to carry volatility, in order.

    Matches `volatile: bool`, `_volatile: bool`, `mut volatile: bool`.  A
    parameter whose TYPE mentions volatile (`p: *volatile u8`) is not a flag and
    is not matched: the name is what the rule is about.
    """
    names = []
    for raw in _split_top_level(param_list):
        m = re.match(r"\s*(?:mut\s+|ref\s+)?(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*:",
                     raw)
        if not m:
            continue
        name = m.group("name")
        if "volatile" in name:
            names.append(name)
    return names


def check_volatile_params(text: str, origin: str) -> list[str]:
    """One violation per volatility parameter its own body never reads."""
    src = _strip_comments(text)
    violations: list[str] = []
    for m in _FN_RE.finditer(src):
        open_idx = src.index("(", m.end() - 1)
        close_idx = _match_paren(src, open_idx)
        line = src.count("\n", 0, m.start()) + 1
        if close_idx < 0:
            violations.append(
                f"{origin}:{line}: could not delimit the parameter list of "
                f"`{m.group('name')}` (unbalanced parens)")
            continue
        params = _volatile_params(src[open_idx + 1:close_idx])
        if not params:
            continue
        # A trait method declaration has no body: no site to honour the flag
        # there, and each implementation is checked as a function in its own
        # right.
        brace = src.find("{", close_idx)
        if brace < 0 or ";" in src[close_idx + 1:brace]:
            continue
        body_close = _match_brace(src, brace)
        if body_close < 0:
            violations.append(
                f"{origin}:{line}: could not delimit the body of "
                f"`{m.group('name')}`; refusing to claim its volatility "
                f"parameter is honoured")
            continue
        body = src[brace:body_close + 1]
        for name in params:
            stripped = name.lstrip("_")
            underscored = name.startswith("_") and stripped != ""
            # A bare mention counts; `volatile: false` inside the body does NOT,
            # for the same reason `_USE_RE` has the lookahead: there the word is
            # the name of a field being written, which is exactly the shape of
            # the bug (a dropped flag re-emitted as a literal).
            used = re.search(
                r"\b" + re.escape(stripped) + r"\b(?!\s*:)", body) is not None
            if underscored:
                why = (f"is named `_{stripped}`; the underscore asserts the flag "
                       f"is unused")
            elif not used:
                why = "is never read in its own body"
            else:
                continue
            violations.append(
                f"{origin}:{line}: volatility parameter `{name}` of "
                f"`{m.group('name')}` {why} -- a flag that reaches a function and "
                f"is dropped there is a wrong-code bug, not a missed optimisation "
                f"(if it is genuinely irrelevant, discard it explicitly: "
                f"`let _ = {stripped};`)")
    return violations


def check_source(text: str, origin: str) -> list[str]:
    """Return one violation string per dead `volatile` binding in ``text``."""
    src = _strip_comments(text)
    violations: list[str] = []
    for m in _ACCESS_RE.finditer(src):
        open_idx = src.index("{", m.start())
        close_idx = _match_brace(src, open_idx)
        if close_idx < 0:
            line = src.count("\n", 0, m.start()) + 1
            violations.append(
                f"{origin}:{line}: could not delimit the "
                f"Instruction::{m.group('kind')} pattern (unbalanced braces)")
            continue
        if not _binds_volatile(src[open_idx + 1:close_idx]):
            continue
        if not _is_pattern_position(src, m.start(), close_idx):
            # A struct literal that happens to write `volatile: <expr>` is a
            # construction, not a guard that could have been dropped.
            continue
        line = src.count("\n", 0, m.start()) + 1
        kind = m.group("kind")
        block = _enclosing_block(src, open_idx)
        if block is None:
            violations.append(
                f"{origin}:{line}: binds `volatile` from Instruction::{kind} "
                f"but its enclosing block cannot be delimited; refusing to "
                f"claim this site is clean")
            continue
        _, block_close = block
        end = _use_window(src, close_idx, block_close)
        span = src.count("\n", 0, block_close) - src.count("\n", 0, close_idx)
        if span > MAX_SCAN_LINES:
            violations.append(
                f"{origin}:{line}: binds `volatile` from Instruction::{kind} "
                f"inside a {span}-line block, past the {MAX_SCAN_LINES}-line "
                f"scan cap; refusing to claim this site is clean")
            continue
        if not _USE_RE.search(src, close_idx + 1, end):
            violations.append(
                f"{origin}:{line}: binds `volatile` from Instruction::{kind} "
                f"and never uses it -- an observable-access guard was dropped "
                f"(write `volatile: _` if volatility genuinely does not "
                f"matter here)")
    violations += check_volatile_params(text, origin)
    return violations


# ---------------------------------------------------------------------------
# Self-test
# ---------------------------------------------------------------------------
_SELF_TEST_CASES = [
    # (label, source, expect_violation)
    ("the LICM regression, verbatim shape",
     """
     fn f(inst: &Instruction, dominates: bool) -> bool {
         if let Instruction::Load { ptr, ty, volatile, .. } = inst {
             if dominates { helper(ptr, ty) } else { false }
         } else { false }
     }
     """, True),
    ("explicit discard is fine",
     """
     fn f(inst: &Instruction) -> bool {
         if let Instruction::Load { ptr, volatile: _, .. } = inst { helper(ptr) } else { false }
     }
     """, False),
    ("binding with a real use is fine",
     """
     fn f(inst: &Instruction) -> bool {
         if let Instruction::Load { ptr, volatile, .. } = inst {
             if *volatile { false } else { helper(ptr) }
         } else { false }
     }
     """, False),
    ("match arm, not if-let",
     """
     fn f(inst: &Instruction) -> bool {
         match inst {
             Instruction::Store { val, ptr, volatile, .. } => {
                 if *volatile { emit(ptr, val) } else { helper(ptr) }
             }
             _ => false,
         }
     }
     """, False),
    ("a dropped store guard is caught too",
     """
     fn f(inst: &Instruction) -> bool {
         if let Instruction::Store { val, ptr, volatile, .. } = inst {
             sink(ptr, val)
         } else { false }
     }
     """, True),
    ("`volatile: false` construction is not a binding",
     """
     fn make() -> Instruction {
         Instruction::Load { dest, ptr, ty, seg, volatile: false }
     }
     """, False),
    ("a mention inside a comment does not count as a use",
     """
     fn f(inst: &Instruction) -> bool {
         if let Instruction::Load { ptr, volatile, .. } = inst {
             // we used to check *volatile here
             helper(ptr)
         } else { false }
     }
     """, True),
    ("a mention inside a string does not count as a use",
     """
     fn f(inst: &Instruction) -> bool {
         if let Instruction::Load { ptr, volatile, .. } = inst {
             note("volatile");
             helper(ptr)
         } else { false }
     }
     """, True),
    ("`volatile_x` is a different identifier",
     """
     fn f(inst: &Instruction) -> bool {
         let volatile_x = 1;
         if let Instruction::Load { ptr, volatile, .. } = inst {
             helper(ptr, volatile_x)
         } else { false }
     }
     """, True),
    # The shapes that produced false positives while this gate was being
    # written.  Each one is a real construct from src/passes, and each was
    # first reported as a dead binding: they all bind `volatile` from an IR
    # access and then use it, but not in any delimiter a naive arm-boundary
    # scan would find.  A ratchet that cries wolf on the tree it guards gets
    # deleted, so these are the cases worth pinning.
    ("`let .. else` whose use is in the NEXT statement",
     """
     fn f(inst: &Instruction) -> bool {
         let Instruction::Store { val, ptr, ty, seg_override, volatile } = inst
         else { continue };
         if *seg_override != AddressSpace::Default || *volatile { continue; }
         sink(ptr, val)
     }
     """, False),
    ("or-pattern binding into a tuple",
     """
     fn f(inst: &Instruction) -> bool {
         let (ptr, volatile, seg, ty) = match inst {
             Instruction::Load { ptr, volatile, seg_override, ty, .. }
             | Instruction::Store { ptr, volatile, seg_override, ty, .. }
                 => (ptr, volatile, seg_override, ty),
             _ => continue,
         };
         if !*volatile && *seg == AddressSpace::Default { helper(ptr, ty) }
         else { false }
     }
     """, False),
    ("remapping `match`: the PATTERN is not a dead binding",
     """
     fn f(inst: &Instruction) -> bool {
         match inst {
             Instruction::Load { dest, ptr, ty, seg_override, volatile } =>
                 Instruction::Load {
                     volatile: *volatile,
                     dest: remap(*dest),
                     ptr: remap(*ptr),
                     ty: *ty,
                     seg_override: *seg_override,
                 },
             _ => inst,
         }
     }
     """, False),
    ("remapping `match`: the CONSTRUCTION is not a binding at all",
     """
     fn f(inst: &Instruction) -> bool {
         match inst {
             Instruction::Store { val, ptr, ty, seg_override, volatile } =>
                 Instruction::Store {
                     volatile: *volatile,
                     val: remap(*val),
                     ptr: remap(*ptr),
                     ty: *ty,
                     seg_override: *seg_override,
                 },
             _ => inst,
         }
     }
     """, False),
    ("remapping `match` with the guard actually dropped",
     """
     fn f(inst: &Instruction) -> bool {
         match inst {
             Instruction::Store { val, ptr, ty, seg_override, volatile } =>
                 Instruction::Store {
                     volatile: false,
                     val: remap(*val),
                     ptr: remap(*ptr),
                     ty: *ty,
                     seg_override: *seg_override,
                 },
             _ => inst,
         }
     }
     """, True),
    # Rule 2: volatility passed as a parameter.  The first case is the OLD
    # `store_bitfield_split` signature verbatim -- the commit that hid the
    # volatile-bitfield bug by renaming the parameter instead of using it.
    ("the store_bitfield_split regression, verbatim shape",
     """
     fn store_bitfield_split(&mut self, addr: Value, storage_ty: IrType,
                             bit_offset: u32, bit_width: u32, val: Operand,
                             _volatile: bool, sso: SsoMode) {
         let x = self.load(addr, storage_ty);
     }
     """, True),
    ("a volatile parameter the body reads is fine",
     """
     fn store_bitfield_split(&mut self, addr: Value, volatile: bool, sso: SsoMode) {
         self.emit(Store { volatile, ptr: addr });
     }
     """, False),
    ("a volatile parameter never read is flagged",
     """
     fn lower(&mut self, volatile: bool, addr: Value) {
         self.emit(Load { volatile: false, ptr: addr });
     }
     """, True),
    ("explicit discard of a parameter is fine",
     """
     fn lower(&mut self, volatile: bool, addr: Value) {
         let _ = volatile;
         self.emit(Load { ptr: addr });
     }
     """, False),
    ("a pointer type that mentions volatile is not a flag",
     """
     fn describe(ptr: *volatile u8, len: usize) -> usize { len }
     """, False),
    ("a trait method declaration has no body and is not judged",
     """
     trait T { fn lower(&self, volatile: bool, addr: Value) -> u8; }
     """, False),
    ("unterminated body fails closed",
     """
     fn f(inst: &Instruction) -> bool {
         if let Instruction::Load { ptr, volatile, .. } = inst { helper(ptr)
     }
     """, True),
]


def self_test() -> int:
    failures = 0
    print("== volatile-destructuring ratchet self-test ==")
    for label, src, expect in _SELF_TEST_CASES:
        got = check_source(src, "<self-test>")
        ok = bool(got) == expect
        failures += 0 if ok else 1
        print("  %-4s %-46s expect=%-9s got=%s"
              % ("ok" if ok else "FAIL", label,
                 "violation" if expect else "clean",
                 "violation" if got else "clean"))
    print("  (%d cases)" % len(_SELF_TEST_CASES))
    if failures:
        print("SELF-TEST FAILED: %d/%d" % (failures, len(_SELF_TEST_CASES)))
    else:
        print("self-test: PASS")
    return failures


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    ap.add_argument("--self-test", action="store_true",
                    help="verify the detector against known-bad and known-good "
                         "shapes, then exit")
    args = ap.parse_args()
    if args.self_test:
        return 1 if self_test() else 0

    src_root = args.root / "src"
    if not src_root.is_dir():
        print(f"volatile-destructuring ratchet: no src/ under {args.root}",
              file=sys.stderr)
        return 1

    violations: list[str] = []
    files = sorted(src_root.rglob("*.rs"))
    if not files:
        print("volatile-destructuring ratchet: no Rust sources found; "
              "refusing to report the tree clean", file=sys.stderr)
        return 1
    for path in files:
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError) as exc:
            violations.append(f"{path}: unreadable: {exc}")
            continue
        try:
            violations += check_source(text, str(path.relative_to(args.root)))
        except Exception as exc:                      # noqa: BLE001
            # Fail closed.  A detector that raises on unexpected input must not
            # be reported as "clean" by its caller.
            violations.append(f"{path}: detector raised: {exc!r}")

    if violations:
        print("volatile-destructuring ratchet: FAIL", file=sys.stderr)
        for v in violations:
            print("  " + v, file=sys.stderr)
        return 1
    print(f"volatile-destructuring ratchet: ok ({len(files)} files)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
