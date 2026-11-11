# Rebaseline 2026-11-10: `pr67106.c` now compiles (`-fpack-struct`)

## What changed and why this file exists

The per-case ratchet correctly refused [run 42817930714](https://github.com/ms178/lccc/actions/runs/42817930714)
(head `470a775acdda2cb622ebe1873e2706b52dee2cb3`, the merge of #1171) on both
x86 targets with exactly one problem each:

```
["pr67106.c", "-O2"]: compile-fail -> pass
```

`pr67106.c` is `/* { dg-options "-g -fpack-struct" } */`. #1171 implemented
`-fpack-struct[=<n>]`, so the old failure

```
ccc: error: -fpack-struct: LCCC does not implement this data-model request, ...
```

no longer reproduces. The status change is a **fix**, and the ratchet's own
contract is that a fix, like a new failure, requires explicit baseline review
rather than being silently accepted. This note is that review.

## Seed run selection (the two previous seed runs cannot be reused)

The committed baselines were captured from run 42535980102, whose run-level
conclusion was `failure` for a *ratchet identity mismatch* (`runner_sha256`).
`scripts/torture_ci_capture.py` rejects any job whose non-ratchet steps are not
`success`, so no later "good instance of the old outcomes" is capturable. The
only valid input is this run, and its pre-existing problems are disjoint from
the change under review (identity now matches; every other case is unchanged).
It is used with `--allow-ratchet-failure`, whose documented purpose is exactly
"complete error annotations plus separate human review".

## Review procedure actually executed

1. Retrieved the run and job metadata with `gh api` (never a cached copy).
2. Retrieved `check-runs/<job>/annotations --paginate` for both arch jobs.
3. Ran the sanctioned capture tool:

   ```
   python3 scripts/torture_ci_capture.py \
     --run .../run.json --jobs .../jobs.json \
     --annotations-dir .../anns --out-dir .../out --allow-ratchet-failure
   ```

   It validated provenance/inventory agreement, complete outcome chunks
   with zero omissions, case counts, and the absence of reference failures.
4. Diffed each proposed file against the committed baseline. The proposed
   `comparison` blocks are **byte-identical** to the committed ones, so this is
   not identity drift: the corpus pin (`c94b688a…`), case-set digest, selected
   source digest, dependency digest, corpus stamp, runner hash, harness
   dependency hash, oracle hash, oracle environment, hosted image and search
   environment are all unchanged. Re-verified locally on this branch that
   `harness_dependency_sha256` and `runner_sha256` still hash to the values in
   the committed baseline, so a merge of this branch cannot invalidate them.
5. Diffed the outcome rows. Across all four legs there is **exactly one**
   changed row, `pr67106.c -O2` on both compile legs (6 → 5 compile failures,
   1994 → 1995 / 1987 → 1988 passes). The execute legs differ only in
   `source_run`/`source_head`.
6. Re-ran the ratchet logic itself on the real identity with the observed
   outcome set. With the committed baseline it produces the single problem
   string `["pr67106.c", "-O2"]: compile-fail -> pass`, character-identical to
   the hosted annotation; with the proposed baseline it produces no problems.

## Independent proof that this is a fix, not a masked failure

Executed in this sandbox with the pinned harness cache (rustc 1.99.1,
GCC 16.2.0, Clang 23.1.5), fastbuild compilers:

| Probe | LCCC x86-64 | GCC 16.2 | Clang 23.1.5 | LCCC i686 | GCC 16.2 `-m32` |
|---|---|---|---|---|---|
| `pr67106.c -O2 -g -fpack-struct -c` | rc 0 | rc 0 | rc 0 | rc 0 | rc 0 |
| `sizeof(struct S)` | 12 | 12 | 12 | 8 | 8 |
| `_Alignof(struct S)` | 1 | 1 | 1 | 1 | 1 |
| `offsetof(struct S, a)` | 8 | 8 | 8 | 4 | 4 |

The anonymous `struct { S *s; }` member, the typedef'd self-referential
pointer, and pack-1 layout agree on both ABIs, so the acceptance is not
"compile something wrong": the layout LCCC produces is the layout GCC and Clang
produce. The `-g` flag is included, so debug info generation is on the path
too. On the DWARF side this TU is a non-issue in both compilers: GCC 16.2's
`.debug_info` for `pr67106.c` contains only a `DW_TAG_compile_unit` and an
attached-to-nothing `int` base type (no `DW_TAG_structure_type` at all, because
nothing in the TU is emitted or used), which is exactly the
`check_debug_info_flags.sh` pin-3 contract the DWARF campaign documents —
"silence is only honest where there is nothing to describe". LCCC writes the
line tables only, matching that contract.

## What this note does *not* claim

- It does not claim the remaining five compile-leg `compile-fail` cases
  (`limits-caselabels.c`, `limits-exprparen.c`, `limits-structnest.c`,
  `pr115143-2.c`, `pr115143-3.c`) are fixed; they are carried forward
  unchanged, with unchanged diagnostics.
- It does not claim the corpus is green: reference-compile skips and the
  parser-limit refusals are unchanged and intentional.
- It does not claim the torture lane is hermetic; the caveats in this
  directory's `README.md` still apply.
- It does not re-enable the previously-recorded `-fpack-struct` refusal. That
  refusal was a deliberate design decision (silently ignoring a data-model
  request corrupts ABI); #1171 replaced it with an implementation, which is the
  only change that makes this status transition legal.

## Expected effect

The next scheduled/main whole-corpus run compares against these files and
passes, provided the identity fields still match. Any future change to
`pr67106.c`'s outcome, phase, exit code or bounded diagnostic digest fails
again for fresh review.
