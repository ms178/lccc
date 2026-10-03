# FOLLOWUP-2026-10-03B — A13/A14 libcall synthesis, rebased onto `9f6cfcf`, with a red-team audit of upstream PR #742

Session 4 of the EDG-transplant Daueraufgabe. Base: upstream `9f6cfcf`
(merge of PR #742, `21cc329`). Deliverable: `/home/user/ms178-1.patch`
(snapshot S12+, `APPLIES-CLEAN` on a pristine clone of `9f6cfcf`).

This document records (1) what A13/A14 is and why the final shape differs
from the first implementation, (2) the full evidence, (3) a red-team audit of
my own work, and (4) an audit of upstream's merged choices — including one
measured defect the merge's own rationale does not hold for.

---

## 1. A13/A14 in one paragraph

Passes that replace user code with a library call (`loop_idiom` →
`memcpy`/`memmove`, `loop_memset` → `memset`, `fortify_fold` →
`puts`/`fputs`/`putchar`/`fputc`/`fwrite`) and the backend's constant-size
`memcpy`/`memset` expansion may only act while the callee really is the
library function. A TU that defines the symbol (glibc's own string routines
do) makes the synthesised call resolve into a definition the compiled code is
part of: unbounded self-recursion, or a silently ignored user definition.

Three mechanisms, one shared collector:

| piece | file | role |
|---|---|---|
| state | `src/common/builtin.rs` | `BuiltinPolicy` (blanket + per-name withdrawals, GCC 16.2 semantics) and `defined_symbols(&IrModule)` — the single collector both halves use (asm label wins, alias + target, asm-label keys, non-declarations only) |
| pass gate | `src/passes/libcall.rs` | `LibcallAllowance::{from_module, may_use}` snapshot once per compilation in `run_passes`; consulted by `loop_idiom`, `loop_memset`, `fortify_fold` |
| backend gate | `src/backend/libcall_policy.rs` | `set` (driver), `set_defined` (module, at `collect_symbol_sets`), `may_assume_builtin(name)` = not defined && not withdrawn; **fail-closed** before publication |
| CLI | `src/driver/cli.rs` | `-fno-builtin`, `-fno-builtin-<fn>`, `-fbuiltin`; `-ffreestanding` implies blanket withdrawal, `-fhosted` restores (last wins, per-name sticky — measured on both pinned oracles) |

### 1.1 The design decision that changed during re-implementation

The first implementation gated the *dispatch* in `recognize_idioms`:

```rust
let rewritten = if matched.needs_overlap_guard || matched.needs_zero_guard {
    libcalls.may_use("memmove") && rewrite_guarded_copy_loop(..)   // WRONG
} else {
    libcalls.may_use("memcpy") && rewrite_copy_loop(..)
};
```

That is wrong, and the regression gate caught it within minutes on the
rebuild: `-fno-builtin-memcpy` still produced `call memcpy@PLT` for
`void cp(unsigned char *restrict d, const unsigned char *restrict s, size_t n)`.
Root cause: the guarded rewrite is entered for the *zero* guard alone when
`restrict` proves disjointness — and in that case it emits **`memcpy`**, not
`memmove` (the call name depends on `needs_overlap_guard`, not on which guard
sent you there). The gate asked about the wrong symbol.

Final shape: **the gate lives where each rewrite computes its callee**, one
source of truth per function:

* `rewrite_copy_loop`: refuses unless `may_use("memcpy")`, next to
  `let call_name = "memcpy";`
* `rewrite_guarded_copy_loop`: computes `call_name` from
  `m.needs_overlap_guard` (memmove) or else memcpy, refuses unless
  `may_use(call_name)`, and *uses that same variable* for `make_copy_call`.

So the check and the emission cannot drift. `fortify_fold` gets the same
property differently: every stdio synthesis funnels through `classify`, and
`run` filters its returned `Decision` through
`stdio_fold_permitted(&decision.insts, libcalls)` — a scan for
`puts|fputs|putchar|fputc|fwrite` calls among the replacement instructions
(one funnel, so no site can be missed; a dropped decision leaves the original
`printf` in place).

---

## 2. Evidence

### 2.1 Structural gate `tests/regression/check_libcall_synthesis_no_selfcall.sh`

45 rows, 5 groups, **green on the rebuilt compiler** (`all checks passed`,
`GATE_EXIT=0`, `results/gate-libcall-*.log`):

1. **Refusals** (15): `def_memcpy/def_memmove/def_memset` at `-O2/-O3/-Os` emit
   0 calls in their own body; `def_puts` uses the symbol-aware assertion
   (`def_puts` legitimately keeps `call printf@PLT` — "no calls at all" would
   have been a false failure); `static_memcpy.c` proves a TU-local `static`
   definition counts.
2. **Positive controls** (9): `free_copy`→`memcpy`, `free_fill`→`memset`,
   `free_puts`→`puts` at all three levels — the optimisation must not be lost.
3. **Flag contract** (8): `-fno-builtin`, `-ffreestanding`, `-fno-builtin-<fn>`
   stop synthesis; `-fno-builtin -fbuiltin` and `-ffreestanding -fhosted`
   restore it; per-name withdrawals stay per-name.
4. **Expansion** (9): constant-size `memset`/`memcpy` calls are inlined by
   default; every withdrawal keeps the call; `__memset_chk`/`__memcpy_chk`
   stay inlined under all four spellings (both pinned oracles agree, so gating
   them would be wrong); a **global TU definition** keeps the call; a
   **static** definition may inline but must not bake the library fill.
5. **Runtime** (3): the refusing program honours its own `memcpy`/`memset`
   (exit 0); a TU-defined `puts` prints once and returns (pre-fix: SIGSEGV);
   and a **forced** call through a `volatile` pointer to a TU-defined
   `memset` (nothing can fold it away) returns its own semantics.

### 2.2 Runtime matrix against the pre-fix oracle and GCC (`audit/session3_runtime.sh`)

| row | lccc post | lccc pre | gcc 14.2 |
|---|---|---|---|
| `usercall.c` (`-O2`, TU memset ignores `c`) | 0 | **7** | 0 |
| `def_global_memset.c` + `rt_def.c` | 0 | **7** | 0 |
| structural: self-calls in `memset` / baked fill / `movl $7,%esi` | 0 / 0 / 1 | **1 / 1 / 0** | **1 / 0 / 0** |
| `rec_puts.c` (`-O2`) | `hello`, 0 | **139** | **139** |
| `puts_fixture_test.c` (`-O2`) | `hello`, 0 | **139** | **139** |
| `rec_memset.c` / corpus fixture | 0 | **1** | 0 |
| `forced_memset_call.c` (`-O2`, volatile pointer) | **0** | **139** | **139** |

The last row is the sharpest: **GCC 14.2 SIGSEGVs at `-O2`** on a program whose
TU-defined `memset` is called through a `volatile` function pointer — its own
compiled `memset` is `xorl %esi,%esi; call memset@PLT` (self-recursion with the
same `d`/`n`), and it only *hides* this when its fold removes the trivial
caller. Pre-fix lccc did the same; post-fix lccc is the only compiler of the
three that runs the program correctly. The `puts` rows are the same story
(GCC 16.2 / Clang 23.1 / ICX 2025 emit the self-call; GCC 14.2 crashes), which
is why `tests/regression/puts_definition_no_selfcall.env` is marked
`LCCC_NO_COMPARE=1`: **no correct oracle exists for that shape.**

### 2.3 Corpus fixtures

* `libcall_synthesis_never_selfcalls.c` (+`.flags -O2`) — one runtime program
  covering both halves: a rewritten copy loop would run the TU's *reverse*
  `memcpy` (detectable), and a library-contract expansion of `memset(b,7,8)`
  would store 7s. Pre-fix 1, post-fix 0, GCC 0.
* `puts_definition_no_selfcall.c` (+`.flags`, +`.env`) — lccc-only fixture for
  the printf→puts self-call.

---

## 3. Red-team audit of this session's own work

| # | Finding | Severity | Resolution |
|---|---|---|---|
| R1 | **Gate placed on the wrong symbol** (§1.1): the guarded rewrite was gated on `memmove` while its zero-guard-only shape emits `memcpy`. Caught by the gate's own `-fno-builtin-memcpy` row on the first rebuild | would have shipped a hole (TU-defined `memcpy` + zero-guard rewrite = self-call) | gate moved next to each callee decision; rewrite uses the same `call_name` variable it gated |
| R2 | A tautological assertion (`assert!(x \|\| !x)`) I wrote in the backend policy test — it asserted nothing | would have shipped as fake coverage | replaced with a real fail-closed test (`unpublished_state_fails_closed`, after a test-only reset of both cells) |
| R3 | The two policy tests mutate **process-global** cells and run on a shared thread pool; without serialisation they interleave and fail non-deterministically | flaky CI | process-global `POLICY_LOCK` (same reasoning as `test_support::ENV_LOCK`, documented in place) |
| R4 | `cargo test -j 2` on this 1 984 MB host gets **SIGKILLed by the OOM killer** during the lib-test compile (measured: `signal: 9`), exactly as `ci_local.sh` documents for < 6 GB hosts | wasted cycles | use CI's own recipe (`-j1`, `debug=0`, `incremental=false`, the `cargo-test` gate) instead of a hand-rolled invocation; never raise jobs here |
| R5 | `pkill -f "cargo test"` matched its own command line and killed the invoking shell (again) | self-inflicted | self-excluding `[c]argo` pattern / `pkill -x rustc` |
| R6 | The harness wiped the whole worktree between turns (no `.git`, no `src/`, no toolchain, no swap). All A13/A14 sources were lost and re-implemented; only the *evidence* (fixtures, logs) survived in `audit/` and `results/` | catastrophic-but-recoverable | snapshot **S12** written immediately after the first rebuild; base ref re-pointed to `9f6cfcf`; this session's rule: snapshot after every validated increment, and keep standalone reproducers in `audit/` |
| R7 | My first `run_passes` wiring inserted the new argument in the wrong position (`after the paren`, not `after &mut module`) — the check told me in one round-trip | compile error only | fixed; the scripted-edit approach (`stage1/stage2.py` with anchor counting, atomic write) is why nothing else drifted |
| R8 | A first sweep script measured with `bc` (not installed) and captured `tail`'s exit code instead of the compiler's, producing nonsense timings; the "1000-deep chain took 0.023 s" figure was an artifact of the broken capture | would have published garbage numbers | all measurements redone in Python with explicit rc/size capture before any number entered this document |

---

## 4. Audit of upstream PR #742 (`21cc329`) — "parser frame budgets, C23 #embed, truthful `__has_warning`"

### 4.1 What the change does

* **Parser frame budgets**: one shared live-frame counter (`parser_frame_depth`)
  replaces the expression-only counter; class budgets `PARSER_FRAME_BUDGET =
  32768` (expressions/initializers/records-are-not), `BLOCK_FRAME_BUDGET = 8192`
  (statements/blocks: one brace level = two counted frames), `TYPE_FRAME_BUDGET
  = 2048` (record definitions); a single `nesting_budget_diagnosed` latch keeps
  the verdict at exactly one diagnostic; termination guarantees for the
  compound-statement and struct-field loops; `alignof` memoization for named
  records.
* **C23 `#embed`** family: full parameter grammar (`limit` in all C integer
  forms, `prefix`/`suffix`/`if_empty`, `clang::offset`), duplicate-param
  detection, adversarial-limit tests, byte-oracle test.
* **Truthful `__has_warning`** and diagnostics: `MAX_RENDERED_WARNINGS = 200`
  (counting continues past the cap so `-Werror` and the summary stay exact),
  char-based snippet windowing (UTF-8 safe, 160-char window).
* **173 files made executable** (mode-only) and the new gate registered in
  both orchestrations.

### 4.2 What I verified, and what I think of the choices

**Agree — verified by measurement:**

1. **Crash-class elimination works.** Sweep of `(…(1)…)` at depths
   1 000/4 000/4 800/5 000/10 000/50 000/200 000: lccc never crashes — every
   row above the budget returns rc=1 with a clean single diagnostic
   (`results/`; `audit/`-style sweep recorded in this session's log).
   The budget boundary behaves exactly as designed: depth 2 100 records returns
   `error: type nesting too deep (exceeds 2048 parser frames)` in **0.69 s**.
2. **The commit's cross-compiler claim is true.** On the same generated inputs,
   **GCC 14.2 ICEs** (rc=4, internal compiler error) from depth ~50 000, where
   lccc now emits a bounded diagnostic. Their choice to fix the class rather
   than to match a crashing reference is right.
3. **A shared counter with one latch** is the right structure: the resource is
   one stack, the user-visible contract is one diagnostic. Per-class counters
   would have produced cascades.
4. **Making the gate run in both ci_local and ci.yml** (and making 173 scripts
   executable) is correct discipline — `bash x.sh` masks a missing `+x` bit,
   and standalone gates are the ones that rot. I had to satisfy the identical
   parity contract for my own gate this session (`check_ci_gate_parity.py`,
   135 commands, PASS), so I can attest the rule bites.
5. **`MAX_RENDERED_WARNINGS = 200` rather than Clang's 20**, justified by the
   corpus pinning exact warning output — a deviation defended by a concrete
   constraint instead of fashion. Char-based snippet windowing is the correct
   fix for caret alignment (byte slicing can split a UTF-8 sequence).
6. **Named-record `alignof` memoization**: measured, depth-8 000 chains of
   *named* records compile in **0.16 s** where the commit message records
   11.6 s before the fix.

**Disagree — one measured defect, with data:**

7. **The record budget's stated compile-time rationale does not hold.**
   `TYPE_FRAME_BUDGET`'s doc says the bound "caps the worst-case compile of a
   hostile-but-legal TU at ~0.6 s". Measurement (this session, this host):
   **anonymous** nested record definitions cost super-quadratic time in the
   `lowering` phase *below* the budget:

   | depth | lccc `-O2 -S` | phase breakdown | gcc `-O2 -c` |
   |---|---|---|---|
   | 200 | 0.155 s | lowering dominates | 0.022 s |
   | 400 | 0.552 s | lowering ≈ total | 0.049 s |
   | 600 | 1.50 s | lowering 1.441 s (parse 0.007, sema 0.008, codegen 0.001) | ~0.05 s |
   | 800 | 2.81 s | lowering 2.810 s | ~0.05 s |
   | 1 000 | 5.93 s | — | ~0.06 s |
   | 2 000 | **> 300 s (killed)** | — | ~0.1 s |

   Fitted exponent ≈ 2.1–2.5 between 400 and 800 (super-linear at least;
   400→1000 is ×10.7 for ×2.5 depth). Shape matters: **named** nested
   definitions are linear (depth 400: 0.07 s), so this is specifically the
   anonymous-record path in `lowering`, not "deep types are slow". The
   reproducer is `audit/nested_records_scale.py` (`--phases` for the
   breakdown, `--named` for the control).
   *Impact:* a hostile-but-legal TU, or a generated header with deeply nested
   anonymous structs/unions, occupies the compiler for minutes inside the
   legal window `[~400, 2048]`; the documented cap is off by ~3 orders of
   magnitude at the boundary. Not a correctness bug, and not *introduced* by
   PR #742 (the cost lives in `lowering`, which the PR does not touch) — but
   the budget **choice** leans on a cost model that does not exist.
   *Options, in the order I would take them:* (a) find and fix the
   anonymous-record walk in `lowering` (the named path is linear; the fix is
   presumably a memoized size/align or a type-identity map keyed per
   definition level); (b) meanwhile, if the bound is to mean something,
   derive `TYPE_FRAME_BUDGET` from the *measured* legal-range cost (≈512 ⇒
   ≈1 s worst case) or state the real bound in the doc; (c) add a legal-range
   compile-time regression (depth 200 under a generous wall bound) so the
   window cannot silently widen. I did **not** change the number: it is
   upstream policy for real TUs (which nest records ≤ ~8 deep), and the
   right fix is (a), in a focused change.
8. **Minor, for the record**: the three budgets carry different safety
   margins (expressions ≈2× the measured stack ceiling, blocks ≈3.5×, types
   25× vs the stack but bounded by sema cost). Two of those are justified in
   the source comments; the expression margin is the thinnest and depends on
   per-frame bytes, which vary by build profile. The gate runs on the shipping
   build, so a future profile change would be caught — but a comment pinning
   *why 2× is enough* (or a stack-size assertion) would make it deliberate
   rather than incidental.

### 4.3 Interaction with my patch (checked, not assumed)

* Both gates are registered: `ci_local.sh` line 326 (mine) and 602 (theirs);
  `.github/workflows/ci.yml` lines 774 (theirs) and 783 (mine).
* Their gate's `--lccc` defaults to `target/fastbuild/lccc`, i.e. the ci.yml
  invocation (no `--lccc`) and the ci_local one (`--lccc "$LCCC"`) test the
  same binary. No drift.
* The S11 patch (6 files) applied **cleanly** to the new base despite 211
  upstream files changing: the touched regions are disjoint.
* `scripts/ci_local.sh` and `ci.yml` carry both changes; parity checker PASS
  with 135 commands.

---

## 5. Verification status — read before trusting any number

* **Green, reproducible:** `cargo check --profile fastbuild --all-targets`
  (0 errors, 0 warnings); `cargo fmt --check` clean; `build_lccc_fast.sh`
  `BUILD_EXIT=0`; the 45-row structural gate; the runtime matrix above;
  `check_ci_gate_parity.py` + `check_ci_workflow_shell.py` PASS.
* **`ci_local.sh --fast`: `SUMMARY: 153 passed, 0 failed, 5 skipped`,
  `ALL GATES GREEN`, `CI_EXIT=0`.** Run as
  `env -u LD_PRELOAD ./scripts/ci_local.sh --fast`, never wrapped in `stdbuf`
  — see FOLLOWUP-2026-10-03 §4 R11. The five skipped rows are the declared
  slow class (`cargo-test-debug-assertions`, `regression-corpus-ssa`,
  `benchmark-output-oracle`, `peephole-whitespace-invariance`,
  `regression-corpus-debug-assertions`) — GitHub CI runs those.
  Log: `results/ci-fast-verdict3-*.log` (frozen tree, pass stamp). The
  identical preceding run (`results/ci-fast-verdict2-*.log`) produced the same
  153/0/5 but was stamp-voided because this document was edited *while it was
  running* — the whole reason the stamp exists; treat the stamp as covering
  commit `a579a0a` for the source, with this file's later prose excluded.
* **`cargo-test` (the gate, not a hand-rolled `cargo test`):** 4 038 passed /
  0 failed / 7 ignored (`results/ci-fast-verdict2-*.log`), same numbers as the
  targeted run under the gate's exact recipe.  `rustfmt` PASS, `clippy` PASS
  (`results/clippy-2-<HHMMSS>.log`) — clippy found one real lint in this
  patch's own code (`clippy::derivable_impls` on `BuiltinPolicy`'s manual
  `Default`, which is exactly `#[derive(Default)]`); fixed by deriving.

### 5.1 A test-harness defect this session's own gate run exposed

The first `--fast` run after the A13/A14 commit went **red on `cargo-test`**:

```
test backend::generation::remat_call_arg_tests::memcpy_shaped_call_is_excluded ... FAILED
src/backend/generation.rs:6860: assertion failed: !set.contains(&0)
```

That is not a product bug and not a flake — it is an ordering defect in the
**tests**, and a genuinely interesting one: `build_rematerializable_global_addr_set_for`
consults the same `may_assume_builtin` predicate that decides the const-size
inline expansion, and that predicate is *fail-closed before publication* (the
policy is published at codegen entry, `collect_symbol_sets`).  A unit test that
never publishes a world therefore observes "no module has been compiled yet";
whether it passes depends on whether another test on the shared thread pool had
already published.  My own `libcall_policy` test module was that other test —
and it published the world under a **module-local** lock, i.e. exactly the
anti-pattern `test_support` documents ("module-local locks serialized each
module against itself and nothing else").

Fix (commit `a579a0a`): `libcall_policy` now owns one serialized window
(`PolicyWindow`), installed by `unpublished_policy_for_test` /
`published_policy_for_test` and restored to the pre-driver world on drop —
panic-safe, unlike the hand-restored `reset()` it replaces.  The generation
remat tests take it through `with_module_definitions`, which also made it
possible to pin the A14 rule from the *codegen* side, which nothing did before:

* `memcpy_shaped_call_is_excluded` — builtin world: expansion ⇒ root must stay
  homed.
* `module_defined_memcpy_keeps_the_root_homed` (new) — a TU that defines
  `memcpy` makes the const-size call a real call ⇒ root must be
  rematerializable.
* `memset_exclusion_mirrors_backend_policy` — set/predicate agreement asserted
  in all three worlds the predicate distinguishes (builtin, module-defined,
  CLI withdrawal), not only the default one.

Ordering between tests can no longer change any of these outcomes: every test
that publishes holds the same lock, and every test that reads an expansion
decision takes a window.

### 5.2 Validation-environment repairs (this host, not the repo)

Two harness wipes removed packages that several gates need; the first `--fast`
run therefore failed gates that are *not* about this patch.  Both were repaired
with apt and re-verified:

| symptom | missing package | gate(s) affected | repair |
|---|---|---|---|
| `/usr/include/stdio.h: bits/libc-header-start.h: No such file or directory`, `ld: cannot find Scrt1.o` | `gcc-multilib`, `libc6-dev-i386` | all `-m32` gates: `reassoc-latency` (`-m32` rows), `i686-integer-isa-parity`, `map-i64-two-lane`, `linker-suite`, `regression-corpus-link`, `redundant-test-elimination`, `i686_atomics` rows | `apt-get install -y --no-install-recommends gcc-multilib libc6-dev-i386` |
| `/usr/include/c++/14/exception: bits/c++config.h: No such file or directory` (one row: `linker-suite`/`i386_dso_emit_semantics`) | `g++-multilib` (headers land in `/usr/include/x86_64-linux-gnu/c++/14/32/`) | `linker-suite` | `apt-get install -y --no-install-recommends g++-multilib` |

`scripts/ci_local.sh --only <substring>` re-runs a single gate, which is how
each repair was confirmed in isolation (31 s for `linker-suite` vs ~20 min for
the whole run) before the final full `--fast`.  **Lesson for future sessions:
if `--fast` reports i686/i386 failures, check the multilib packages first —
`dpkg -l gcc-multilib g++-multilib libc6-dev-i386` — before suspecting the
patch.**
* **Not claimed:** no wall-clock performance claim for the *fix* (it buys
  correctness; the const-size expansion gate can only ever remove an
  optimisation in a TU that defines the callee); no claim about non-x86
  targets; the anonymous-record `lowering` cost is measured but **not fixed**.
* Snapshot **S12** (`artifacts/`, ledger) covers the rebuild; the final
  snapshot is taken after `--fast` is green.

---

## 6. Follow-ups, priority order

1. **[perf, from the audit] Anonymous nested record definitions are
   super-quadratic in `lowering`** (§4.2.7) — reproducer
   `audit/nested_records_scale.py`; fix = memoize the anonymous-record
   size/align/identity walk; then re-derive or keep the 2048 budget with an
   honest bound.
2. **[verification] Run the slow `regression-corpus-ssa` gate for A14b's blast
   radius** (any fixture that defines `memcpy`/`memmove`/`memset` loses the
   synthesis it may assert). GitHub CI runs it; the fast gate proves the
   contract but not the corpus.
3. **[coverage] `fortify_fold` asm-level refusal rows for `fwrite`, `fputs`,
   `putchar`, `fputc`** — `puts` is the only one with an asm row today.
4. **[performance, unchanged] `tests/workloads/gzip-1.14/run.py`** — still the
   cheapest end-to-end perf check, still unrun; then `scripts/callgrind_ab.py`
   on a real workload (the PMU-free doctrine).
5. **[E1 remainder]** curate the §3.1(a) reproducer list; note that PR #742
   already landed EDG-mined tests for C23 tag redefinition, empty
   initializers, `typeof` statement-expr cast, DR423 const-return qualifier,
   anon-member designated init, `#embed`, `__has_warning`, and
   `gnu_null_constexpr` — the remaining gaps are `??=`, labels-before-
   declaration, `[[attr]]` before a tag, enum fixed underlying type, `bool` in
   `_Generic`, controlling-expression scope, `_BitInt`.
6. **[driver honesty] `-fno-builtin` now acts on the synthesis paths**;
   recognition-driven transforms on calls the *user wrote* are still
   unprobed (see FOLLOWUP-2026-10-03 §6 item 2).
7. **[E5] const-eval step/depth budget** — unchanged, still unbounded.

---

## 7. Session metrics

| metric | value |
|---|---|
| base | `9f6cfcf` (PR #742), previously `358104c2` |
| upstream files changed between bases | 211 (+3 028 / −112) |
| S11 patch application on the new base | clean (apply-check exit 0) |
| A13/A14 sources lost to a harness wipe and re-implemented | 3 new modules, 6 modified files |
| structural gate | 45 rows, first run green |
| runtime matrix rows | 7 shapes × 3 compilers |
| own-work defects found and fixed in red team | 8 (R1–R8) |
| upstream findings | 1 measured defect with a reproducer (anonymous-record `lowering` cost), 1 minor note (budget margins) |
| measurement discipline | every number in §2/§4 produced by an explicit rc/phase-capturing harness; the first broken harness was thrown away (R8) |
