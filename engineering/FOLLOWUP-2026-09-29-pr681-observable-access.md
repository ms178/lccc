# FOLLOWUP-2026-09-29 — PR #681 audit response: the observable-access contract

Session scope: re-base `arena/01a0ee1a-lccc` (PR #681, commit `6358140a`) onto
`main` `02d4c906`, adjudicate an external review audit finding by finding,
fix what is real, and close the gate holes the review itself exposed.

Everything below is stated as **measured** or **not measured**. Nothing is
claimed on the strength of a review, and nothing is claimed on the strength
of a build that was not run.

---

## 0. The headline

The review reported one blocker and one high-severity finding. Both were
confirmed at source level and fixed. The more valuable outcome is the third
thing nobody reported:

> **The review's stated reason for the high-severity finding is wrong, and its
> stated reason is the thing that actually matters.**

The review says the deleted `!*volatile` guard escaped detection because
*"rustc's `unused_variables` does not lint bindings introduced by refutable-
pattern destructuring"*, and that *"Clippy can't see it"*.

That is false, and it is false in a way that is cheap to check:

```
$ cat lint2.rs            # the exact shape: refutable enum-struct pattern in an
                         # else-if-let chain, only `v` dead
$ rustc --edition 2021 --crate-type lib -o /dev/null lint2.rs
warning: unused variable: `v`
$ rustc ... -D warnings
error: unused variable: `v`
```

rustc fires `unused_variables` on refutable-pattern bindings exactly as it
does on any other, and `cargo clippy -- -D warnings` — the command CI runs —
turns it into a hard error.

**The real reason is `src/lib.rs:2`:**

```rust
#![allow(
    dead_code,
    unused_variables,   // <-- this one
    unused_mut,
    ...
)]
```

The crate has the lint switched off wholesale, so the dead binding is silent
and CI's Clippy job is structurally incapable of seeing this class. That is
not a one-line observation about PR #681; it is a repo-wide hole in the safety
net for exactly the field the review was worried about, and it is invisible
until you go looking for it. The fix in §1 is built around that fact.

---

## 1. What was fixed, and how it was proved

### 1.1 BLOCKER — the regression corpus went red (confirmed, fixed)

`tests/regression/run_regression.py:142` auto-discovers every
`tests/regression/*.c` and builds each one **standalone**. PR #681 added
`minmax_harness.c` and `minmax_refused_main.c` to that directory root, but
both are half of a pair — they reference kernels defined in
`minmax_shapes/minmax_kernels.c` and are only ever compiled together by
`check_minmax_reduction.sh`. Standalone, they are undefined references and the
corpus is red. The sub-directory glob is non-recursive, so `minmax_shapes/`
was never the problem.

Fix: `git mv` both into `minmax_shapes/`, update the two path variables in the
gate. Verified by the acceptance test that matters, not by inspection:

```
$ python3 tests/regression/run_regression.py --filter minmax -j 2
corpus: 2 tests ... == 2 passed, 0 failed
```

Two minmax tests, not four. The paired fixtures are no longer corpus members.

### 1.2 HIGH — the deleted volatile guard in LICM (confirmed, fixed, *and* the miscompile re-measured)

`src/passes/licm.rs`, `hoist_loop_invariants`, Load arm. The PR converted
`!*volatile && dominates && is_load_hoistable(..)` into an if/else-if/else
chain, and `!*volatile` did not survive the rewrite. Restored as an explicit
first branch, so the debug message names the reason rather than leaving the
guard implicit in a boolean chain:

```rust
if *volatile {
    licm_debug(|| eprintln!("[LICM] load v{} in block {} not hoisted: volatile \
                             access must execute exactly as written (C11 5.1.2.3)", ..));
    false
} else if !(dominates ..) { .. } else { is_load_hoistable(..) }
```

**The miscompile is real, and it is not hypothetical.** Measured on the tree
with the guard deleted, `while (!regs[4]) { }` over a `volatile u32 *`:

```
spin:
    movl 16(%rdi), %edx     <-- hoisted out of the loop
.LBB1:
    testl %edx, %edx
je .LBB1
```

That is the archetypal MMIO poll. It reads the register once and spins on a
frozen value forever. `-O0` and `-O1` are correct; `-O2` is wrong.

### 1.3 The gate that was supposed to catch it does not (not reported — found here)

The review asserts that `tests/regression/check_volatile_pointer_subscript.sh`
"exists in the base and asserts exactly this failure mode". It does not.
Measured, on the deliberately-broken compiler:

| instrument | result on the miscompiled spin loop |
|---|---|
| `check_volatile_pointer_subscript.sh` | **PASS** — "ok" |
| new `check_volatile_spin_loop.sh` | **FAIL** |
| new `check_volatile_destructuring.py` | **FAIL** |

The subscript gate counts *accesses per function*. Hoisting leaves the count
at one, so a count-based assertion passes on miscompiled code. It is the right
instrument for "the access was eliminated" — which is what it was written for
— and provably the wrong one for "the access was moved".

So the review's instruction "wire the existing gate" would have produced a
green CI over a live miscompile. **The gate had to be written, not wired.**

### 1.4 What was added instead: three instruments, each mutation-verified

The rule this session adopted, from the repo's own `check_env_test_hygiene.sh`:
*a gate nobody has ever seen fail is not a gate.* Every new instrument was
checked by breaking the thing it guards and confirming it goes red.

**`tests/regression/check_volatile_spin_loop.sh`** (behavioural, assembly)
Asserts a volatile access is positioned **between the loop header and the
backward branch**. Five volatile probes (`spin_index`, `spin_ptr`, `spin_two`,
`spin_local`, `spin_ptrd`, `store_spin`), each paired with a non-volatile twin
that LICM **must** hoist. The twins are the point: without them a green run
could mean "LICM hoisted nothing at all", and the volatile half would be
decoration. With them, a green run means the two cases were distinguished.

*Mutation:* reintroducing the guard deletion → `FAIL: spin_index … 0 memory
access(es) in the loop body, need >= 1`, while the old gate still reports ok.

**`scripts/check_volatile_destructuring.py`** (static, no build, ~5 s over 496 files)
Fails when a `volatile` destructured from an `Instruction::Load`/`Store`
pattern is bound and never used — the exact shape of a guard disappearing in a
refactor. This exists because the compiler's own lint for it is off
(§0). `volatile: _` is the sanctioned explicit discard.

- 15-case self-test (`--self-test`), run first in CI: a detector nobody has
  seen fire is not a detector.
- Fail-closed by construction: an undelimitable pattern or a raised exception
  is a violation, never a silent pass. This was not theoretical — a
  `'{'`-char-literal defect in the scanner made it report all 496 files
  instead of quietly passing them, which is the correct failure direction.
- The hard part was pattern-vs-construction. `Instruction::Load { volatile,
  dest, .. }` is textually identical whether it destructures or constructs, and
  this tree does both, often in the same function (`inline.rs` and
  `outline_switch.rs` remap instructions by writing the flag back out). A
  first cut reported 7 false positives on real code; all 7 are now self-test
  cases, because a ratchet that cries wolf on the tree it guards gets deleted.

*Mutation:* reintroducing the guard deletion → `src/passes/licm.rs:1255: binds
volatile … and never uses it`.

**`src/backend/generation.rs`** — the ratchet found one genuine site tree-wide:
the codegen dispatcher destructured `volatile` from `Instruction::Load` and
never used it. Adjudicated by hand before touching it: the backend emits
exactly one load per instruction, and the three places that *can* rewrite an
access all check the flag themselves (`generation.rs:550` memory-reordering
barrier, `generation.rs:2445` memcmp fold, x86 `isel.rs:2044/2264/2465`).
Benign — but the binding was silently dead, which is precisely the condition
that hid HIGH-1. Changed to `volatile: _` with the reasoning inline, so the
decision is stated rather than absent. No generated-code change.

**Wiring:** three gates in `scripts/ci_local.sh` and three steps in
`.github/workflows/ci.yml`. `scripts/check_ci_gate_parity.py` green, and the
stale allowlist entry for the subscript gate was removed — the repo's own
parity checker is what told us the wiring was now real.

---

## 2. MEDIUM / LOW findings

| # | Finding | Disposition |
|---|---------|-------------|
| MED-1 | `reloc_oracle_agreement` PASSes vacuously on an empty oracle set (`floor = min(2, 0) = 0`, `all([]) == True`) | **Fixed.** Explicit early `FAIL("no oracles configured to cross-check against")`, with the reasoning in the docstring ("Four ways to fail"). Unreachable from the live call site, which is exactly why it needed a known-answer case. Suite 30 → **32**. |
| MED-2 | Persistent cache ignores compiler version drift | **Fixed, minimally.** `remote()` now records the CE version string *inside* the record and compares it on hit; mismatch is a miss, so the sweep re-measures and overwrites, and the eviction is reported. Deliberately **not** in the cache key: that needs a probe before every compile and doubles requests against a rate-limited API. One probe per compiler per process. Policy documented next to `GODBOLT_CACHE`. Records predating the field are still honoured — invalidating them all at once is a far larger and less safe change than the problem. |
| MED-3 | `loop_preheader.rs` docstring names end-to-end coverage that does not exist | **Fixed.** See §3. |
| LOW-1 | `load_text(errors="replace")` turns corruption into a hit full of U+FFFD | **Fixed.** Strict decode; `UnicodeDecodeError` → miss. Self-test 13 → **14** (well-formed round-trip, undecodable→miss, `load_lines`→`None`). |
| LOW-2 | `stats()` filters `endswith(".tmp")`, but temps are `*.tmp.<pid>.<tid>.json` | **Fixed.** Shared `_is_record()` matching the `.tmp.` infix. |
| LOW-3 | ~110-line dynamic-limit hoist duplicated between the AVX2 and SSE2 reduction transforms | **Not done — see §5.** Deliberate, with reasoning. |
| LOW-4 | `LATE_MINMAX_ONLY` set/clear around `for_each_function` | **Fixed.** `LateMinmaxOnlyScope` RAII guard, held in a block so the flag is provably off before the `n > 0` epilogue. The set/clear pair is the same two operations in the same order as before, and `check_minmax_reduction.sh` (4 contracts, differential vs GCC) is green. **Not claimed:** a cross-build byte-identity comparison of the emitted assembly — that was not run, and the argument above is structural, not measured. |

---

## 3. MED-3 in detail — the promised coverage now exists

`src/passes/loop_preheader.rs` is 528 lines of CFG surgery on by default at
`-O2`. Its unit tests reach only `retarget_edges` and `is_dedicated_to`, pure
functions of a terminator. **They pass identically whether the pass fires once
or never** — which is the whole reason its docstring pointed at a gate.

First thing measured, before writing anything: on the shapes its own docstring
uses, **the pass is inert.** `derived_sum` and `sqlite_shape` are
byte-identical with `CCC_DISABLE_PASSES=loop_preheader`. A gate written on
those shapes would have been a gate that asserts nothing.

A `do`-while shape does fire, because the guard is then *outside* the loop, so
the header is the body and the load dominates every loop block:

| | default | `CCC_DISABLE_PASSES=loop_preheader` |
|---|---|---|
| `dowhile_sum` | `movl (%rdi), %edx` **before** `.LBB3` | `movl (%rdi), %r9d` **inside** `.LBB2` |
| `while_sum` | identical | identical |

So `tests/regression/check_loop_preheader.sh` asserts both directions:

1. **It fires** — `dowhile_sum` hoists with the pass on, does not with it off.
   Both halves are required; either alone passes trivially.
2. **It declines** — `while_sum` (guard at top), `alloca_sum` (alloca loads)
   and `guarded_sum` come out **byte-identical**, not merely equivalent, and
   keep their loads in the loop.
3. **It does not unlock the unsound hoist** — `guarded_sum` is the SQLite
   `if (p == 0) return 0;` shape from the pass's own docstring. Its outside
   predecessor is not dedicated, so hoisting `p[0]` into it would dereference
   NULL on the early-exit path. This is the assertion that would catch a
   future "widen the dedicated-preheader test" change.

*Mutation:* `CCC_DISABLE_PASSES=loop_preheader` in the environment makes the
"pass on" half fail (`dowhile_sum … 1 memory access(es), want exactly 0`). The
gate can be seen to fail without a rebuild.

The shapes live in `tests/regression/loop_preheader/`, **not** the corpus root
— the §1.1 lesson applied to the file written to fix §1.1. The docstring now
names files that exist.

### A shared primitive, because two gates answering one question must agree

`tests/regression/lib_loop_bounds.sh` holds "is this access inside the loop?"
in one place. Two instruments that drift apart on the only question they exist
to answer is how a suite ends up with a gate that can see a hoist and one that
cannot — which is precisely the §1.3 failure. Both gates now source it.

---

## 4. Validation actually run

`scripts/ci_local.sh --fast` was run end to end on this host.

**First run: 104 passed, 7 failed, 5 skipped — and all 7 failures were host
provisioning, not code.** `ci_local.sh` expects the multilib contract that
`ci.yml` installs explicitly (`gcc-multilib g++-multilib libc6-dev-i386`);
without it, seven gates fail on missing 32-bit system headers and startup
objects. After `apt-get install gcc-multilib g++-multilib libc6-dev-i386`:

| gate | first-run failure | after provisioning |
|---|---|---|
| `notype-code-routing` | i686 link | **PASS** |
| `i686-integer-isa-parity` | i686 headers | **PASS** |
| `map-i64-two-lane` | i686 scalar assembly | **PASS** |
| `nocfi-peephole-parity` | `bits/libc-header-start.h: No such file` | **PASS** |
| `reassoc-latency` | `bits/types/FILE.h: No such file` | **PASS** |
| `copy-alias-sizes` | `ld: cannot find crti.o` | **PASS** |

The errors are categorically not ours: they are preprocessor and linker
failures on `#include <stdio.h>` and `crti.o`, which happen before any
optimization pass runs, and **this diff touches no frontend or linker code**.
As a direct check, `target/fastbuild/lccc -m32` on `#include <stdio.h>` failed
before the install and succeeds after it, while the host `gcc -m32` succeeded
either way — so the headers were always present and lccc's `-m32` include path
simply had no multiarch package to resolve them through.

**Final run after provisioning: 110 passed, 1 failed, 5 skipped.** The 5 skips
are the `--fast`-excluded slow gates, which CI runs on every push. The single
failure is `linker-suite`, and it is worth being exact about it:

```
== linker tests: 300 pass, 1 fail, 0 warn, 0 skip (oracles: bfd mold) ==
[FAIL] reloc_pc32_out_of_range_diagnosed_on_script_path
  only 1 of 2 oracles could express an opinion, need 2: {'bfd': ('refused', ...),
  'mold': ('inapplicable', 'mold: fatal: t.ld:1: ENTRY(probe) ^ unknown linker
  script token')}
```

`reloc_oracle_agreement` **does not exist on `main`** — the PR introduces it.
The base logic at that call site was

```python
if not agree:                 FAIL
elif not all(ok for _, ok in agree):   FAIL
else:                          PASS
```

so with bfd refusing and mold `inapplicable`, `agree == [("bfd", True)]` and
**the base passes**. The PR's applicability floor (`min(2, len(oracles)) = 2`
against one capable oracle) turns that into a FAIL.

