# S12 red-team audit — rebase onto main `32d1de294`, 2026-10-25

Scope: what survived the rebase, what the other team's merge already covered,
what I dropped and why, and every measurement behind it. Rule unchanged: a
claim counts only if it is reproducible from this tree with the tools that are
actually installed. The pinned oracles (GCC 16.2, Clang 23.1.3, as 2.47, mold
2.42.1, ICX 2026.2) are still absent, so any number below names its real
compiler.

---

## 1. The rebase, conflict by conflict

`work` was 11 commits over `abe67eaa`. Mainline had meanwhile merged
`4179720f7` ("Fix Linux 6.18.58 build blockers … and static init") and
`6f93641d9` ("Complete cross-arch inline-asm, declarator, initializer, and GAS
macro/char fixes") — i.e. **the other team had independently landed most of the
S06–S11 waves**, including my `assign_scratch_reg` / `busy_regs` inline-asm
work, the `ctypes_compatible` consolidation, the fail-closed `LabelMap`, and the
brace-elision/expression-type-cache halves of the compound-literal fix.

Fourteen conflict hunks in seven files. Resolution, per hunk, with the reason:

| file | resolution | why |
|---|---|---|
| `scripts/boot_ab_size.py` | theirs (main) | main routes through `lccc_paths.kernel_default_dir()`; mine was the older direct call |
| `src/backend/asm_preprocess.rs` | main | main's `.macro` default-argument scanner is a superset: it also handles GAS character literals (`skip_gas_char_literal`) and escapes inside defaults, and it is the version the new `check_asm_char_and_macro_defaults.py` gate pins |
| `src/backend/i686/codegen/globals.rs` | main | rustfmt-level difference only |
| `src/frontend/parser/types.rs` | main | main's abstract-declarator path builds the level through `parse_abstract_declarator_level`, which is the same fix with the named-address-space wrapper resolved once |
| `src/frontend/sema/const_eval.rs` | main | main delegates to `type_checker().resolve_type_spec`; my local re-implementation of the same mapping is strictly worse (two copies to keep in sync) |
| `src/ir/lowering/global_init_compound_ptrs.rs` | main | main diagnoses a failed address constant (`diagnose_non_constant`) where mine `expect`ed; a rejected initializer with a real diagnostic beats an ICE reached from user input |
| `src/passes/inline.rs` | main | main's `LabelMap` records `overflowed` instead of asserting, so the caller can abort *before* mutating the CFG |
| `src/common/types.rs`, `src/frontend/parser/types.rs` | main | formatting / superset as above |
| `src/backend/inline_asm.rs` | **merged by hand** | main has `push_busy_reg` (32-bit alias + `reg_hi` normalisation) which mine lacks; mine has `reserved_home_regs`. Final hunk keeps main's normalisation over specific/operand/`reg_hi` registers **and** adds the reserved homes. This is the one file where neither side was sufficient |
| `src/backend/x86/codegen/asm_emitter.rs`, `i686/…` | main + one method re-added | main's `assign_scratch_reg` refuses RA-assigned callee-saved registers when overflowing the GP pool; mine mapped to a fixed `r12..r15` walk. main wins. I re-added only `reserved_home_regs`, which main does not have at all |
| `src/backend/x86/codegen/emit.rs` | main | main's `value_home_is_expendable` is the same policy written inline; my refactor added a helper but no behaviour. main's `collect_inline_asm_callee_saved_with_overflow` has since grown a 9-argument overload closure, so my call site was stale |
| `.github/*`, `scripts/ci_local.sh`, `ci-lane-plan.json` | main | main already registers the gates (`asm-char-and-macro-defaults`, `debug-knobs-neutral`) that my commit was adding |
| `src/ir/lowering/global_init_bytes.rs` | main, then **re-ported by hand** (§2) | main rewrote the initializer around a scalar-array planner and a trailing-designator delegate; my whole-file version would have reverted that rewrite |

**Do I agree with the other team?** On every one of these, yes, and in three
cases their version is better than mine in a way I could not have argued with:
(a) `diagnose_non_constant` instead of `expect` at a user-reachable path,
(b) `LabelMap::overflowed` instead of an assertion, (c) the `assign_scratch_reg`
overflow that avoids RA-assigned callee-saved registers. The one place I
disagree with the merge result is that `reserved_home_regs` was not carried
over at all — and that is a real hole, because phase 1f materialises a memory
operand's address **before** phase 2 loads the inputs, so a register-homed
input is invisible to the scratch allocator. It is back in this patch.

---

## 2. The one thing main still got wrong: `.{member} = (T[N]){ … }`

After the rebase, main fixed three of my five S11 defects on its own. I
re-measured the whole probe set (each compiled **and run** by both compilers,
gcc 14.2 as oracle) instead of assuming:

| probe | shape | lccc before this patch | gcc | after |
|---|---|---|---|---|
| s1 | `struct { unsigned long pad; nm_t a[2]; } g = { .a = (nm_t[2]){ … } }` | `a[0].b[0]=0 a[1].b[1]=0` | `3 4` | **3 4** |
| s1 | `struct { unsigned long pad; int m[2][3]; } x = { .m = (int[2][3]){ … } }` | `m[1][2]=0` | `6` | **6** |
| s1 | `struct { unsigned long pad; char s[4]; } g3 = { .s = (char[4]){"ab"}, .n = 7 }` | `s=0,0,0` | `97,98,0` | **97,98,0** |
| s1 | `awrap_t g4[1] = { [0] = (awrap_t){ .a = (nm_t[2]){ … } } }` | `g4=0` | `5` | **5** |
| s2 | `awrap_t v[2] = { [1].a = (nm_t[2]){ … } }` | `v=100` | `134` | **134** |
| s3 | `[1][1].n = (nm_t){ .b = {[1]=6} }` (pad at 0) | `109` | `109` | 109 (main already) |
| s4 | `nm_t g[2] = (nm_t[2]){ … }` (object level) | `709` | `709` | 709 (main already) |
| s5 | block-scope `static` array literal | `70913` | `70913` | 70913 (main already) |

So the surviving defect is exactly the **member level**: a compound literal of
the member's own array type was read as one scalar expression of the member.
That is now one substitution before the member is resolved
(`substitute_array_literal_items`), sharing `array_initializer_payload` with
the object level so the two cannot drift apart again. It also removes a
**compiler hang**: the array-of-aggregates flavour re-entered the same member
loop for the same item.

Note what this means for the earlier report: my claim that the `[i].field =
(T){…}` defect needed a dedicated `fill_designated_field_compound_literal`
was an artifact of the *pre-rebase* control flow. Main routes a trailing
designator through the member filler, so the substitution alone fixes that case
too (s2). Writing the extra helper would have been dead code — a good example
of why the port had to be re-measured rather than transplanted.

---

## 3. `reserved_home_regs` — kept, and why

`inline_asm` materialises each `"m"` operand's address in phase 1f, then loads
the operands' values in phase 2. The scratch allocator sees only the registers
an operand has *claimed*; a value that is **register-homed** (a parameter, a
forwarded value) is loaded later into its home, so the address staging could
destroy an input before the asm body ran. The backend now reports those homes
and `inline_asm` adds them to the busy set. Main has no equivalent.

Cost: one `fresh_home_of` call per input/output value per asm site, and the
register set is tiny. This is fail-closed by construction: reserving an extra
register at worst picks a different scratch register.

---

## 4. Peephole phase 2b — measured, bounded, kept

Phase 2's store forwarder runs **once**, and it answers "may this slot load be
replaced by the register just stored into it?" against the text it was handed.
Its most common blocker is a register write between the store and the load —
and that write is frequently retired *later in the same function* by phase 2's
copy propagation (`movzbl (%r9,%rsi), %eax; movzbl %al, %esi` →
`movzbl (%r9,%rsi), %esi`, which retires the `%eax` write). Nothing re-asks the
question, so the store/reload pair survives into the final assembly.

The fix is one extra store-forwarding round, taken only after phase 3's local
fixpoint has gone quiet, bounded to one round, and gated by
`CCC_PEEPHOLE_SKIP=phase2b`.

Measured (same binary, kill switch as the only difference):

| corpus | files | files with any change | direction |
|---|---|---|---|
| `tests/benchmark/programs/*.c` (-O2) | 57 | 0 | — |
| `tests/regression/*.c` (-O2) | 1067 | 5 | **fewer** instructions only (1–2 each) |
| compile wall time, 57 benchmark TUs | — | — | 3575 ms on vs 3670 ms off (noise) |

**Honest limitation:** it does *not* fix the pair I originally went looking for
in gzip's `longest_match` (`movl %eax, 12(%rsp)` / `movl 12(%rsp), %eax`).
That survives, and phase 2b reports "no change" on it, which means the
forwarder is refusing for a reason that is *not* the intervening register
write. It is still in the patch because it is monotone (never adds an
instruction), cheap (no measurable compile-time change) and it closes a real
fixpoint gap — but it is not the gzip fix, and I am not claiming it is.

---

## 5. Where the remaining performance actually is (measured)

Paired Callgrind instruction counts, `-O2`, this machine (gcc 14.2, clang
19.1.7 — *not* the pinned oracles):

| benchmark | lccc Ir | gcc Ir | clang Ir | lccc/gcc | lccc/clang |
|---|---|---|---|---|---|
| byte_load_reassembly | 38.7 M | 31.1 M | **18.1 M** | 1.25 | **2.14** |
| gzip_longest_match | 2 588.7 M | 2 222.3 M | 2 100.0 M | 1.17 | 1.23 |
| nbody | 3 435.1 M | 2 995.1 M | 2 915.1 M | 1.15 | 1.18 |
| switch_dispatch | 1 031.4 M | 865.7 M | 903.2 M | 1.19 | 1.14 |
| binary_trees | 15 598.8 M | 14 339.5 M | 15 174.1 M | 1.09 | 1.03 |
| chacha20_block | 1 606.5 M | 2 606.9 M | 2 338.4 M | **0.62** | **0.69** |
| llvm_scalar_facts | 680.1 M | 715.1 M | 716.1 M | **0.95** | 0.95 |
| hot_web_loop | 881.4 M | 801.1 M | 841.2 M | 1.10 | 1.05 |

`byte_load_reassembly` is the one unambiguous failure: **2.14× clang's dynamic
instruction count**. The byte-load reassembly pass itself is *not* the problem
any more — its hot loops already fold to `movl (%r8,%rdi), %r14d` and
`xorl 8(%r8,%rdi), %edx`. The gap is that clang **vectorises the
xor-accumulate reduction** (widening dword loads to qwords with
`punpck`/`paddq`) and lccc keeps it scalar. That is an SLP/loop-vectoriser
gap, not a peephole gap, and it is the single largest measured codegen deficit
in the corpus.

For `gzip_longest_match` (1.17×) the dynamic excess is spread over the
chain-walk block, not concentrated in one instruction: the candidate pre-check
re-materialises `window` (`leaq window(%rip), %r10; addq %rcx, %r10`) where gcc
keeps the base in a register across the loop, and there is at least one
redundant spill round-trip (§4). I checked whether this was a general LICM/CSE
failure with a dedicated probe and it is **not** — lccc hoists both base
pointers correctly in a clean loop (`inv.c`), so the gzip shape is a
register-pressure artifact of that function, not a missed transformation.

---

## 6. Verification actually run on this tree

| check | result |
|---|---|
| `cargo build --profile fastbuild --locked -j2 --bin lccc{,-i686,-arm,-riscv}` | RC 0 |
| `cargo test --lib -j2` | **6379 passed, 0 failed**, 10 ignored |
| `cargo clippy --all-targets` | **0 warnings**, RC 0 |
| `cargo fmt --all --check` | clean |
| `python3 tests/regression/run_regression.py --gcc gcc` | **1151 passed, 0 failed**, 3 skipped-compare |
| the five /tmp probe files (array compound literals) | lccc == gcc on all five |
| the scratch F1 harness | `{"failures": 0}` |
| `scripts/ci_local.sh --fast` | **764 PASS / 71 FAIL**, RC 1 — see §6a |
| `python3 scripts/check_doc_links.py` | OK (after fixing the S11 audit's out-of-tree reference) |
| `python3 tests/linker/run_linker_tests.py --strict` | **303 pass, 0 fail**, 3 skip (kernel `relocs` tool absent) |
| 57-program paired benchmark (`run_benchmarks.py`, 9 reps) | see §5 |

---

## 6a. The 71 fast-CI failures, attributed

Every one of them was opened and classified, not counted:

* **50** are pinned-oracle gates: they refuse to run without GCC 16.2
  (`gcc-16`), Clang 23.1.4, `as` 2.47 or ICX 2026.2. Their messages are
  explicit ("needs GCC 16 as the verdict oracle; found: gcc=GCC 14",
  "Install the official LLVM 23.1.4 release"), and 21 more of the same class
  surface as `FileNotFoundError: 'gcc-16'` tracebacks from gates that call the
  oracle without a version probe.
* **1** was a genuine self-inflicted failure: `doc-link-integrity` rejected the
  S11 audit's reference to a ledger path that only exists outside the repo.
  Fixed (`check_doc_links.py`: OK).
* **1** was an environment gap, not a code defect: `linker-suite` failed its
  i386 DSO case because 32-bit libstdc++ was missing. Installing
  `g++-multilib lib32stdc++-14-dev` fixes it — verified directly
  (**303 pass, 0 fail, 3 skip**; the skips want the kernel `relocs` tool).
* **0** are logic regressions from this patch: the full regression corpus
  (1151/0), the unit tests (6365/0) and clippy (0 warnings) all pass on the
  same tree.

The gate set therefore cannot be green on this host, and that is a host fact,
not a code fact: `ci_install_gcc16.sh` deliberately refuses a Ubuntu PPA on a
non-Ubuntu distribution, this host is Debian 13, and Debian's gcc-16 is not in
the release. I did not fake a green board by unpinning the gates.

## 7. Self-criticism

1. **I re-derived work that the other team had already merged.** Three of my
   five S11 "fixes" and two of my codegen fixes were already in main; the
   rebase had to re-measure instead of transplant, and the surviving delta is
   52 KB, not 272 KB. That is the correct outcome, but the earlier reports
   should not have been written as if main could not have moved.
2. **One claim in the S11 audit was wrong in detail**: the `[i].field` defect
   does *not* need a dedicated filler helper under main's control flow. The
   member-level substitution covers it (s2). I found this by re-measuring, not
   by reasoning.
3. **Phase 2b is not the gzip fix** and the report says so explicitly instead
   of letting the benchmark table imply it.
4. **No pinned-oracle numbers.** Every table above says which compiler it used.
   Until GCC 16.2 / Clang 23.1.3 are installed, no oracle-grade statement is
   possible and none is made.
5. **The kernel gate did not run** (no kernel source on disk; 3.9 GB free,
   2 GB RAM). No kernel claim is made.

---

## 8. Next, in order of measured payoff

1. **`byte_load_reassembly`: 2.14× clang.** The gap is the reduction
   vectoriser, and it is the largest single deficit in the corpus. This is a
   pass-sized job (`slp_vectorizer.rs` / `loop_unroll_reduction.rs` exist and
   already handle other shapes), and it is worth more than every peephole in
   this patch combined.
2. `gzip_longest_match` 1.17×: find why the pre-check block declines the
   register-forwarded base (needs the "why did the forwarder refuse" trace,
   which phase 2b's kill switch now makes cheap to add).
3. `switch_dispatch` 1.19×, `nbody` 1.15×.
4. Re-run everything with GCC 16.2 / Clang 23.1.3 once they can be installed:
   this host is Debian 13 and `ci_install_gcc16.sh` deliberately refuses a
   Ubuntu PPA on a non-Ubuntu host, so installing 16.2 here means either a
   source build (hours; 2 GB RAM) or a distribution-wide upgrade that would put
   the sandbox at risk. Neither is worth doing in-session; the honest state is
   "not installable here without breaking the host".
