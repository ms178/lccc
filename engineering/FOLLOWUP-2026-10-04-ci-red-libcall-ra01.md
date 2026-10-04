# Session record — 2026-10-04: red Test Suite root-caused, audit adjudicated, RA-01 measured

Scope of this record: the work merged on top of `ms178/lccc` main `7aa88838`
(PR #753 baseline), namely

1. the root cause and fix of the **red `Test Suite` check** (run `37196917342`,
   job `111420606509`, step *Run tests* = `cargo test --profile fastbuild
   --all-targets --locked -j 2`),
2. the adjudication of the external review's findings — what was accepted, what
   was rejected, and on what evidence,
3. the new gates/tooling this session added,
4. the RA-01 performance measurement and the precise, measured statement of what
   remains open.

Everything below is reproducible from the tree; commands are given inline.

---

## 1. Root cause of the red `Test Suite`

**Symptom.** The job failed in `Run tests`; every later step (`Run tests
(debug-assertions on)`, the regression corpus, the gates) was skipped. Clippy and
the Compiler-Explorer oracle-delta job were green, so the red check was
unambiguously `cargo test --all-targets`, not a gate.

**Cause.** `src/backend/libcall_policy.rs::may_assume_builtin` was **fail-open
before publication**:

```rust
!live.policy.withdrawn(name) && !live.inventory.blocks_library_assumption(name)
```

`Live::default()` is an empty world, and an empty world reads exactly like a
published world that withdrew nothing and defines nothing: `withdrawn()` is
false, `blocks_library_assumption()` is false, so the predicate answered
`true` — "the library contract holds, inline/gate the libcall" — for a
compilation the driver had not published anything about. The module's own unit
test (`unpublished_state_fails_closed`) asserted the opposite, which is why the
suite failed at exactly that test.

This is not a cosmetic test failure: the predicate is the single choke point
that decides whether the compiler may bake library semantics (memcpy/memset
expansion, `__*_chk` fortified arms, `puts`→`fputs`-style rewrites, the
self-recursion gate) into generated code. Answering `true` in an empty world is
the fail-open direction of a security-relevant decision.

**Fix (stronger than the audit's proposal).** Both halves of the published state
are now required, and each is tracked by its own flag:

* `policy_published` — set by `set()` (the driver's CLI policy);
* `module_published` — set by `set_module_state()` (the module's symbol
  inventory).

Either one alone is unsound: a published policy with no published inventory
cannot see a TU that defines the callee (the A14b bug), and a published
inventory with no published policy cannot see a `-fno-builtin` that withdrew it.
Tests cover both half-published states (`policy_only_for_test`,
`module_only_for_test`) and the restore window, not just the empty one.

## 2. Adjudication of the external review

Reviewed against the code and against measurements, not against the review's
authority. Verdicts:

| finding | verdict | basis |
| --- | --- | --- |
| **B1** `may_assume_builtin` fail-open + a test that contradicts it | **Agreed — and fixed harder than proposed** | The root cause of the red CI check (§1). The review proposed making the *policy* fail closed; the tree now requires **both** publication halves, with half-published-state and window-restore tests. A single-flag fix would leave the A14b blindness (a module that defines the callee) reachable. |
| **H1** `__STDC_HOSTED__` not covered behaviorally | **Agreed, fixed** | The pipeline was already correct; what was missing was *behavioural* coverage. The regression gate now asserts `__STDC_HOSTED__ == 1/0` for `-O2`, `-ffreestanding`, `-ffreestanding -fhosted`, and `-ffreestanding -D__STDC_HOSTED__=1`. |
| **H2** nothing prevents the compiler from *newly* exploiting UB | **Agreed, fixed** | `scripts/check_no_ub_exploitation.py` — a narrow (9-pattern), comment-aware tripwire over the optimizer's source, self-tested, wired into **both** CI mirrors (`scripts/ci_local.sh`, `.github/workflows/ci.yml`). It is the machine-checked precondition for the tolerated tier: `-fno-strict-aliasing`/`-fwrapv`/`-fno-strict-overflow` are only safe to ignore while there is no type-based alias analysis and no overflow poison. Today's scan: 0 hits outside comments. |
| **H3** the `__*_chk` doc claim ("both reference compilers keep inlining under every withdrawal spelling") is false | **Agreed on the fact, rejected both proposed fixes — the claim, not the code, was wrong** | Measured on Compiler Explorer (`-O2`, GCC 16.2 / Clang 23.1 / ICX): a **TU-defined** `__memset_chk`/`__memcpy_chk` is expanded by **all three** (`movabsq $506381209866536711` on the fill). Gating the arms on `may_assume_builtin` (option a) would therefore diverge from every oracle; dropping the arms (option b) would lose a fill that removes a symbol a freestanding link may not have. The false comment is replaced by the measured table, and the one genuine divergence is recorded rather than argued away: with `-D_FORTIFY_SOURCE=2 -fno-builtin` **GCC alone** keeps the call while Clang/ICX/LCCC expand. |
| **M1a/M1b/M2, F8** | **Agreed, fixed** | i686 array-bound narrowing validated *before* conversion (`usize::try_from`, `sema/const_eval.rs`) so a negative count cannot become `usize::MAX` and then a plausible-looking saturated size; `saturating_mul` for the initializer-list array size (`ir/lowering/global_init.rs`); the `array_bound_elems(_, 0)` contract documented at the definition site (callers that only need the *shape* of a declaration skip the byte test by construction; the declaration path that forms the array type always checks the byte size with the real element type); the `cli.rs` tolerated-tier comment now names the invariant its safety depends on. |
| **M3** `-nodefaultlibs`/`-nostartfiles` is an unrelated drive-by | **Disagreed** | It arrived with `44ed9b0d` ("S21 content") as part of the option-contract workstream and is documented with measurements in `src/backend/common.rs`. It is not an unrelated change and is not split out. |
| **M4/F7** the sparse-`.bss` equivalence proof is out-of-tree and unreproducible | **Agreed, fixed in-tree** | `tests/regression/check_sparse_bss_equivalence.py` replaces the out-of-tree script: section A re-proves byte/layout equivalence against a `--reference` build, section B proves the 4 GiB `.bss` case stays inside a bounded RSS, section C checks `.bss` fidelity against the `nm` symbol sum. |

## 3. What this session changed (files)

* `src/backend/libcall_policy.rs` — fail-closed on **both** halves + tests (§1).
* `src/backend/x86/codegen/globals.rs`, `src/backend/x86/codegen/memory.rs` —
  remat LEA: `sym + register` in **one** instruction (see §5).
* `tests/oracle/delta_corpus.json` — explicit `-fno-pie`, with the reason in
  `flags_note` recorded next to the flags (see §4).
* `tests/benchmark/programs/ra01_global_match.c` — new dynamic RA-01 benchmark
  (Callgrind A/B, correctness-gated against GCC at `-O0..-O3`).
* `scripts/check_no_ub_exploitation.py` (+ both CI mirrors), H1/H3 gate rows in
  `tests/regression/check_libcall_synthesis_no_selfcall.sh`,
  `tests/regression/check_sparse_bss_equivalence.py`, and the small
  correctness/robustness fixes listed under M1a/M1b/M2/F8.

## 4. What the first acceptance run caught (all fixed)

`cargo test --profile fastbuild --all-targets` — the CI step verbatim — ran for
the first time on this tree in this session (the earlier attempt was killed
mid-build, so its verdict never existed). It reported **4091 passed, 8 failed**,
and the eight split into three distinct causes. None of them is papered over;
each is fixed at its root:

| failure | cause | fix |
| --- | --- | --- |
| 5 × `libcall_policy::tests::*` | **mine.** The pre-existing helper `published_policy_for_test` filled in `policy` and `inventory` but never set the two publication flags — a test world that models nothing the compiler can reach, which the hardened predicate now (correctly) refuses. | the helper sets both flags, exactly as `set()` + `set_module_state()` publish them in production. The half-world windows (`policy_only_for_test`, `module_only_for_test`) keep testing the fail-closed direction. |
| `remat_call_arg_tests::memcpy_shaped_call_is_excluded` | **mine, same root.** The test builds its world through the same helper; with the predicate failing closed, `inline_memcpy_len("memcpy", …)` returned `None`, so the memcpy-shaped call stopped being excluded from the remat set. | fixed by the same helper correction; the assertion is untouched. |
| `cli_tests::debug_selector_boundary_matches_gnu` | **pre-existing.** `-gz=bogus` is documented (driver header table) and tested as a hard error — the selector grammar is closed and GCC 16.2 / Clang 23.1 reject it — but the arm warned for every `-gz=…` spelling. | unknown values are refused with the option named; the five values GCC does accept (`none`, `zlib`, `zlib-gnu`, `zlib-gabi`, `zstd`) keep the "not supported here, emitting uncompressed" warning. |
| `cli_tests::meaning_changing_requests_are_refused_not_ignored` | **pre-existing, and the most interesting one.** `-funsigned-char` is listed as a data-model request that must be refused, but upstream `9df694e2` *implements* it (`set_char_unsigned`), so the refusal entry was unreachable dead code and the doc row was wrong. Measuring the implementation showed it was half-wired: `char` flipped to unsigned while `__CHAR_UNSIGNED__` was defined only in the AArch64 target block (RISC-V, whose `char` is unsigned too, had no macro at all) — and glibc's `<limits.h>` derives `CHAR_MIN`/`CHAR_MAX` from that macro, so on x86-64 `-funsigned-char` gave a `char` that compares as unsigned while `CHAR_MIN` still read **-128** and `#ifdef __CHAR_UNSIGNED__` was **false**. GCC 14.2 on the same input defines the macro and prints `CHAR_MIN=0`, which is the oracle. | the macro is now derived from `char_is_unsigned()` inside the builtin macro table (one source of truth for the type system, the preprocessor and `<limits.h>`); the AArch64-only definition is gone; the stale refusal entry and doc row are gone; and `tests/regression/check_char_signedness.sh` pins type + macro + limits behaviourally against GCC, including both cross-target defaults via the binary-name target selector (wired into both CI mirrors; parity check now reports 141 commands). |

The lesson recorded for the next session: the eight failures were *not* eight
bugs but three, and only one of the three was visible in the unit tests' own
terms — the char-signedness defect needed a compile-and-run row to see, which is
exactly why §5's behavioural gate exists.

## 5. The oracle corpus was comparing different code models

`tests/oracle/delta_corpus.json` compiled every competitor with
`-O2 -march=x86-64-v3` and left the **mode** to each compiler's default. Those
defaults do not agree:

| | lccc | CE GCC 16.2 / Clang 23.1 / ICX |
| --- | --- | --- |
| default for `-O2` | **PIE** (`opts.pic \|\| opts.pie`) | **non-PIE** |

Comparing the two modes is comparing the driver's hardening default against the
oracle's, so the corpus now states the mode explicitly. Both mode-matched
measurements, produced with the *same* pipeline (`scripts/oracle_delta_gate.py
--corpus` differs only in the flag), RA-01 kernel:

| mode | LCCC | GCC 16.2 | Clang 23.1 | ICX | LCCC / best |
| --- | ---: | ---: | ---: | ---: | ---: |
| non-PIE (**the corpus flag**) | 86 | 55 | 106 | 56 | **1.56** |
| PIE | 88 | 66 | 106 | 75 | **1.33** |

Two facts come out of this, and both are recorded rather than argued:

* PIE costs **GCC 11 instructions** on this kernel and LCCC 2. LCCC's PIE path
  (the PF-07 symbol-base promotion, register-homed bases + SIB) is *relatively*
  stronger than GCC's; the mode-matched ratio is better for LCCC in PIE mode.
* The absolute `sym(,%idx,scale)` SIB form does not exist under PIE (there is no
  RIP+index encoding), which is precisely why the remat LEA of §6 becomes one
  instruction only in the non-PIE mode the corpus asks for.

The corpus keeps `-fno-pie` deliberately: it is the mode in which LCCC's ratio
is *worse* (1.56 vs 1.33), and a gate must guard the harder comparison, not the
flattering one. The PIE numbers stay in this record.

## 6. RA-01: `sym + register` in one instruction, measured

`emit_rematerialized_global_addr_impl` rebuilt an audited address derivation as
`leaq sym(%rip), %dst` **+** `addq %off, %dst` — two instructions, and the
`addq` clobbered flags. RIP-relative addressing cannot carry an index register,
so the single-instruction `symbol + register` form is the absolute disp32 SIB
encoding, `leaq sym(, %off, 1), %dst`, legal in exactly the modes the jump-table
emitter already relies on (`code_model_kernel || !pic_mode`, the same predicate,
repeated verbatim so both sites move together). Composed `sym+off` names and
`%rsp` as an index fall back to the two-instruction sequence, and the operand
string is built by the existing `sib_mem64_sym` formatter so the dialect has one
source of truth.

Measured effect (static, same pipeline: `scripts/oracle_delta_gate.py --only`):

| kernel | PIE (default) | non-PIE, with the change | non-PIE, without (code path) |
| --- | ---: | ---: | ---: |
| `gzip_crc32_update_no_xor` | 14 | **13** | 14 (`leaq sym(%rip),%dst` + `addq`) |
| `ra01_global_match_probe` | 88 | **86** | 88 (two such sites) |

Attribution, stated carefully: under PIE the absolute-SIB form is not available
at all, so both the mode and the change are needed for the 2-instruction win;
what the asm proves is that the two 2-instruction sequences are gone and the
`(, %idx, 1)` operands are present (`window(, %r8, 1)` in the non-PIE listing of
§6). The "without" column is read off the removed code path, not from a rebuilt
older compiler.

`scripts/oracle_delta_gate.py`, all five corpus entries, after the change:

| entry | lccc | best oracle | ratio |
| --- | ---: | ---: | ---: |
| `ra01_global_match_probe` | 86 | 55 | 1.56 |
| `zlib_ng_adler32_c` | 269 | 132 | 2.04 |
| `gzip_crc32_loop` | **13** | 15 | **0.87** |
| `expat_name_length` | 70 | 67 | 1.04 |
| `expat_scan_document` | 81 | 50 | 1.62 |

Dynamic measurement (`scripts/callgrind_ab.py`, Valgrind 3.24.0, pinned cache
geometry, correctness-gated by identical stdout, lccc vs local GCC 14.2,
`-O2 -march=x86-64-v3 -fno-pie -no-pie`):

| benchmark | Ir GCC | Ir lccc | ratio | note |
| --- | ---: | ---: | ---: | --- |
| `ra01_global_match` | 392,446,998 | 608,788,017 | **1.551** | the open gap |
| `gzip_crc32` | 545,379,602 | 545,380,330 | **1.000** | parity |
| `zlib_ng_adler32` | 356,926,299 | 381,694,726 | 1.069 | |
| `expat_xml_scan` | 586,720,183 | 691,480,278 | 1.179 | lccc retires 12 % **fewer** branch mispredicts (5.78 M vs 6.62 M) |

The static and dynamic tables disagree in sign on `gzip_crc32` (0.87 vs 1.00)
and in magnitude on `zlib_ng_adler32` (2.04 vs 1.07) — which is the documented
reason `scripts/loop_density_oracle.py` exists: for loop kernels the static count
is a proxy, and the honest question is instructions retired per unit of work.

## 7. The open RA-01 gap, stated precisely

The gap is **not** the missing one-instruction LEA (that is now in). It is that
LCCC materialises *derived pointers* while GCC keeps the symbol as the memory
operand's displacement and the offsets in registers. Side by side, hot loop of
`global_match_probe` (`-O2 -march=x86-64-v3 -fno-pie`):

```asm
# GCC 16.2 (55 insns, 1 push)          # LCCC 86 insns, 6 pushes
.L10:                                  .LBB1:
    movl %eax, %edx                        movl %edi, %r8d
    cmpb %r9b, window(%rdx,%rcx)           leaq window(, %r8), %r11
    jne  .L4                               movl %r12d, %r8d
                                           movzbl (%r11,%r8), %eax
                                           cmpl %edx, %eax
                                           je   .LBB6
```

GCC never materialises `window + cur_match`: the SIB operand *is* the address.
LCCC's IR for the same line is exactly the shape that prevents this
(`CCC_DUMP_IR=1`, block `.LBB1`):

```
Cast    { dest: 57, src: 163 → I64 }                 ; cur_match
BinOp   { dest: 58, op: Add, lhs: 6 (window), rhs: 57, ty: I64 }   ; match = window + cur_match
GetElementPtr { dest: 62, base: 58, offset: 60 }     ; match[best]
Load    { dest: 63, ptr: 62, ty: U8 }
```

`Value(58)` has exactly one use shape — GEP bases (`62`, and `74` in the
`best-1` arm) — so it is a candidate for the fold the tree already implements for
*constant* symbol displacements and for `GEP(root, index)`: the missing piece is
that the fold's map (`build_global_addr_map`) only composes constant offsets, so
`sym + register` never becomes a foldable base, and the value keeps a register
home.

**Next step (specified, not hand-waved).** Extend the existing fold machinery —
do not add a parallel one — with a register-offset symbol map built beside
`build_global_addr_map`:

1. map `Add(Value(root∈map), Value(off))` results (single definition, 8-byte
   integer/pointer type) to `(sym, off)`;
2. accept such a base in `can_indexed_addr_fold` under the same gating the
   symbol arm already applies (`supports_indexed_sym_base`, `rip_rel_blocked`,
   the PIC use-count rule);
3. emit `sym(%off,%idx,scale)` (and `sym(,%off,1)` for a direct pointer use)
   through the same `sib_mem64*` formatters;
4. mark the value dead through the **existing** "all uses are folded-GEP bases"
   logic in `generate_function` (which already inserts such bases into
   `dead_global_addrs`), so the `Add` emits nothing;
5. keep the two safety nets the machinery already has: the `value_to_reg_inner`
   home-less rebuild and `rematerialize_skipped_indexed`.

Expected effect on the loop above: the `leaq` disappears and the two `movl`
shuffles lose their cause (one fewer live value), i.e. the 5-instruction
prologue of the current loop body becomes 3 — the GCC shape — with the flag
dance gone. Deliberately **not** attempted in the same commit as the CI-red fix:
it changes the register-allocation/fold contract that twelve documented past
regressions came from, it needs its own full battery, and shipping it
unfinished would trade a green tree for a faster one.

## 8. Verification run in this session

| check | result |
| --- | --- |
| `tests/regression/check_libcall_synthesis_no_selfcall.sh` | all checks passed (incl. new `__STDC_HOSTED__` rows, `chk` rows) |
| `scripts/check_array_bound_contract.py` | 15 contract cases + 4 i686 hold |
| `tests/regression/check_sparse_bss_nobits.sh` | 21 passed, 0 skipped |
| `scripts/check_no_ub_exploitation.py` | self-test PASS; scan clean (0 hits) |
| `scripts/check_ci_gate_parity.py` | PASS — 140 commands |
| `tests/correctness/run_correctness.py` | 60 passed, 0 failed |
| `tests/regression/check_char_signedness.sh` (new) | 11 passed, 0 failed — `-funsigned-char` moves type + macro + limits together, matching GCC 14.2; AArch64/RISC-V defaults define the macro, `-fsigned-char` removes it |
| `cargo test --profile fastbuild --lib` | **4099 passed, 0 failed, 7 ignored** (was 4091/8 before §4's fixes; the +8 are exactly the fixed tests) |
| `tests/regression/check_sparse_bss_equivalence.py` | sections B and C pass (16 GB `.bss` assembles at 0-12 MiB peak RSS); **section A SKIPped** — it needs a build of the pre-change compiler at `01924053`, which this session did not produce |
| `scripts/check_benchmark_outputs.sh ra01` | PASS 4 / FAIL 0 (new benchmark program, `-O0..-O3`, vs GCC) |
| `scripts/oracle_delta_gate.py` | no regressions vs baseline; table in §5 |
| `scripts/callgrind_ab.py` | 4/4 comparable, table in §5 |
| `scripts/ci_local.sh --fast` | the acceptance run (§8) |

## 9. Acceptance

`CI_LOCAL_JOBS=2 scripts/ci_local.sh --fast` — its test step is the CI step
verbatim (`cargo test --profile fastbuild --all-targets --locked -j 2`,
`.github/workflows/ci.yml:87`), so a green `--fast` run is the local proof that
the red job goes green. Log: `/home/user/results/ci-fast-after.log`.

## 10. Second session, same day: the sandbox was recycled — recovery and re-measurement

The execution sandbox was wiped and the workspace came back from an older
point: a 431-file partial tree, no `.git`, no `target/`, no toolchain, no
swap (the WORKSPACE snapshot does not capture git objects or build dirs).  What
made full recovery possible, in order of value:

1. **`refs/pull/753/head` was still `958f09ab`** on the remote.  A fresh clone
   plus `git fetch origin '+refs/pull/753/head:refs/remotes/origin/pr753'`
   restored the entire committed series.  It was rebased onto the now-live main
   `c319eab9` (PR #754, "assembler: checked AArch64 offsets, shared pair
   classification, fmov/#fbits/by-element fixes") with **no conflicts** →
   `1e884114`.
2. **A hash-diff of the surviving tree against PR753 found exactly four
   differing files**: NEW `engineering/FOLLOWUP-2026-10-04-ci-red-libcall-ra01.md`
   and `scripts/check_no_ub_exploitation.py`; MODIFIED
   `docs/builtin-libcall-policy.md` (+9 lines) and `scripts/ci_local.sh`
   (+16 lines: the two gate registrations).  All four were pure *additions* vs
   PR753, which is what made copying them exact rather than approximate — after
   `scripts/ci_local.sh` was reverted and re-applied as two surgical hunks
   (a wholesale copy of that file would have **deleted** the
   `aarch64-oracle-selftest` gate: the salvaged copy predated it).

Everything else in the uncommitted layer — the fail-closed predicate and its
tests, the `__CHAR_UNSIGNED__` derivation, the `-gz=` grammar, both new gates,
the benchmark program, the corpus flags — was **re-derived from the §1/§4
specifications and re-measured**, not copied from anywhere.  §12 records the
numbers; §11 is the audit of that reconstruction.

## 11. Red-team audit of my own reconstruction

Ordered by severity, each with the evidence that settles it.

**A. A "measured" claim was false as written.**  The `-gz=` comment asserted
that `none`, `zlib`, `zlib-gnu`, `zlib-gabi`, `zstd` are "what GCC 16.2
documents and what the local GCC 14.2 accepts".  Re-measured on the local GCC
14.2: it accepts the first four and **rejects `zlib-gabi`** (`gcc: error:
unrecognized argument in option '-gz=zlib-gabi'`) — that spelling is Clang's.
The rule the code implements is now stated as what it is: the tolerated set is
the **union of the reference compilers' grammars**, so a spelling either
reference accepts is warned about (never refused — refusing it would break a
build system that works under Clang), while a name neither accepts is a typo
and a hard error.  This is precisely the failure mode the standing instruction
forbids ("no guesswork"), and it was caught only because the claim was
re-measured in the tree it ships in.

**B. The carried-open "absolute-SIB drops the `addq` flag clobber" question is
resolved: no hazard exists.**  The fast path replaced `leaq sym(%rip),%r` +
`addq idx,%r` (which clobbers EFLAGS) with a single
`leaq sym(,%idx,scale),%r`.  LEA never writes flags, so the change can only
*lengthen* flag lifetimes — it can never invalidate a consumer.  The flags in
this backend are consumed only by the fused-Cmp and `cmp_replay` mechanisms,
and both are guarded by adjacency rules computed *against* clobbering emitters
(`emit.rs`: "no flag-clobbering instruction between them"; `cmp_replay`
re-emits the compare at the consumer precisely because flags may be clobbered
in between).  There is no flag-value cache that could go stale.  Closed with
code evidence, and the RA-01 kernel doubles as a live probe (its chain loop
re-tests both guards on every link; `check_benchmark_outputs.sh` pins the
behaviour at `-O0..-O3`).

**C. The 86 → 87 instruction delta is not a regression; the "86" was stale.**
Diffing the pre-wipe assembly against the current output: the old build
materialised `prev`'s address (`leaq prev(%rip),%r15`) and indexed through it;
the current build folds the access into `movzwl prev(,%rdi,2)` — one
instruction fewer and one fewer live register in the chain loop.  The earlier
record's "86" came from a mid-session build; the measured value on the rebased
tree is **87**, the baseline now records 87 with the mode-matched flags, and
the ratio to the best oracle (GCC 16.2, 55) is 1.58 — a hair worse than the
1.56 recorded before, because the reference number is the same while the LCCC
number moved by one instruction.

**D. My first reconstruction of the sparse-`.bss` fidelity check encoded a
wrong model of the object format — the gate was the bug, not the compiler.**
It compared `sh_size` against the *sum of symbol sizes*, which is invalid:
alignment gaps are real section bytes that no symbol owns, so the `aligned`
probe (`char c; __attribute__((aligned(64))) int a0, a1;`) legitimately has
`sh_size = 132` against a symbol sum of 9.  The check now asserts what is
actually true and checkable from `nm` alone, without linking: `sh_size` reaches
the end of the last `.bss` symbol, over-reservation stays within one cache
line, and every symbol's address is a multiple of its declared alignment.
(Four rows: `many-objects`, `aligned`, `array-then-obj`, `mixed-sections`.)
This is the second time in one day that a *gate's assumption* was the defect
(§4's char-signedness finding was the first); gates are code too.

**E. Requiring *both* publication halves could in principle block a legitimate
expansion.  Audited: nothing depends on the permissive direction.**  `set()`
runs in the driver's option handling, `set_module_state()` at codegen entry
from the same collector the pass-side gate snapshots, and both precede every
`may_assume_builtin` caller (`inline_memcpy_len`, `inline_memset_const_len`,
the `__*_chk` arms, the `puts` rewrite).  A forgotten publication now *fails
closed* — no library semantics are baked — which is the correct default for a
decision that licenses assuming the callee's behaviour.  Corroboration (not
proof): the full unit suite passes (4115 tests), and the oracle corpus shows
adler/gzip/expat counts byte-identical to the committed baseline while ra01
*improved*; a blocked expansion would have moved one of those numbers.

**F. The reconstructed Callgrind driver is not the lost one — and the ratio is
labelled accordingly.**  The kernel is the corpus kernel verbatim (`main` added)
and reproduces the exact static count (87); the driver prints the same stdout
as GCC at `-O0..-O3`.  But the dynamic ratio it measures (1.79 simulated Ir vs
local GCC 14.2) depends on the chain/iteration mix and is **not comparable** to
the 1.551 recorded for the lost driver.  The stable, comparable metric remains
the static oracle delta: 87 insns vs GCC 16.2 55 / ICC 52 / ICX 56 / Clang 106,
and `gzip_crc32_loop` at 13 vs 15 — the one row where LCCC beats every oracle.

**G. Deliberately not rushed.**  The register-offset symbol fold (§7) is a
register-allocation/liveness change in the backend; landing it without the full
battery (unit suite, correctness, benchmark outputs, all gates, oracle delta,
Callgrind) would violate "a fast wrong compiler is worthless".  Same for §A of
the sparse-equivalence gate, which needs a pre-change `--reference` build.  Both
are specified with their probes already in the tree.

## 12. Verification after the recovery (this session's numbers)

| check | result |
| --- | --- |
| cold `cargo build --profile fastbuild -j2` | **BUILD_EXIT=0** (3 m 47 s) |
| `cargo test --profile fastbuild --lib` | **4115 passed, 0 failed, 7 ignored** (was 4091/8 before the §4 fixes) |
| `tests/regression/check_char_signedness.sh` | **11 passed, 0 failed** — 5 behavioural rows match GCC 14.2 byte for byte; 6 preprocessor rows across the binary-name targets |
| `tests/regression/check_sparse_bss_equivalence.py` | sections B (4 rows, 16 GB `.bss` at 15-16 MiB peak) and C (4 rows, slack 0, alignments honoured) pass; A SKIP (no `--reference`) |
| `scripts/check_no_ub_exploitation.py` | `--selftest` PASS; scan clean (9 patterns, 0 hits) |
| `scripts/check_ci_gate_parity.py` | PASS — **142 commands** |
| local GCC 14.2 `-gz=` re-measurement | `none`/`zlib`/`zlib-gnu`/`zstd` accepted; `zlib-gabi` and `bogus` rejected |
| `scripts/oracle_delta_gate.py --update-baseline` + verification | exit 0, **no regressions**; lccc 87/269/13/70/81 vs oracles 55/132/15/67/50 (live CE) |
| benchmark program `ra01_global_match.c` | output identical to GCC at `-O0..-O3`; probe counts 87 insns (== corpus) |
| `scripts/callgrind_ab.py ra01_global_match` | lccc 3 240 066 Ir vs local GCC 14.2 1 807 290 Ir = **1.79** (simulated Ir; driver not the lost one, see §11 F) |
| `CI_LOCAL_JOBS=2 scripts/ci_local.sh --fast` | log `/home/user/results/ci-fast-s24.log` |

## 13. §7 closed (for the shape where it pays): the register-offset symbol fold

Shipped as the register-offset symbol fold commit on top of the re-applied
recovery series (see the series history in `artifacts/series/`; the pre-recycle
SHAs do not exist in this repository).  The chain is the one §7
specified: `Add(mapped symbol, register value)` becomes a foldable base
(`build_reg_offset_sym_map`), the arm is decided by `reg_off_sym_fold_ok` —
the shared predicate the DEADENING pass and the EMITTER dispatch both call —
and x86-64 emits `sym+disp(%off,%idx,scale)` loads/stores through the same
`sib_mem64*` family the plain symbol form uses.  The offset register is kept
live by the existing RA link extension (`collect_folded_gep_links_all`), the
same contract the SIB index has had since session 28.

**The dead-only policy, and why it is the measured choice.**  The first
implementation took the symbol form whenever the arm's static preconditions
held.  Measurement killed it: on the RA-01 kernel the fold fires in the two
probe compares, but the kernel's extension loop reads the SAME derived
pointer through the constant-offset path (`match[0]`, `match[1]`), so the
`Add` stays materialised either way — body 87 instructions with and without
the fold — while the offset register's live range grows.  The shipped policy
therefore takes the symbol form only when the deadening pass removed the
`Add`, i.e. only when EVERY use of it is a fold through this arm; that is the
only case where an instruction disappears.  The RA-01 kernel is consequently
byte-for-byte unchanged (pinned by the gate), and the fold is dormant there
until the constant-offset arm lands (see "what remains" below).

**Measured effect** (`tests/benchmark/programs/reg_off_window.c`; all three
builds print the identical checksum `391680000`, Callgrind, same flags):

| build | Ir | vs kill switch |
| --- | --- | --- |
| lccc + fold | 31 454 579 | **−8.90 %** |
| lccc, `CCC_NO_REG_OFF_SYM=1` | 34 526 575 | — |
| GCC 14.2 | 23 242 466 | (lccc/GCC = 1.35) |

Whole-chip shape A/B (`const u8 *m = win_window + cur; m[i] + m[j]`, the
gate's win probe): body 7 instructions folded vs 9 with the kill switch —
the `leaq`/`addq` pair is gone and one shuffle with it.  Oracle corpus
UNCHANGED: ra01 87 (1.58), adler 269 (2.04), gzip 13 (**0.87 — beats every
oracle**), expat 70 (1.04) / 81 (1.62), i.e. no regression anywhere the fold
does not apply.  Lib tests 4120/0/7 (+5 new unit tests on the map builder);
regression corpus 892/0; rustfmt + clippy clean; CI gate parity 143 commands.

**Red-team audit of this change** (same standard as §11 — every way it could
be wrong, and what closes it):

* *Deadening a live `Add`*: closed three times over — the deadening requires
  the attributed-use count to equal the total (only folds taking THIS arm are
  attributed), the emitter dispatch takes the symbol form only for values in
  `dead_global_addrs`, and both sides call the same predicate.  The gate's
  RA-01 row and the corpus are the behavioural check.
* *In-place extension of the offset register*: admitted deliberately, and it
  is the index contract verbatim (`ensure_sib_index_form`: low bits preserved,
  upper bits become the DEFINED 64-bit form; `movslq`/`movl`/`movzbq`
  according to the value's own type).  `CCC_NO_FOLDED_INDEX_LIVENESS`
  disables the whole indexed family including this arm, never half of it;
  `CCC_NO_REG_OFF_SYM` is the local switch.
* *Encoding limits*: `%rsp` in the index slot is refused explicitly (index=100
  means "no index"); the base slot may be `%rsp` because the symbol
  displacement always supplies a disp32 (mod=10).  Base == index is legal and
  semantically correct here (the base contributes unscaled, the index scaled).
* *PIC*: refused by the shared predicate AND by both emitters.  A SIB has two
  register slots; a symbol under PIC needs a staging register, so the fold
  would cost exactly the LEA it removes.  Pinned by a gate row.
* *GOT/TLS/absolute symbols*: cannot enter the map (it is built from the
  GOT/TLS/abs-filtered `global_addr_map`), and the emitters re-check
  `needs_got_for_addr` on the base symbol.
* *Store path*: store consumers keep their base (the existing value-staging
  caution), the store emitter is attempted only for deadened bases, and the
  gate's loop row (load AND store through such a base) compares against GCC.
* *MachInst replay fallback*: passes empty maps, so the fallback path is
  fold-free by construction (unchanged behaviour).
* *Symbol+symbol, multi-def `Add`s, unmapped operands*: refused by the map
  builder, each with a unit test.

**Red-team finding in my OWN change, fixed before it shipped: the refusal
path could read the deadened `Add`'s home.**  The audit above asserted that
"the only dynamic gate left is the register-form freshness that the RA link
extension is contractually responsible for" — and then the first version of
the code *depended on that assertion*: if `emit_load_indexed_sym_reg_base`
returned false (a home invalidated by an intervening clobber: `home_clobbered`
is populated at call windows, caller-saved restore windows and opaque
definitions), the dispatch fell through to `emit_load_indexed`, which reads
the base's register home — and the base is the `Add` this same change
deadened, so that home is never written.  That is precisely the bug class the
tree's own comments record as a past regression ("`movzwl (%rbx,%r14,2)` read
a never-written %rbx", `narrow_compare_constant_semantics`), so "the extension
makes it impossible" is not good enough: the tree's shipped symbol-base family
does not rely on such an assertion either — it has a rematerialisation path.

Fixed by giving this arm the same kind of escape, built on an EXISTING hook
rather than new machinery: on a dynamic refusal the dispatch now calls
`rematerialize_dead_reg_off_add`, which rebuilds `sym + off` through
`ArchCodegen::emit_rematerialized_global_addr` (the RA-01 emitter).  That hook
needs only the symbol's NAME and the offset OPERAND, so it stays sound even
when the offset's home is exactly what went stale: its general fallback routes
the operand through `operand_to_rcx`, i.e. the ordinary reload machinery
(home → slot → reload).  Only then does the access go through the generic
tail (`rematerialize_skipped_indexed` + `emit_load`/`emit_store`).  The escape
is two instructions on a path that the decision predicate makes unreachable
in the absence of a clobber — Callgrind re-measurement after the fix is
byte-identical to before it (31 454 579 Ir) — and it fails LOUDLY (`assert!`)
rather than compiling a wrong address if a future refusal class appears.
`RegOffSym` consequently carries `sym_val` (the symbol's IR value) so the
rebuild never has to touch a home.

**Oracle corpus row for the fold's shape (pinned).**  The shape above is now
entry `reg_off_window_probe` (`engineering/evidence/godbolt/reg_off_window_fold.c`)
in `tests/oracle/delta_corpus.json`, with its baseline row updated.  Measured
against the live Compiler Explorer oracles under the pinned corpus flags
(`-O2 -march=x86-64-v3 -fno-pie`):

| build | insns |
| --- | --- |
| lccc (folded) | **8** |
| GCC 16.2 / Clang 23.1 / ICX (latest) | 7 / 7 / 7 |
| lccc with `CCC_NO_REG_OFF_SYM=1` (pre-fold) | 9–10 |

All three oracles agree on the same shape — zero-extend the three u32 inputs,
two `movzbl win_window(%base,%idx)` operands, `addl`, `ret`.  lccc now emits
exactly that addressing shape; the single remaining instruction is
`leal (%rdi,%r10), %edi; movl %edi, %eax` where the oracles' add lands in
`%eax` directly.  That is a return-value-home artifact in the register
allocator, NOT the addressing fold: the same one-instruction tax appears on
any small integer-returning function whose result is computed into a
caller-saved home and then moved to the return register.  Fixing it means
biasing the RA's home choice for the function's returned value toward `%eax`
— a whole-allocator change that must clear the full corpus + oracle battery
on its own commit; it is written down here with the exact diff so the next
session starts from the measurement, not from a hunch.

**What remains open, stated precisely.**  The constant-offset path still
materialises the derived pointer.  RA-01's hot block therefore keeps

```
    leaq window(%rip), %r9
    addq %r10, %r9
```

while GCC's loop head has no materialisation at all.  Closing it means giving
`emit_{load,store}_with_const_offset` a symbol+register-base arm — the const
emitters are type-branching and (unlike the indexed ones) non-refusable, so
the arm must be complete for every type they accept in ONE commit; that is
its own battery, not a rushed add-on.  Target: the 2-instruction loop-head
materialisation, i.e. the RA-01 probe body 87 -> 85 and the loop's per-iteration
count down by two, to be measured with the same oracle + Callgrind protocol
before it ships.