So: **not a regression from this session's work** — the MED-1 edit adds only an
early return for an *empty* oracle set, which is unreachable here — but a real
**PR-introduced, host-dependent** behaviour change. This host has bfd + mold
2.37 and no `wild`; the suite's third oracle is `wild`, not `lld`
(`run_linker_tests.py:10852`), so it cannot be satisfied locally. On a host
where the non-reference oracles cannot parse the fixture's linker script, the
floor of two capable opinions is unreachable and the test is unpassable
regardless of what lccc does.

Deliberately **not** "fixed" here, because both available fixes are wrong:
lowering the floor would delete the property the gate exists to protect, and
rewriting the fixture to drop `ENTRY(probe)` would weaken a test named
`..._on_script_path` to satisfy one host's linker. Filed as **OBS-6** with the
recommended fix (a fixture every configured oracle can parse, or an explicit
`SKIP` carrying the incapability reason rather than a FAIL that reads like a
conformance verdict against lccc).

### Everything else

| check | result |
|---|---|
| `cargo build --profile fastbuild -j2` (project policy) | clean |
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --all-targets --profile fastbuild -- -D warnings` | **clean** |
| `cargo test --profile fastbuild --all-targets` (the `cargo-test` gate) | PASS |
| `run_regression.py --filter minmax` | 2 passed, 0 failed (was 4 with 2 undefined-reference failures) |
| `check_minmax_reduction.sh` (4 contracts) | PASS |
| `check_volatile_spin_loop.sh` | PASS; **FAIL** on the mutated compiler |
| `check_volatile_pointer_subscript.sh` | PASS (and proven blind — §1.3) |
| `check_volatile_destructuring.py` | ok, 496 files; **FAIL** on the mutated compiler |
| `check_loop_preheader.sh` | PASS; **FAIL** with the pass force-disabled |
| `test_godbolt_cache.py` | PASS, 14 checks |
| `test_reloc_oracle_verdict.py` | PASS, 32 cases |
| `check_ci_gate_parity.py` | PASS, 101 commands |
| `check_doc_links.py` | PASS — **it caught a stale path** left by the fixture move, now fixed |

Method note: every "the gate can fail" claim above is a **mutation**, not a
reasoning step. The volatile guard deletion was reintroduced into a real
compiler build and measured, then reverted. The preheader gate was mutated via
an environment variable, for free.

### A note on cost

`ci_local.sh --fast` is ~30 min on 2 cores / 1.9 GB with the 4 GB swap, most
of it the monolithic `fastbuild` test binary's link. `ONLY=<gate>` does **not**
skip the `build` and `cargo-test` gates, so iterating on one gate through
`ci_local.sh` costs a full rebuild; run the gate script directly instead. Worth
fixing in the script, not fixed here.

---

## 5. Why LOW-3 was not done

`transform_reduction_avx2` and `transform_reduction_sse2` share a ~110-line
dynamic-limit hoist. Extracting it is the right call *eventually*, and the
twins do correctly share the `reduction_remainder_references_sound`
precondition, so the duplication is genuinely mechanical.

It was left alone this session because: the transform is the single largest
behavioural change in the PR, it is guarded by a 4-contract differential gate,
and the mutation-verified evidence in §4 would all have to be re-earned
against a new build. The honest cost of a late refactor here is a
regression window on the one optimisation the PR exists for, to remove a
maintainability wart that has caused no defect. That trade is worth making
**when the session has room to re-validate**, not in the same pass as a
correctness fix. It is first on the next session's list, and it should be done
as its own commit with its own measurement.

---

## 6. To-do, in priority order

### 6.1 Correctness and gates

1. **`src/lib.rs` has `#![allow(unused_variables)]` crate-wide.** This is the
   real root cause behind §0 and it is much larger than one deleted guard. It
   was left as-is: removing it produces a large warning set and is a
   separate, well-scoped change. The right move is a counted ratchet (count
   the sites today, never raise it, migrate the safety-relevant ones first)
   in the style of `check_env_test_hygiene.sh`. **Until that exists, the
   static ratchet in §1.4 is the only thing covering this field.**
