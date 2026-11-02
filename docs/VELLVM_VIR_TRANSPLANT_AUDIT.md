# Red-team audit: `vellvm` and `vir` → LCCC transplant analysis

**Date:** 2026-10-18 · **LCCC base:** `ms178/lccc` `main` = `50167bc26ae4b43643ed44a4d63e10f00d83434e` (PR #857; session started at `d5b3d481`, PR #855) · **Sources (shallow clones, read-only):**

| Project | Commit | Language / prover | License | Size |
|---|---|---|---|---|
| [`vellvm/vellvm`](https://github.com/vellvm/vellvm) | `4c6db50077cbc0df3fb3156c0570129201cc35f3` | Rocq 9.1 + OCaml (`src/rocq`, 84 `.v` files, 45 958 lines) | MIT | 132 MB clone |
| [`jlebar/vir`](https://github.com/jlebar/vir) | `c61e3517a12921395b3c043805844863901a0d60` | Lean 4.27.0 (`Vir/`, 3 648 lines) | Apache-2.0 | 429 KB clone |

Licensing: LCCC's own contributions are `MIT OR Apache-2.0 OR BSD-2-Clause`
([`licensing.md`](licensing.md)). **No source text was copied from either project.**
The only concepts reused are described below; every transplanted artifact is
an original LCCC file with its own provenance comment.

> **Audit boundary.** This is a source-level and executed-tooling audit. Vir was
> built and its test suite was run; Vellvm was **not** built (a Rocq 9.1 + opam
> switch is a multi-hour toolchain on this 2-core host) and its claims are
> assessed from source only. Nothing here is a compiler-performance claim.

---

## 0. TL;DR

1. **`vir` is a proof-of-concept skeleton, not a verified compiler.** It has an IR
   of 5 instruction kinds (`bin`, `icmp`, `br`, `switch`, `ret`) and no `phi`, no
   memory, no calls. Its only *theorems* are 8 `theorem`/`lemma` declarations. Its
   correctness claims rest mostly on 45 `native_decide` examples, which trust the Lean
   compiler and runtime. **Red-team result:** the test suite is **blind to an
   off-by-one in its SSA dominance rule** (reproduced below). Its value to LCCC is a
   checklist of invariants and one negative-test lesson, not code.
2. **`vellvm` is a serious formal semantics of LLVM IR** (1 041 lemmas, 1 052
   `Qed.`, **0** `Admitted.`), with a byte-level memory model, `freeze`, poison and
   provenance. It verifies *semantics and equivalences*, not optimizer passes. Its
   value to LCCC is as a **reference model** for memory-punning behaviour. It also
   has four explicit assumptions in the trusted base, and a documented simplification
   (`undef` ≡ `poison`) that LCCC must **not** adopt.
3. **Five transplants were implemented and validated** (§5, status `DONE`):
   a third, code-disjoint dominator oracle; a self-use SSA regression test; and a
   byte-precise type-punning differential fuzzer built on a Vellvm-style byte model.
   The fuzzer's model was itself validated against GCC (1 500/1 500) before it was
   used against LCCC (5 000/5 000 pass, see §6).
4. **Eight follow-up items** are registered (§8), including a C-level test-case
   reducer, which LCCC does not have, and a silent-round-cap audit.

---

## 1. What was measured (not assumed)

| Claim | Evidence (reproduced in this session) |
|---|---|
| `vir` builds | `lake build` → *Build completed successfully (52 jobs)*, exit 0 (toolchain `leanprover/lean4:v4.27.0` via elan). |
| `vir` tests are compile-time assertions | `lake test` builds `VirTests`; no runtime test driver exists (`lakefile.lean`, `@[test_driver] lean_lib VirTests`). Exit 0 at baseline. |
| `vir` proof content is thin | `grep -rn "^theorem\|^lemma"` → **8**; `native_decide` → **45**, all in `Vir/IR/ASTTest.lean`, `ValidationTest.lean`, `DomTreeTest.lean`. |
| `vir` IR is minimal | `Vir/IR/InstrCore.lean`: `inductive Instr` = `bin \| icmp \| br \| switch \| ret`. |
| `vir` dominance is set-based | `Vir/Analysis/DomTree.lean`: `Dom(n) = {n} ∪ ⋂ Dom(p)` over `ExtTreeSet`s, i.e. O(n²) space. The README itself says "Avoid O(n²) algorithms". |
| `vellvm` has no admitted proofs | `grep -rn "Admitted\." src/rocq` → **0**; `^Qed\.` → **1 052**; `^Lemma\|^Theorem` → **1 041**. |
| `vellvm` trusted-base assumptions | `src/rocq/Utils/ParserHelper.v:63` `Axiom Zlog2_log_inf`; `src/rocq/Numeric/Archi.v:31` `Parameter ppc64`; `src/rocq/Numeric/Integers.v:60-61` `Parameter wordsize` + `Axiom wordsize_not_zero`. |
| `vellvm` has no optimizer proofs | No file in `src/rocq` proves a transformation (`mem2reg`, SCCP, LICM…) preserves semantics; matches are README prose only. |
| `vellvm` known open failure | `tests/undef/README.md`: `insertvalue` into `undef` fails 2/5 `ASSERT EQ` cases after `undef` was folded into `poison`. Fix proposed, not implemented. |
| `vellvm` semantic simplification | Same README: on the `ctrees-minimal` branch `undef` is *treated as* `poison`. This is weaker than LLVM's `undef` (a per-use arbitrary value). |

---

## 2. Red-team: `vir`

### 2.1 Verified defect class: the test suite does not catch an SSA dominance off-by-one

`Vir/Analysis/InstrDomTree.lean:89` implements same-block dominance as
`defLoc.index < useLoc.index`. An instruction that consumes its own result
(`%2 = add %2, %1`) has `defLoc = useLoc`, so only the *strictness* of `<` rejects it.

**Experiment (run in this session, clean `.olean` rebuild before each state):**

| Rule | Self-use scratch file (`native_decide` says `operandsDominateUses = false`) | Full `lake test` |
|---|---|---|
| original `<` | proposition holds → **exit 0** (self-use rejected) | exit 0 |
| mutant `<=` | proposition **false** → **exit 1** (self-use *accepted*) | **exit 0** (suite blind) |

So the existing suite contains no self-referential negative case: reading the
fixture definitions in `ValidationTest.lean` and `DomTreeTest.lean`, no instruction
operand ever equals its own id. The use-before-def fixture (`instrs := #[val3, val2]`)
uses two *different* instructions.
**Transplant:** LCCC gets the self-use regression (§5, T2), mutation-verified.

**Methodological trap found while doing this:** after restoring a mutated source
file, `lake env lean` still loaded the *mutant* `.olean`s. The first "original"
measurement was therefore invalid. Rule: **after every source restore, run
`lake build` before measuring.** This is recorded in §9.

### 2.2 What is sound in `vir` (worth keeping as ideas)

- **Validation as predicates with fail-closed defaults.** `Function.IsOk` is a
  conjunction over blocks, values and dominance, and `none` maps to `False`.
  LCCC's `verify.rs` already embodies this as 8 checks with violation lists.
- **Location-equality guard.** `operandsDominateUsesInBlockFrom` requires
  `location? instrId == some (block, index)`. A stale or duplicated index cannot
  validate by accident. Equivalent protection in LCCC is check 7 (single definition).
- **Fuel-bounded fixpoint returning `Option`.** `Data/Digraph.lean fixedPoint`
  returns `none` when fuel runs out. This is the right *shape* for any iterative
  analysis; see T5 for the LCCC audit that follows.

### 2.3 What is weak or misleading in `vir`

- The README and file headers say "validation-focused IR skeleton", but the
  top-level `README` also calls the project a "formally-verified compiler" in its
  motivation. Only validation predicates exist; no semantics, no pass, no
  preservation theorem.
- `Digraph.reversePostOrderFromMany` uses `fuel = nodes.size + 1` per recursion
  depth. It is correct for finite graphs, but it is a DFS with an artificial bound
  rather than an explicit stack.
- The dominator algorithm is the O(n²)-set dataflow that the README tells
  contributors to avoid. It is useful only as a *reference* (see T1).
- Unreachable blocks are initialised optimistically (`Dom = all`). Whether uses inside
  unreachable blocks are then accepted is not covered by any test; this was noted but
  not investigated further.

### 2.4 Verdict on `vir`

**Low transplant value in code; medium value as a checklist and as a negative-test
lesson.** Do not port the Lean IR. LCCC's IR is far richer (`phi`, memory, calls,
asm goto) and its verifier is stronger than `vir`'s validation.

---

## 3. Red-team: `vellvm`

### 3.1 Strengths (verified from source)

- **Byte-level memory model** (`src/rocq/Semantics/MemoryBytes.v`, 832 lines;
  theory in `Theory/MemoryBytesFacts.v`, 7 lemmas). Stores serialise to bytes,
  loads deserialise whatever bytes are present, regardless of the writing type.
  This is precisely the discipline needed to specify store→load forwarding across
  types (T3).
- **`freeze` per bit** (`Semantics/Operations/Freeze.v`): poison bits become bits
  of one arbitrary-but-fixed value, provenance preserved. A clean spec for
  `freeze`, should LCCC ever model poison.
- **GEP with explicit padding arithmetic** (`Operations/Gep.v`, `handle_gep_h`,
  struct offsets via `pad_to_align`). A readable reference for C struct layout.
- **Refinement with an explicit OOM cut-off** (`Theory/I2F`, `ruttc`): OOM is
  treated as the dual of non-time-travelling UB. Interesting theory; not relevant
  to a C compiler's codegen.

### 3.2 Weaknesses and risks (red-team)

| # | Finding | Severity for LCCC | Evidence |
|---|---|---|---|
| V1 | Four trusted-base assumptions (`Axiom`/`Parameter`) are not discharged. | Low (Vellvm's problem) | `ParserHelper.v:63`, `Archi.v:31`, `Integers.v:60-61`. |
| V2 | `undef` is modelled as `poison` on the minimal branch, a known deviation from LLVM. | **High if copied** | `tests/undef/README.md`. LCCC must keep distinct semantics. |
| V3 | Known failing aggregate case: `insertvalue` into `undef`/`poison` fails. | Medium (a *test idea*) | `tests/undef/README.md`, `memory/insertAndExtractValue.ll`. |
| V4 | No optimizer correctness proofs in the repo; the "verified compiler" framing is about semantics. | Medium (expectation setting) | §1 table. |
| V5 | Test corpora mix round-trip checks (`test_pp_dir`) with semantic `ASSERT EQ`. The `undef` corpus mainly checks parsing. | Low | `tests/undef/README.md`. |
| V6 | Build needs Rocq 9.1, OCaml 4.14.1, opam and Clang: impractical on the research host. | Process | `README.md` "Assumes". |

### 3.3 Corpora worth knowing about (not transplanted as-is)

- `tests/undef/alive2/` contains **71** `*.srctgt.ll` refinement pairs (Alive2-derived).
  They are LLVM-IR input, and LCCC is a C compiler without an `.ll` front door, so
  they cannot run directly. Deferred as T8.
- `tests/undef/yarpgen/` holds one generated program; `tests/memory/*.ll` covers
  GEP/alignment/`inttoptr` cases; `utilities/vellvm-reduce.sh` is a bugpoint-based
  reducer (see T4).

### 3.4 Verdict on `vellvm`

**High value as a specification and as a source of test *shapes*; low value as code.**
The most transferable artifact is the byte-serialisation discipline (T3). The most
dangerous artifact is the `undef ≡ poison` shortcut (V2), which must be recorded as
a rejected idea so it is never re-derived.

---

## 4. LCCC baseline: what already exists (checked, not assumed)

| Area | LCCC state | Consequence for the transplant |
|---|---|---|
| IR verifier | `src/passes/verify.rs`: 8 checks incl. phi-predecessor validity, single definition, def-dominates-use with CHK + preorder intervals. Enabled by `CCC_VERIFY_IR=1`, **off by default**. | `vir`'s dominance validator is strictly weaker. Do not port it. |
| Verifier tests | `src/passes/verify/tests.rs`: 32 tests. No self-use case before this audit. | T2 closes the gap. |
| Dominators | `src/ir/analysis.rs`: SNCA (`compute_dominators`) + CHK test oracle; differential test on 2 000 random CFGs. | T1 adds a third, code-disjoint oracle. |
| Load forwarding | `src/passes/store_load_forward.rs`, `load_forward.rs`, `redundant_loads.rs`, `byte_load.rs`. Same-width reinterpretation only; size-mismatch not forwarded (test `size_mismatch_not_forwarded`). | Conservative; the punning oracle (T3) checks it differentially. |
| Differential fuzzers | `tests/fuzz/differential_fuzz.py`, `phi_cfg_fuzz.py`. **No `union`/`memcpy`/mixed-width punning generator.** | Gap closed by T3. |
| C-level reducer | None. The only "reduce" hits are benchmark-related. | Gap → T4. |
| Silent round caps | `MAX_ROUNDS` in `loop_carried_forward` (3), `backedge_pre` (3), `reassoc_latency` (8), `bool_thread` (512); `loop_preheader` (blocks+1); inline `max_rounds`. Loops `break` on no change and **silently stop at the cap**. | Sound by construction (each round is a complete rewrite), but unobservable. → T5. |

---

## 5. Transplant register

Status: **DONE** = implemented and validated here · **FOLLOW-UP** = registered with
next step in [`engineering/FOLLOWUP-2026-10-18-vellvm-vir-transplant.md`](../engineering/FOLLOWUP-2026-10-18-vellvm-vir-transplant.md) · **REJECTED** = decision recorded, do not re-derive.

| ID | Source | Concept | LCCC consumer | Status | Evidence / decision |
|---|---|---|---|---|---|
| **T1** | `vir` `DomTree.lean` (set equations) + red-team of `vir` | Code-disjoint *ground-truth* dominator oracle | `src/ir/analysis.rs` tests | **DONE** | §6.1. Oracle self-checked on textbook shapes; mutation-verified. |
| **T2** | `vir` `InstrDomTree.lean:89` (strict index rule) + §2.1 experiment | Self-use SSA negative regression | `src/passes/verify/tests.rs` | **DONE** | §6.2. Mutation `<` → `<=` fails the new test. |
| **T3** | `vellvm` `MemoryBytes.v` / `MemoryBytesFacts.v` (byte serialisation) | Byte-precise type-punning differential fuzzer with independent reference model | `tests/fuzz/punning_fuzz.py` (+ `test_punning_fuzz.py`) | **DONE** | §6.3. Model validated vs GCC 1 500/1 500; UBSan/ASan clean; LCCC 5 000/5 000. |
| T4 | `vellvm` `utilities/vellvm-reduce.sh` (bugpoint + two-oracle interestingness) | C-level test-case reducer for miscompiles (two-oracle: error preserved, or output differs from reference) | new `tests/fuzz/` tool | FOLLOW-UP | Gap confirmed (§4). Needs C-aware shrinking (line/AST delta-debugging). |
| T5 | `vir` `Digraph.fixedPoint` (fuel → `Option`, fail-closed) | Make silent `MAX_ROUNDS` exits observable | `src/passes/round_cap.rs` + 5 round loops | DONE (observability) | Five loops (`loop_carried_forward`, `backedge_pre`, `reassoc_latency`, `bool_thread`, `loop_preheader`) now count cap exits with work remaining; printed under `CCC_TIME_PASSES`. Measured over 2 654 C files at `-O2`: the cap binds in 2 files (both `backedge_pre`). Raising its cap 3→12 changes the code in both but moves callgrind Ir by 6 and 13 instructions (0.00007 % and 0.008 %), so the cap is **kept** (decision recorded, not a claim of gain). Gate: `check_round_cap_observability.sh`. `inline.rs` (budget-based, two passes) is not covered and is a separate follow-up. |
| T6 | `vir` `Function.IsOk` / `Block.IsOk` (fail-closed predicate style) | Enable IR verifier in more gates (not only opt-in) | `ci_local.sh`, `CCC_VERIFY_IR` | FOLLOW-UP | Needs compile-time cost measurement first (RULES #1, #59). |
| T7 | `vellvm` `Gep.v`, `MemoryBytes.v` (struct padding via `pad_to_align`) | Layout oracle: compare LCCC `sizeof`/`offsetof` against an independent C-layout model | `tests/` layout tests | FOLLOW-UP | Extend T3 generator with structs/bitfields/`long double`. |
| T8 | `vellvm` `tests/undef/alive2/` (71 refinement pairs) | Refinement corpus | optimizer tests | FOLLOW-UP / deferred | Requires an IR-level entry point; LCCC is C-only. Revisit after an `.ll` reader exists. |
| T9 | `vellvm` `tests/undef/` `insertvalue` failure | Aggregate-with-undef test in LCCC (C analogue: partially initialised struct, field read) | `aggregate_sroa`, `store_load_forward` tests | FOLLOW-UP | Not yet reproduced on LCCC. Do **not** claim a bug until a witness exists. |
| T10 | `vellvm` `Theory/Equational.v` (rewrite equations as lemmas) | Property tests for peephole identities | `src/backend/*/peephole` | FOLLOW-UP (low) | Low priority; existing peephole gates cover much of this. |
| T11 | `vellvm` `Theory/I2F` (OOM as dual of UB) | Model address-space exhaustion | — | REJECTED | No consumer in a C codegen. |
| T12 | `vellvm` `undef ≡ poison` (minimal branch) | Collapse `undef` into `poison` | — | **REJECTED** | Weaker than LLVM; would make LCCC's optimizations less conservative or unsound. Recorded in §7. |
| T13 | `vir` whole Lean IR as LCCC IR | Port IR / validator | — | **REJECTED** | No phi/memory/call; O(n²) sets; verifier already stronger; see §2.4. |
| T14 | `vellvm` Rocq semantics as runtime oracle | Run Vellvm's extracted interpreter on LCCC IR | — | **REJECTED** | Needs an `.ll` path that LCCC lacks; Rocq/OCaml toolchain cost. Use its *spec*, not its binary. |

---

## 6. Implemented transplants: evidence

### 6.1 T1 — set-equation ground-truth dominator oracle

`src/ir/analysis.rs` (test module `dominator_tests`):

- `naive_idom`: textbook dataflow `Dom(entry) = {entry}`, `Dom(n) = {n} ∪ ⋂ Dom(p)`
  over reachable predecessors, fixed point; immediate dominator = strict dominator
  with the largest dominator set (strict dominators form a chain). Shares **no code**
  with SNCA or CHK.
- `set_equation_oracle_matches_textbook_shapes`: validates the oracle first, on the
  diamond with an unreachable block, the irreducible loop, and a singleton.
- `matches_set_equation_ground_truth_on_random_cfgs`: 1 500 random CFGs (up to 48
  blocks, back/forward/cross edges, self loops, unreachable islands) — both SNCA and
  CHK must equal the oracle.

**Results:** `cargo test --profile fastbuild --lib dominator_tests` → 5 passed.

**Negative control (mutation):** `max_by_key` → `min_by_key` in `naive_idom`.
→ `set_equation_oracle_matches_textbook_shapes` **FAILED**,
`matches_set_equation_ground_truth_on_random_cfgs` **FAILED**;
the pre-existing `matches_cooper_harvey_kennedy_on_random_cfgs` **passed**. This
shows that the old CHK-vs-SNCA test cannot detect a shared misreading, while the new
oracle can. Source restored and re-verified (`max_by_key` count = 1).

### 6.2 T2 — self-use SSA regression

`src/passes/verify/tests.rs::an_instruction_consuming_its_own_result_is_reported`:
`v9 = copy v9` in a single block must be reported as "does not dominate its use".

**Results:** passes with the original rule (`di < ui`; all 32 `passes::verify` tests green).

**Negative control (mutation):** `is_none_or(|ui| di < ui)` → `di <= ui`
→ the new test **FAILED** (`panicked at src/passes/verify/tests.rs:102`).
Source restored (`di < ui` count = 1).

### 6.3 T3 — byte-precise type-punning differential fuzzer

`tests/fuzz/punning_fuzz.py` generates deterministic C translation units that store
and load `union cell` (8 bytes) through **defined** punning forms only: union member
access, `memcpy` at arbitrary byte offsets, and `unsigned char` byte access. Each
generated body mixes `u8…i64` reads and writes, and each body runs on a local and a
`static` object over 2–4 rounds. The expected output is computed by an independent
Python byte model (little-endian, modular conversions), in the spirit of Vellvm's
`MemoryBytes`.

**Validation chain (order matters):**

1. **Model vs GCC (oracle self-check):** `--ccc gcc --seeds 0:300 --levels O0,O1,O2,O3,Os`
   → **1 500 / 1 500 pass**. Two defects in the *first* model draft were found and
   fixed by this check: (a) the model advanced the value stream on read operations,
   which the C does not do (750/750 failures); (b) per-seed parallel jobs deleted each
   other's source files (35 compile errors). Neither was a compiler bug.
2. **Generator definedness:** 60 seeds compiled with `-Wall -Wextra
   -fsanitize=undefined,address -fno-sanitize-recover=all`: **0** warnings, **0**
   sanitizer reports.
3. **LCCC (fastbuild binary, base commit + test-only changes):**
   `--seeds 0:1000 --levels O0,O1,O2,O3,Os` → **5 000 / 5 000 pass**.
4. **Unit tests** `tests/fuzz/test_punning_fuzz.py`: 7 tests (arithmetic, little-endian
   encoding, round trip for all types, determinism, in-bounds and alignment invariants,
   output format, and an in-tree GCC agreement check that runs when `gcc` is present).

**Honest reading of result 3:** LCCC passes this class. That is a regression *oracle*
for store-to-load forwarding with mixed-width punning, not a discovered bug. The
generator does **not** yet cover `float`/`double` bit punning, `__int128`, bit-fields,
struct members or overlapping `memcpy` (listed as T7).

---

## 7. Rejected ideas (do not re-derive)

- **`undef ≡ poison` (T12).** Vellvm's minimal branch does this. Under LLVM semantics,
  `undef` may take different values at each use, while `poison` taints the whole
  dependency chain. Merging them would either block valid optimizations or license
  invalid ones. LCCC is a C compiler, so the closest C notion is indeterminate
  values, and those must stay separate from UB.
- **Porting `vir`'s Lean IR (T13).** Five instruction kinds, no `phi`, no memory, O(n²)
  dominance, and an author note that the code is "almost completely vibe-coded". The
  existing verifier is stronger in every dimension checked.
- **Running Vellvm's interpreter as an oracle (T14).** It needs `.ll` input, which
  LCCC does not accept, and a Rocq/OCaml toolchain that does not fit the host.

## 8. Follow-up register (summary)

Full text and next steps: [`engineering/FOLLOWUP-2026-10-18-vellvm-vir-transplant.md`](../engineering/FOLLOWUP-2026-10-18-vellvm-vir-transplant.md).

Priority = expected value × breadth ÷ cost. Next three: **T4** (C reducer), **T7**
(layout and punning extensions), **T5b** (`inline.rs` round budgets, same counter API).
T5 itself is done as observability; see the register row above.

## 9. Methodological lessons carried forward

1. **Rebuild after every source restore before measuring.** Lean `.olean`s and Rust
   incremental artifacts can outlive a reverted mutation. A "baseline" taken before
   `lake build` was wrong in this session.
2. **Validate an oracle on known answers before trusting it** (T1 validates
   `naive_idom` on textbook shapes; T3 validates the model against GCC).
3. **Every new test is mutation-verified** (RULES #5): the test must fail when the
   bug it targets is reintroduced. T1 and T2 both were.
4. **"The suite is green" is not evidence** when the suite does not contain the
   failing shape. `vir`'s suite is green under an accepted self-use.
5. **Treat formal-verification badges as scope statements.** Count `theorem`,
   `Qed`, `Admitted`, `native_decide`, `Axiom`, and `Parameter` before believing any
   "verified" claim.

## 10. Reproduction

```bash
# vir (needs elan; toolchain pinned by lean-toolchain)
cd vir && lake build && lake test; echo $?
# self-use mutation experiment: see engineering/FOLLOWUP-2026-10-18-vellvm-vir-transplant.md §3

# LCCC
cargo test --profile fastbuild --lib dominator_tests
cargo test --profile fastbuild --lib passes::verify
python3 -m unittest discover -s tests/fuzz -p 'test_*.py'
python3 tests/fuzz/punning_fuzz.py --ccc gcc --seeds 0:300 --levels O0,O1,O2,O3,Os --out /tmp/p-gcc
python3 tests/fuzz/punning_fuzz.py --ccc target/fastbuild/lccc --seeds 0:1000 --levels O0,O1,O2,O3,Os --out /tmp/p-lccc
```
