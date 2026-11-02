# Round 10 — the #788 follow-ups, and an evidence-based reading of #781/#777/#776/#725

Base: `ms178/lccc` main = `106b7e35` ("Merge pull request #788 from
ms178/arena/23ccb971-lccc"), i.e. the round-8/9 work (`aaf23ea0..a71df4ce`,
delivered as `5bb11474`) is **merged**. `git rev-parse origin/main` at the time of
writing; #788, #787, #785, #784, #783, #782, #780 are closed and #786, #781, #777,
#776, #725 are open (GitHub API, not memory).

The CI log attached to the request for this round was stale: it prints
`legality probe: 411 spellings` and `no binutils sys-regs.def: pass --def …`, which
is the pre-fix text from the #785 run at `d6f7591d` — main has carried
`_gate_inputs_ok` and the `share/opcodes` install since `2dd4f8d6`, and `#788`
merged it. The live check-suite for `106b7e35` at the time of writing was Clippy
**success**, Compiler Explorer oracle delta **success**, Test Suite and LCCC-vs-GCC
**in_progress**. Nothing here claims main is green; it claims the red that was
quoted is the red that #788 fixed, which is a different statement.

## 1. The nine follow-ups, each measured before it was edited

The follow-up brief came from an audit of tree `b7287678` (28 files, +4182/−398,
35 new tests). Its verdict (merge #788, close #787, do not merge #785) is
independently correct — #787 was byte-identical to #785's head and #785 was the
branch that had already been superseded. Per standing instruction, each item was
verified against the code before being acted on, and three of its citations did not
survive that check.

| # | item | verdict | what the measurement showed |
|---|---|---|---|
| 1 | region latch in the dispatcher ratchet | **agree, fixed** | `src[a + 10..].find("\npub fn ")` returns −1 *today*: the counted region ran 104,346 B to EOF instead of the 54,114 B that are the function, and a rough arm scan saw 569 "arms" out there vs 528 inside. Now delimited by the first column-0 `}`, loud if absent, in both copies. |
| 2 | orphan `///` block + `small_nop` field doc | **agree, fixed** | The block sat between `new()` and `ensure_section()` with a blank line after it. The field doc claimed RVC conditionality that `riscv/assembler/elf_writer.rs:507` (`with_small_nop(RISCV_C_NOP)`, unconditional) contradicts. |
| 3 | RISC-V `.p2align` cap/fill pins | **agree, fixed** | The parser module had 3 tests and none pinned the cap-drop, the explicit-fill survival, `.p2align 4,,` vs `,,,`, or the exponent clamp. `gas247_riscv_align_tail_verdicts_at_the_parser` now pins all four plus the `.balign` tolerances. |
| 4 | string-sniffed tail errors | **agree, fixed** | `Err(e) if e.contains("fill") \|\| e.contains("too many")` replaced by `AlignTailError::{BadFill, BadMax, TooMany}` with `to_message()` reproducing the old text byte-for-byte, so no diagnostic pin had to move. |
| 5 | no unit tests for the tail grammar | **agree in substance, wrong in citation** | There is no `src/backend/riscv/assembler/asm_expr.rs`; the shared parser is `src/backend/asm_expr.rs`, which *does* have `mod tests` (11 tests). What it lacked was tail coverage, so `gas247_align_tail_grammar` was added there. |
| 6 | `.macro` double charge vs `.rept` | **agree, documented + pinned** (not "fixed") | The doubling is over-counting on nesting, and `.rept`'s per-frame multiply is exact; both are ≥ what is materialized. Halving it would weaken the ceiling on the class that costs the most, so the law is documented on `ExpansionTotals::charge` and pinned by `nested_macro_output_is_billed_twice` (4 lines materialized, 7 billed). |
| 7 | prose drift | **partly agree** | `~401 mnemonic arms` → 526, with the `# arms:` line named as the authority. "the `+1`/`−1` in `tests/regression/check_ensure_gcc_torture.sh`" does not exist: that file has no count arithmetic; the `+10` magic offset it probably meant is *gone*, which removes the need for the explanation. A stale claim the audit missed was also fixed: `encode_gp_reg` cited "the 1,624-row `sweep register class` group … found 401 of them" while the TSV is 10,411 rows with 3,773 in that group (counted now); the number was dropped rather than refreshed. |
| 8 | `neon_shapes_agree` scope | **agree, and the defect it points at is real** | See §2 — eleven leaves now call it, and a census gate makes the set mechanical. |
| 9 | merge order / dedup | **agreed with a correction** | See §3. The audit's premise that #786 and #788 collide is right — but the collision is on `scripts/ensure_gas_247.sh`, where #786 adds 34 lines and deletes lines main still has, and on `scripts/README.md`. #786 must rebase onto `106b7e35`, not the other way round. |

## 2. The defect found while closing item 8

`encode_neon_three_same`, `add_sub`, `logical`, `bic`, `bsl`, `mul`,
`bitwise_insert`, `zip_uzp`, `pmul`, `mla`, `mls` all read the destination's
arrangement, discarded the sources' (`let (rn, _)`), and derived one `size`/`Q`
from it. Measured with the fastbuild binary before the change:

```
add v0.4s, v1.4s, v2.8h      exit 0
mla v0.8h, v1.16b, v2.16b    exit 0
pmul v0.8h, v1.16b, v2.16b   exit 0
sqdmulh v0.4h, v1.4h, v2.8h  exit 0
```

A three-same form has one `size`/`Q` triple for all three operands, so those
spellings have no encoding at all: LCCC assembled a *different instruction than the
source named* and reported success. All eleven leaves now call `neon_shapes_agree`,
which the float leaves had already been using for the same reason.

Two things make this more than a local patch. `three_same_leaves_all_agree_check`
enumerates the qualifying leaves out of `neon.rs` (three `get_neon_reg` reads plus
`size`/`Q` derived from the destination) and fails on a leaf without the check, on
an expected name the scan stops seeing, and on a stale exemption — writing it
caught two of its own bugs (chunking the file on `pub(crate) fn` alone let
`encode_neon_not`, a two-operand form, inherit its neighbour's body; and
`src[s0..].find('(')` found the paren inside `pub(crate)` and sliced backwards).
Second, the legal mixed-shape families are pinned in the same test so the check
cannot be "generalized" into them: indexed `mla v0.4s, v1.4s, v2.s[1]`,
`saddl v0.8h, v0.8b, v8.8b`, `pmull v1.1q, v13.1d, v21.1d`, and the two GAS golden
words (`pmul v17.16b, …`, `add d10, d7, d17`) read the way the ratchet reads them —
`u32::from_le_bytes` over the TSV's byte string, which is not the hex as a number.

`mul v0.4s, v1.4s, v2.4d` and `mls v0.4s, v1.4s, v2.4d` are rejected too, but by the
operand parser ("expected NEON register at operand 2, got Some(Symbol(…))"), not by
the shape check. The verdict is right and the wording is worse than GAS's; recorded
here as an open diagnostic-quality item rather than silently papered over, because
the fix belongs in the operand-class fallback, not in the encoder.

## 3. Open PRs, measured rather than remembered

`analyze.py` (in this session's workspace, not in the tree) compares each PR's
substantive added lines against main's copy of the same file:

| PR | files | substantive added lines | already on main | reading |
|---|---|---|---|---|
| #725 | 4 | 148 | **89.9 %** | Superseded. The residue is `directive_emits_data` prose in `x86/codegen/peephole/passes/flag_peepholes.rs`, and main carries that function today (round-8's fill-aware version, which also skips `.cfi_*` and shares `vector_copy::benign_directive`). Close. |
| #776 | 33 | 4 378 | **98.6 %** | Superseded — 61 unmatched lines of wording across nine files, no behaviour. Close. |
| #777 | 14 | 965 | 11.5 % | Mostly un-merged, and its headline claim is retracted *by its own* §6: the 15.9 % loop-invert regression does not reproduce (+0.57 % on a 61-sample interleaved A/B of the same binary), and §6.4 shows the `min`-of-31 statistic that produced it inventing 5–25 % effects when both arms are identical code. The memory licence is "implemented, measurable, **not shipped**" and main has no `CCC_LOOP_INVERT_MEMORY` in `src/` — only in the audit notes. Adopt the census instrument; do not ship a pass whose measured gain is noise. |
| #786 | 53 | 1 901 | 2.6 % | Un-merged and live: named-segment spaces through x86 lowering. Collides with what #788 just merged on `scripts/ensure_gas_247.sh` (34 added lines, and its deletions are of lines main still has) and on `scripts/README.md`. Rebase onto `106b7e35`. |
| #781 | 42 | 1 722 | 2.0 % | Un-merged, one commit, and it rebases onto current main with **one** conflict (`scripts/README.md`). See §4. |

## 4. What #781 actually contains, and what was done about it

Cherry-picking `d8649508` onto `106b7e35` auto-merged 41 of 42 files. The conflict
is the scripts table, and resolving it is also a defect fix in the PR: its README
row re-states the *old* snapshot policy ("Refuses a tree without a matching
`mode=full` `ci_local.sh` stamp"), which main superseded — the owner's standing
instruction is that the full/slow CI never runs on this VM, and `lccc-snapshot.sh`
accepts a `--fast` stamp for the same tree. The row is resolved toward main; the
PR's new `torture_evidence.py` row is kept.

The substantive half is worth more than the audit gave it credit for:

* i686 fastcall chain rule: a *nonempty* aggregate breaks the register chain, a
  zero-sized GNU aggregate does not consume bytes or break it (with the GCC oracle
  transcript in the doc comment, `f(int,S2,int)` → `ret $12`).
* Call-site struct copies use the **object** size, not the ABI-padded slot size:
  "copying the rounded size can cross a guard page" — an over-read fix, exercised by
  `tests/regression/large_aggregate_call_copy.c`, whose last case ends the source
  immediately before an inaccessible page.
* `rep movsb` staging with ESI/EDI preserved from function entry, *and* the prologue
  marks them used whenever a `CallInfo.struct_arg_sizes` entry reaches
  `LARGE_ARG_COPY_THRESHOLD`, so CFI still describes the caller's values mid-REP.
  That is the part that would otherwise be a miscompile.
* `tests/regression/check_empty_aggregate_abi.sh` is a genuine cross-compiler
  differential: LCCC-caller×GCC-callee and the reverse, `-m64` and `-m32`, five `-O`
  levels, linking and **executing** each pair; a missing 32-bit runtime is an
  infrastructure failure, not a skip.

The two `tests/corpus/test_*.py` contract tests, the GCC-17 sparse-corpus
provisioner, and the Expat workload are additive. The `src/frontend/sema/*`
hunks (209 added lines, K&R prototype promotions) touch code main changed
meanwhile (106 main-only lines), so they are the part that needs the careful
re-read rather than a merge.

Two of its `README` rows also had to be resolved *against* the PR: it re-stated
the retired "a full local `ci_local.sh` stamp is required" policy, which the owner
reversed (the full/slow CI never runs on this VM; GitHub runs it per PR), and it
wired both new gates into the hosted job *without* installing the 32-bit runtime
they insist on -- which would have made the first hosted run red for the wrong
reason.  `1dae1414` adds the `gcc-multilib`/`libc6-dev-i386` step, following the
`mold`/`lld`/`gcc-14` pattern the same job already uses, and leaves both checks
refusing to skip.

What that bought, measured on this VM (where `-m32` works): the empty-aggregate
gate, run against **main** (`106b7e35` + this branch's assembler work, i.e. without
#781's lowering fix), reached `-m32 -O0 lccc-caller` and the mixed-compiler binary
exited **3** -- `fast(a, e, 13, 5) != 163`, the `fastcall` case where a zero-sized
GNU aggregate broke the EAX/EDX/ECX chain instead of consuming nothing.  With the
adopted lowering fix the same gate reports `empty-aggregate ABI: 20 cross-compiler
executions passed`, both `-m64` and `-m32`, at `-O0 -O1 -O2 -O3 -Os`, in both
caller/callee directions, and `aggregate-call-abi` (bounded `REP MOVSB`, object-size
copies against a source ending on a guard page) passes in 29 s.  So #781 is not
infrastructure-only: it fixes a live i686 miscompile that main still has.

Still open in #781, deliberately not silently shipped: its `scripts/x86_torture_
contracts.py` companion tests a rewritten `x86_gcc_torture.py` (58 lines main has
that the PR deletes), and the `sema` half (219 insertions / 92 deletions in
`analysis.rs`) replaces inline `CType::Function` construction with the new
`call_type()` helper -- both auto-merged here and both covered by the regression
corpus, but they are the parts a reviewer should read first, since they are the
only places where the PR predates main's own edits to the same files.


## 5. Gate record

Measured at the head of `arena/round10-gate-hardening`, i.e. the commits between
`106b7e35` and `HEAD` (`git log --oneline 106b7e35..HEAD | wc -l`), which are: the
follow-up fixes (dispatcher region latch, ELF half-NOP docs, typed alignment-tail
verdicts, tail grammar tests, nested-billing law, prose corrections), the NEON
three-same correctness fix with its census gate, the rebased #781, the CI
provisioning fix, and the documentation commits.  No commit count is quoted here on
purpose -- the last attempt at that went stale the moment the sentence explaining it
was committed, which is the same drift item 7 of the brief was about.  The
`--fast` pass stamp in `target/ci_local.pass` names the exact tree it was taken
at, which is the pairing `lccc-snapshot.sh` refuses to deliver without:

| gate | result |
|---|---|
| `scripts/ci_local.sh --fast` | **175 passed, 0 failed, 5 skipped -- ALL GATES GREEN** (the five skips are the slow gates GitHub runs per PR: `regression-corpus-ssa`, `benchmark-output-oracle`, `peephole-whitespace-invariance`, `cargo-test-debug-assertions`, `regression-corpus-debug-assertions`) |
| `cargo-test` (fastbuild, `--all-targets -j2`) | PASS, 501 s |
| `rustfmt` | PASS, 6 s |
| `clippy -D warnings` | PASS, 124 s |
| `codegen-quality-gate` | PASS -- "all golden workloads within tolerance" |
| `empty-aggregate-abi`, `aggregate-call-abi` | PASS (20 cross-compiler executions; 29 s) |
| `check_ci_gate_parity.py` | PASS -- the two new gates are mirrored into `ci.yml`, and the new `apt-get` step is not a gate command, so the mirror stays exact |
| `check_doc_links.py` | PASS after the pointer note in `FOLLOWUP-2026-10-10-pr785-assembler-iv-widen.md` |
| lib unit tests, targeted | `gas247_align_tail_grammar`, `gas247_riscv_align_tail_verdicts_at_the_parser`, `nested_macro_output_is_billed_twice`, `integer_three_same_leaves_require_agreeing_operand_shapes`, `three_same_leaves_all_agree_check`, `dispatcher_goldens_ratchet_one_row_per_arm` (526 arms, bounded region) all pass individually and inside the 4 265-test suite |

The tests written for this round found three of their own bugs before they found
anything else: the tail-grammar test assumed `rest` began with the fill (it begins
*after* the first comma, so `.p2align 4,3,8` arrives as `"3,8"`), the riscv
expectations ignored the parser's `Line N: … : '<source>'` wrapper, and the census
gate inherited its neighbour's body because it split the file on `pub(crate) fn`
only.  All three were real mistakes in the test, not in the code, and each was
caught by running the test rather than by reading it.

Delivery: this branch is `arena/round10-gate-hardening` at `ms178:lccc`, and the
canonical patch is `git diff 106b7e35`, whose commits are itemised at the head of
this section.  #781's authorship is preserved via the cherry-pick trailer, so the ABI
work keeps its own commit rather than being folded into mine.
