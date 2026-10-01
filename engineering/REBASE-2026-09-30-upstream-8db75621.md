# Rebase onto 8db75621: what upstream got right, and where it contradicted itself

Date: 2026-09-30. Rebasing my branch onto `github2/main` = `8db75621`
(12 commits, 182 files, +10281/−804 since my previous base `ec08e6a6`).

Result: 14 commits on top (one dropped, see below). One conflict needed a
judgement call; one commit of mine was superseded upstream.

## The conflict: a `debug_assert!` that cannot fail

`src/passes/loop_preheader.rs`, `preheader_would_unlock_a_hoist`. Upstream
had deleted

```rust
if !natural_loop.body.iter().all(|&b| dominates_block(idom, header, b)) {
    return false; // not must-execute: LICM refuses the hoist anyway
}
```

and put a `debug_assert!` in its place, asserting that **no non-header body
block dominates the header**, under a comment saying this keeps the
header-only shortcut honest. I kept the check and rejected the assert.

### 1. The assert cannot fail

A natural loop is defined so that the header dominates every block of its
body. So if some `b != header` also dominated the header, then `header dom b`
and `b dom header` both hold, and dominance antisymmetry forces `b == header`
— a contradiction. The assert is therefore true by construction of
`NaturalLoop`: it tracks the natural-loop builder, not anything this pass
decides.

### 2. It does not detect the failure its comment names

The comment claimed the assert would catch "LICM's must-execute rule being
relaxed, so the header-only shortcut silently stops being equivalent".
Relaxing LICM changes what LICM hoists. It changes nothing about dominance
among loop blocks. The assert stays green through exactly the regression it
was written to catch — and the new test pins that, on a fixture that violates
the invariant, where the assert passes.

### 3. It costs exactly what it replaced

The assert walks the body with one dominance query per block, O(|body| ×
depth) — the same as the `all(...)` it displaced. So the change bought
nothing under `debug-assertions`, and in release it swapped an always-on
fail-closed guard for one that is compiled out. `fastbuild` inherits
`release`, so the shipping compiler would have had no check at all.

### What I kept is a real guard

`NaturalLoop` has **public fields and no validation** — `header` and `body`
are a `usize` and an `FxHashSet<usize>` that anyone can construct. So a
caller really can hand this pass a loop whose body contains a block the header
does not dominate, and on that input the kept check returns `false` and the
pass declines to insert. That is the difference: one guard is unreachable and
one is load-bearing.

## The file already states the correct doctrine — and then ignores it

This is the part worth sending upstream. Twenty lines above the assert,
`apply_insertions` guards the entry-block case with a hard refusal, and the
comment above it says:

> A `debug_assert!` would be the wrong instrument for this invariant: it
> panics in debug builds and is compiled OUT of release, which inverts the
> intent for a guard whose entire purpose is to contain a catastrophic,
> otherwise-silent outcome. **Assertions document invariants you believe
> cannot be violated; refusals enforce the ones you are not willing to bet
> the function on.**

That is exactly right, and it is exactly the reasoning that condemns the
assert 20 lines below. The same file applies its own doctrine to one
invariant and not to the other. My resolution makes the file consistent with
the standard it had already written down: hard refusal where a wrong answer
would be silent, no assertion dressed up as a guard.

(For the record: 5 hard refusals, 6 `debug_assert!` occurrences in the file —
but the 6 include call sites and comments; the one I removed was the only
one using an assertion to enforce a decision the pass actually makes.)

## The new test, and the non-discriminating first attempt

The test calls `preheader_would_unlock_a_hoist` itself on a malformed loop
and asserts it returns `false`, and that a pruned body returns `true`.

My first version re-derived the predicate locally and never called the
function. It passed with the guard deleted — a non-discriminating test, the
same defect class as the volatile lint's old `eprintln!` case, and the third
one in this series. I only caught it because I ran the deletion mutant
without thinking to check the result was a *failure* first. The rewritten
version fails with "a body the header does not fully dominate must NOT unlock
a hoist" when the guard is removed.

That is three for three. The pattern is mine and it is consistent: I write
the claim before I check the mechanism. The cure is not more care, it is the
mechanical check — mutate the thing, demand the suite go red, and never
accept a green that I did not see go red first.

## Superseded: my godbolt memoisation commit

`039e7296` added a one-catalog-fetch-per-process memo and a `PROBE_VERSION`
kill switch. Upstream independently solved the same problem better: a
`/api/compilers/c` probe (one request for the whole sweep, not the full
multi-megabyte catalog), `revalidate()`, and `--revalidate` / `--max-age`,
with revalidation **opt-in** so a default sweep makes no catalog request at
all. That is strictly more thorough than what I had — it removes the
per-sweep fetch rather than deduplicating it.

I dropped the commit rather than porting it. Re-adding a second, differently
named mechanism for a solved problem is the kind of duplication that makes a
codebase worse, and the reviewer's own instruction for this round was not to
touch what checks out.

## Environment

The sandbox was reset repeatedly during this session, taking `.git`, the Rust
toolchain, and the swapfile with it each time. Restored: `.git` from the
bundle, `cargo` 1.98.1, 8 GiB swap (swappiness 10, in `/etc/fstab`), plus
`lld`, `gcc-multilib` and `g++-multilib` — the last of which need
`apt-get update` first, because stale package lists give 404s that look like
a missing package.

`scripts/ci_local.sh --fast` was deliberately **not** run, per instruction.
Validation was surgical: the two `loop_preheader::tests` groups, the volatile
lint self-test and its mutation proof, `rustfmt --check`, and hand-mutation
of each guard.