2. **Widen `check_volatile_destructuring.py` to `AtomicLoad` / `AtomicRmw`
   / `AtomicStore`.** Excluded today on the reasoning that `_Atomic` accesses
   are already unremovable, so no pass needs to branch on them. That is an
   argument, not a measurement. The gate is generic; the exclusion is one
   regex away from being wrong.
3. **Same ratchet for `Fence` and for `semantic_volatile`.** `Instruction`
   carries a second, distinct flag (`semantic_volatile`, seen at
   `licm.rs:1768`, `1879`, `1998`). It has exactly the same silent-drop
   exposure and currently has **no** instrument at all.
4. **Close the ratchet's documented blind spot.** A guard whose use sits more
   than `USE_DISTANCE_LINES` lines below its own pattern is missed. The bound
   exists because re-deriving Rust's arm grammar produced 7 false positives on
   real code first. A real parser (or `syn`) would remove the bound; until
   then the behavioural gate is the backstop, which is why both are wired.

### 6.2 The PR's own substance

5. **MINMAX-4** (re-open the other reductions). Unchanged and still correct:
   the late rerun's guarded-sum transform measured **+1.05 %** whole-program
   retired instructions on `zlib_ng_adler32` (396,349,832 → 400,499,785) with
   static instructions 335 → 370 and stack references 32 → 44. The steady
   state did get denser (14 → 12 instructions per 32 bytes); the prologue,
   accumulator traffic and spills cost more than that saves. The work is the
   accumulator/traffic side, not the transform's legality.
