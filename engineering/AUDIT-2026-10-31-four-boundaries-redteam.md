# Red-team audit of the four-boundary batch, 2026-10-31

This note is the audit trail for the preprocessor / DWARF / static-initializer /
pointer-IV batch. It records what was fixed, what was **disproved**, and what is
deliberately deferred with the reason and the evidence, so a later change cannot
silently re-open a closed question.

## Resolved in the batch

### 1. The joiner no longer runs for files that cannot splice

`join_continued_lines` decided "no continuations possible" from
`!source.contains('\\')`. A single string escape (`"a\tb"`) therefore sent a
whole translation unit through the joining loop, and — after the phase-2 stream
was added — through a second copy of the source. The predicate is now the
joiner's own per-line test, `has_continuation`, asked once:

```rust
if !source.contains('\\')
    || (!Self::has_continuation(source) && !source.contains('\r') && source.ends_with('\n'))
{
    return (std::borrow::Cow::Borrowed(source), None);
}
```

The extra two clauses are not decoration. When no line splices, the loop appends
each line and one `'\n'`, so its output equals the input **only if** the source
ends in a newline and contains no `'\r'` (which `str::lines` strips). Without
those clauses the fast path would change observable output — LF-normalised line
endings and a manufactured final newline — so they are part of the condition,
and both are pinned by `the_joiner_still_normalises_crlf_and_a_missing_final_newline`.

Measured over the tree (4,143 `.c`/`.h`/`.s`/`.S` files, `find` + predicate model):
36.3 % contain a backslash, but only 9.6 % actually splice. The fast path
therefore removes a source-sized allocation, a source-sized copy and a full byte
scan from **26.7 % of all translation units** (73.5 % of the backslash-bearing
ones), and it makes the phase-2 stream — whose cost was the main performance
objection to the splice fix — reachable only where it is needed.

### 2. The doc-link gate failure is fixed at its source

`scripts/check_doc_links.py` failed on two backticked `artifacts/*.json`
citations in `engineering/session-logs/2026-10-31-upstream-1038-audit.md`; the
files were session-workspace evidence that was never part of the tree. Both are
reworded to say so, and the two adjacent, unflagged `.config`/`.log` citations in
the same paragraph were corrected for the same reason: a path that does not
resolve is a false claim whether or not a gate happens to check it.

### 3. The lane plan is re-derived, not patched, after a rebase

The plan is generated (`scripts/ci_lane_split.py`) and its invariant is
`predicted_gate_seconds == Σ measured_s` per lane. After rebasing onto a `main`
that had re-split the codegen lane, the totals were recomputed from the measured
entries rather than merged textually; `tests/regression/check_ci_lane_plan.py`
then proves the shard sets, the preamble identity and the balance.

## Disproved — findings that did not survive their own audit

### A. "A stale `physical.directive` causes a miscompile"

Self-audit result: **false**. The flag is an *additional* guard. Phase-4
directive authority is `first_token_is_directive(line, carry_block)` (fed into
`splittable`/`join_dir`, and inherited across a bridged comment through
`carry_dir`), which already answers the question with comments skipped. A stale
`physical.directive` can only *refuse* a break elision — the conservative
direction, which changes no token and only keeps `__LINE__` granularity coarser
for that one joined line. Worse, the scenarios where it stays set are exactly
the ones where `carry_dir` requires the join anyway (`#define A /* c` …
`*/ body`), so "clearing" it buys nothing and risks re-opening the truncation
that motivated the guard. Deferred as a non-defect; a tightening would need the
abridged kernel macro corpus to prove the guard is still sufficient.

### B. "The now-strict `.loc` parser may reject in-tree assembly"

Self-audit result: **not reproducible in this tree**. Exactly one file under
version control contains `.loc` at all
(`engineering/evidence/2026-10-14-popcount-repair/oracle/clang-arm.s`), and its
eight directives use only `prologue_end` and `is_stmt 0|1` — both accepted. The
strictness risk is real for third-party assembly but has no in-repo evidence, so
no schema-sweep gate was added for it; a gate over one file would be decoration.

### C. "The DWARF oracle expectations may be wrong, so the gate cannot pass"

Self-audit result: **false, and now proven**. The three expectation sets in
`tests/regression/check_dwarf_loc_options.py` were replayed against GNU as and
readelf **2.40** — a different binutils than CI's pinned 2.47:

| Fixture | Result |
|---|---|
| `.loc` flag rows (4 rows, section switch) | 4/4 match `EXPECTED` |
| special-opcode boundary rows (8 rows) | 8/8 match, including the `advance 0` pair |
| seeded 1,000-row line/address program | 1,000/1,000 match |

So the oracle encodes GNU as behaviour rather than a guess, and the textual
`readelf --debug-dump=rawline` decoding is stable across at least two binutils
generations.

### D. "The div-zero matrix is the failing gate"

Self-audit result: **false**. The failing CI step was
`python3 scripts/check_doc_links.py`, not the compiler differential; the harness
itself is self-consistent, verified here with the host compiler as both sides:
2,420 comparisons plus the 27 named controls pass in 3.3 s, and the batch parser
has its own fail-closed self-test (unattributed error, wrong file, truncated
stream, crash marker, exit-status disagreement).

## Deferred, with the reason

### E. Adjacent string literals in `.file` (pre-existing, unrelated)

`parse_file_directive` joins the two-string `"dir" "file"` form but does not
implement *adjacent* concatenation. Clang emits
`.file 0 "/app""/app/example.c" md5 0x…`, which the current parser turns into
`/app/` — a wrong path in `.debug_line`'s file table — and it drops the trailing
`md5` operand (harmless). Evidence:
`engineering/evidence/2026-10-14-popcount-repair/oracle/clang-arm.s` lines 8-11.
This is a genuine defect, but it is unrelated to the four boundaries, it changes
assembler grammar, and it cannot be compiled in the authoring harness (no Rust
toolchain, no crate cache, 3 GB RAM, no network to crates.io). Landing an
unverifiable grammar change to a debug-info parser at the end of a session is a
worse trade than documenting it: the fix belongs in its own PR with the unit
tests it needs (`"a""b"` → `ab`, `"a" ""` → `a`, escaped quotes inside either
half, and `md5` present/absent).

### F. `resolved_constant` reports width 128 for a bare constant

The width is "the width of the innermost conversion, else 128", and its only
caller clamps with the operation's type width, so no current behaviour is wrong.
It stays as-is with the contract now stated in the function's doc comment; a
future caller that forgets to clamp is the thing to guard, and that is a
`debug_assert` worth adding together with its test, not a silent semantic change.

### G. Per-section row decoding in the DWARF test

`rows()` sorts decoded rows, which hides ordering *between* line programs. The
row *semantics* are still compared exactly, and cross-section state (`is_stmt`,
`isa`) is what the fixture exercises, so the sort costs checking order only. A
per-program decoder is the better shape and needs its own fixture with two
programs that disagree on defaults; deferred rather than half-sharpened.