6. **LOW-3**, as its own commit, per §5.
7. **The loop-preheader pass is 528 lines that fire ~0 times by default.** The
   journal records it firing 18 times under `CCC_LOOP_ROTATE=1`, and §3 found
   a plain `-O2` shape that fires too. That combination — a default-on
   CFG-surgery pass whose hit rate at default settings is near zero — is worth
   an explicit decision: measure the insertion rate over the golden workloads
   and either find the shape family it is actually good for, or gate it off
   by default. Right now it is paid for on every `-O2` compile and its value
   is invisible.

### 6.3 This session's own debt

8. `check_volatile_spin_loop.sh` asserts on the **innermost** loop body. A
   volatile access hoisted out of an *inner* loop into an *outer* one would
   pass. Not reachable today (no pass sinks outward) but the helper should
   support "inside any enclosing loop", and the guard should say which.
9. `_STALE_EVICTED` in `godbolt_oracle.py` is reported but not asserted. A
   test that drives a version change and checks the eviction would make MED-2
   a verified behaviour rather than a documented intent. Requires network or
   a fixture; currently neither.
10. The `'{'`-char-literal defect in the scanner was found by accident (it
    reported 496 files) rather than by a test. There is now a lifetime
    self-test case, but the general "the scanner corrupts the text" failure
    mode has no invariant test — a round-trip check that the stripped text
    has the same brace balance as the original would catch the whole class.

---

## 7. One-paragraph summary for the next agent

PR #681's blocker (a corpus glob collision) and high-severity finding (a
deleted `!*volatile` guard in LICM) were both real and are both fixed. The
guard deletion is a **measured** miscompile: it hoists a volatile MMIO load
out of a spin loop at `-O2`. Fixing it is necessary but not sufficient — the
review's claim that the existing subscript gate "asserts exactly this failure
mode" is **false**, and that gate passes green on the miscompiled compiler,
because it counts accesses and a hoist preserves the count. The lesson worth
more than the fix is the shape of it: *the instrument everyone pointed at was
measuring the wrong quantity, and it was measuring the wrong quantity in a
way that only a mutation test could reveal.* Hence this session's rule — no
gate ships until it has been seen to fail. Three new instruments
(`check_volatile_spin_loop.sh`, `check_volatile_destructuring.py`,
`check_loop_preheader.sh`), each mutation-verified, each wired into both
`ci_local.sh` and `ci.yml`, and each documented in place with the reasoning
that produced it. The repo-wide hole that made the bug invisible —
`#![allow(unused_variables)]` at `src/lib.rs:2` — is named, not fixed, and is
the first item on the next list.
