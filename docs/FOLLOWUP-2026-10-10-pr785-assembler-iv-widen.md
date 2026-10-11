# PR #785 — review adjudication, fixes, and measurements (2026-10-10)

> **Status (2026-10-11): merged.** This note covers the review of PR #785, which was
> redelivered as #788 and landed on main as `106b7e35`.  The follow-up brief that
> the merge produced -- nine items, all measured, plus the reading of #781/#777/
> #776/#725 that the same audit asked for -- continues in
> [`FOLLOWUP-2026-10-11-review-followups-and-open-prs.md`](FOLLOWUP-2026-10-11-review-followups-and-open-prs.md).

Base: `ms178/lccc` main = `1334bb8a` (merge of PR #782). Verified against the
GitHub API at the time of writing, i.e. the branch already sits on the
newest upstream commit — there was nothing to rebase onto, and that claim is
backed by `git rev-parse origin/main` rather than memory.

Head: `05e95bea`, four repair commits on top of the restore commit `dccb3d98`,
delivered as one squashed patch `git diff 1334bb8a` (180 018 B).

## 0. Why there is a "restore commit" in this stack

The sandbox was wiped twice during this round (`/var/tmp`, `/tmp`,
`~/.cache` are all outside the persisted snapshot). The second wipe hit after
§1/§3/§5/§6/§10 were committed but *while* §4 and §7 were still uncommitted in
the working tree, so those edits were lost and re-written. Recovery used the
canonical artifacts:

```
git clone https://github.com/ms178/lccc && git checkout -b arena/round7-repair 1334bb8a
git apply /home/user/ms178-1.patch        # 123 367 B, diff of the pre-wipe head
```

The five pre-wipe commits (`3c35c467`, `6cd47cdc`, `a4a00254`, `7a831384`,
`51be8a52`) are therefore folded into `dccb3d98` with their conclusions
restated in its message; the boundaries of the *new* work are intact. The
restore is not trusted on sight: every gate below was re-run on the restored
tree; the restore-time gate results are recorded in the delivery artifacts
ledger shipped beside the patch (outside the repository, so it is named here
rather than linked).

Harness setup that the measurements depend on (the user's standing rule, and
the reason a 40 M-line macro file can be measured at all):

```
sudo fallocate -l 8G /var/swapfile && sudo chmod 600 /var/swapfile
sudo mkswap /var/swapfile && sudo swapon /var/swapfile      # 8 388 604 kB active
cargo build --profile fastbuild -j2                          # rustc -O1, 2 jobs
```

## 1. How to read the verdicts

The Review AI that produced the 13-section list cannot execute anything. Each
claim was therefore treated as a hypothesis to be measured, not as a work
order. "OVERSTATED" below means the mechanism is real but the claimed
consequence is not; "REFUTED" means the code does not have the property
described. Two of its sections also name files the round-5 patch never
touched, which is worth knowing when judging precision.

| § | claim | verdict | what the measurement showed |
|---|-------|---------|------------------------------|
| 1 | iv_widen casts/dominance/step law unsound | **AGREE, extended** | all defects latent (IR-contract level); added dominance requirement on the guard; 8 C-shape hunts matched gcc exactly |
| 2 | ICF/COMDAT groups mishandled | **REFUTED** | real groups: `--icf=safe/all` produce identical 3 552 B from 3 640 B, runtime 1008/1120, FDEs 5/4/3 |
| 3 | pushf_elim treats any dot-line as flag-neutral | **AGREE** | measured 97/97 in-window corpus directives are `.cfi_*`, so no C miscompile exists — but `.byte 0xf8` *is* `clc`; contract fixed, 3 of 5 new tests fail pre-fix |
| 4 | `.macro` growth unbounded | **AGREE** | 40 M expanded lines: 623 s then SIGKILL, silent → now 2.0 s + a diagnostic at +0.02 % Ir |
| 5 | `.p2align`/`.balign` fill and max-cap mismatch | **AGREE** (fixed earlier) | 600-cell census: aarch64 300/300 identical; riscv64 198 + 51 pre-relaxation + 51 sub-floor, 0 divergent |
| 6 | NEON compare-against-zero / pmull arity | **AGREE partly** | gate widened to shape-only + `Symbol`; `pmull` arity: agreed because lccc is user-facing; the audit's `starts_with('s')` reasoning was wrong |
| 7 | dispatcher goldens cover only a fraction | **AGREE, numbers wrong** | its "574 arms / 151 covered" is wrong both ways; truth: 526 arms, 423 covered → 103 alias names with no golden at all |
| 8.1 | wrap_ring_sweep exit status + hardcoded artifact | **CONFIRMED** | exit 0 on `DIVERGENT(21)`; verdict path collided across runs |
| 8.2 | skipped gates | **AGREE, no action** | every SKIP is documented and conditional on a missing host tool |
| 9 | doccheck `apply_bitdiffs` drops valid forms | **AGREE on the defect, DISAGREE on impact** | 3 valid forms unread → `variants=0`; but dropping a variant makes the checker *stricter*, so no wrong word could pass silently |
| 10 | FMOV imm8 table | **AGREE (doc only)** | re-derived all 8 rows from GAS: `24.0` is `0x38`, the table said `0x70` (which duplicates 1.0). Encoder was already correct |
| 11–13 | perf backlog items | **no change** | remain backlogged on measured grounds (see `LCCC-FOLLOWUP-2026-10-09-round5.md`); nothing in the audit contradicts the earlier numbers |

## 2. §7 — the ratchet certified a hole

`dispatcher_arms()` in `scripts/gen_aarch64_dispatcher_goldens.py` and the Rust
test `dispatcher_goldens_ratchet_one_row_per_arm` both enumerated arm heads
with a per-line regex, so a multi-pattern head contributed only its first key.
423 of 526 arms. The 103 dropped names are the LSE atomics
(`cas* casp* swp* ldadd* ldclr* ldeor* ldset* ldsmax* ldsmin* ldumax* ldumin*
stadd* stclr* steor* stset* stsmax* stsmin* stumax* stumin*`), `crc32*`,
`ldrsw`/`ldrw`, and the rounding `fcvt*` group.

That set is the worst possible place to be blind: those arms forward the
mnemonic string to a shared leaf which re-reads the decoration — `b`/`h` for
element size and register width, `a`/`l`/`al` for memory order, and store forms
take `Rt, [Rn]` where the load forms take `Rt, Rt2, [Rn]`.

Fix: heads are recognised by the dispatcher's own indentation (which is what
keeps a leaf's `if mnemonic == "..."` from being read as an arm), all keys on
the head line and its `|` continuation lines are collected, and the rule is
*latched*: the generator writes `# arms: 526` into the TSV and the Rust test
compares its own re-derivation to it, so the two copies of the rule — which
cannot be shared, the test must run without cross tools — fail loudly if they
drift.

Spelling coverage came from a new `alias_family_templates()` that derives the
operand shape from the mnemonic and lets GNU as decide: the generator submits
each candidate to GAS 2.47 first and keeps only what the oracle accepts. My
first version got this wrong in a way worth recording: it emitted
`staddb w0, w1` (three-operand load shape) and `crc32h w0, x1, w0`, GAS
rejected them, and those arms stayed uncovered — the generator's `UNCOVERED`
line and its nonzero exit are what told me. Result after the fix: **526/526
arms with a GAS-verified golden, generator exit 0**, `doccheck 526/526` over
the new file, corpus 79 599 words.

Incidental finding, not part of §7 and left open deliberately: the generator's
diagnostic list contains ~3 580 rows where **LCCC rejects what GAS accepts**
for `adrp x0, 0x…` (an absolute, page-aligned constant with no symbol). That is
a feature-laxity gap of the same family as F8, it needs a decision about
whether `lccc` should resolve absolute pages without a link step, and it is
recorded here rather than fixed silently.

## 3. §4 — macro expansion now has the budget `.rept` already had

`.rept`/`.irp`/`.irpc` predict their output from a literal count and refuse
before emitting; the `.macro` expander charged only recursion depth (101 live
frames, the GAS 2.47 wall). A shared `ExpansionTotals` is now threaded through
`expand_macros_at`/`expand_macros_with` (riscv-facing) and the arm's
`expand_macros_impl` (which carries GAS's `\@` counter), charged once per
invocation through the existing `check_expansion_growth`, so a macro and a
`.rept` of the same size cost the same and neither is a way around the other.
Threading rather than per-frame counters is the point: 101 frames each granted
4 Mi lines is 400 Mi.

Measured (fastbuild `lccc-arm`, fresh process per run so `ru_maxrss` is not
inherited from a sibling; 200-line `.macro B` invoked N times):

| input | before | after |
|-------|--------|-------|
| N=200 000 → 40 M lines | 623.2 s, 1 266 MiB, SIGKILL, silent | 2.0 s, 245 MiB, rc=1, `.macro: expansion exceeds the 4194304-line limit` |
| N=20 000 → 4.0 M lines (legal) | 8.1 s / 1 735 MiB | 8.2 s / 1 533 MiB, rc=0 |

Callgrind A/B on the most macro-heavy shape in the tree (2 000 × 200 = 400 k
expanded lines): `Ir` 5 880 162 861 → 5 881 330 899, **ratio 1.00020**. These
are simulated instruction counts under Callgrind's model, not Raptor Lake
cycles or PMU counters; they say the accounting is free at the granularity that
matters, and the wall/RSS pair above is the same story at the process level.
For comparison GAS 2.47 streams the 40 M-line file in 0.9 s because it never
materializes the expansion; lccc materializes it, which is what makes
macro-expanded lines addressable downstream and therefore what has to be
bounded.

Adding the accumulator exposed a duplication worth more attention than the fix
itself: x86 already had the identical arithmetic in a private
`ExpansionBudget::charge` (saturate `expansion_size`, compare against
`check_expansion_growth`), while arm and riscv had nothing.  The real defect the
audit pointed at was not a missing line of accounting, it was that each front end
decided for itself what an expansion may cost.  So `ExpansionBudget` now
*delegates* to `asm_preprocess::ExpansionTotals` and keeps only what is genuinely
x86's own — its `depth` frame counter — and `MAX_MACRO_FRAMES = 101` was checked
against the wall measured on GAS 2.47 for the other two targets: the three front
ends share both ceilings from one definition of each (follow-up commit `72deffaa`).
x86's side of that contract is pinned by `gas247_rept_budget_law_x86`, which
still passes unchanged, i.e. the delegation is behaviour-preserving by test and
not by inspection.

Tests: charges accumulate across frames and a charge legal alone is refused
once the run passes the ceiling; `u64` saturation (a resource question must not
become a panic) with the byte-before-line ceiling precedence pinned; legal
work unaffected (101-frame chain, 100 k lines); a self-doubling macro answers
`macros nested too deeply` instead of materializing. One of these failed on
first run and the *code* was right: `check_expansion_growth` consults bytes
before lines, and my assertion had guessed the other order — a good reminder
that a new test is evidence about the test, not about the fix.

## 4. §8.1 — a sweep that cannot fail is not a gate

`wrap_ring_sweep.py`'s `main()` returned nothing, so `DIVERGENT(21)` exited 0,
and the verdict file went to a fixed `$HOME` path selected only by `--opaque`,
where a stale artifact reads like a fresh one. Now: 0 = clean, 1 = any
divergence, 1 = unusable *reference* (agreeing with a crashed compiler is not a
verdict — that was the silent-pass direction), 1 = no cell ran, 2 = usage.
The artifact defaults to the per-run work directory and opens with
`# mode=… compilers=… opts=… cases=… cells=… divergent=… unusable_reference=…`.
Positional compiler names still work, so the documented self-reference
(`wrap_ring_sweep.py gcc`) is unchanged; `--opts` without `=` no longer raises
`IndexError`.

Proven by driving the real `main()` with `build_and_run` substituted
(agree → 0, one divergent compiler → 1, broken reference → 1, no compilers →
2, `--bogus=x` → 2), and re-run for real: 39 shapes × {-O0, -O2} against the
reference compiler, **78/78 cells agree, exit 0**. Writing that self-test also
caught a regression I had just introduced — my new argument loop classified the
positional `gcc` as an unknown flag, which would have broken the very
invocation the docstring recommends.

## 5. §9 — the doccheck's silent zero

`apply_bitdiffs` returned `None` for shapes it could not parse and
`encoding_variants` dropped them, so three real M.d forms were checked against
nothing while the run printed a satisfied count:

```
!(L == 0 && imm3 == 011 && Rt == 11111)          hinte.xml
!(op1 == 000 && op2 IN {00x, 010})  (twice)      msr_imm.xml
```

Root cause: `spec.split("&&")` cut through the negation. The reject model is
now `rejects → pattern → term → (mask, value)` alternatives — OR of ANDs of ORs
— because the naive reading ("treat `!(A && B && C)` as three `!=`s") is not
merely imprecise, it over-rejects: a `HINT #1`-shaped word satisfies two of the
three terms and would have been reported as illegal. `IN {00x, 010}` is one
*term* with alternatives, each carrying its own mask.

Every refusal now appends a reason to a caller-supplied list; `main()` prints
the count and **fails the run** if any bitdiff on a golden's candidate paths
could not be read, including rows that "validated" through another variant of
the same iclass (that is exactly the case where a skipped encoding would look
like a pass). New `--selftest` pins the parser with no XML and no cross tools —
the two real forms, `f != lit`, the field-width rule, and that each refusal
names the spec. The selftest found a live bug in my own replacement: an
`||`-bearing spec made `_split_top` return `[]`, and the loop never ran, so the
encoding came back as an *unconstrained* variant matching every word of the
iclass. The original code was fail-closed there by accident; the fix refuses
explicitly before the loop.

Measured: `--selftest` 0 failures; full run **526/526 golden words validated**,
0 divergences, 0 unread bitdiffs, exit 0.

## 6. Gates

`ci_local.sh --fast` was run twice on this head. The first run came back
171/2: `gvn-cross-block-mul-dot` failed in 0 s inside `mktemp` (a gate script
that assumed `$TMPDIR/tmp` exists — fixed, see below), and `doc-link-integrity`
correctly caught this document naming a file that lives outside the repository.
After both fixes: **173 passed / 0 failed / 5 skipped**. The 5 SKIPs are the
documented host-tool conditionals (clang/lld/mold-style steps and the
slow-corpus gates `--fast` exists to skip); none of them is a gate this round
touched. Two of them being *my own* tooling failures is worth stating plainly:
a gate whose failure mode is environmental is worse than no gate, because it
teaches everyone to re-run until green.

| gate | result |
|------|--------|
| `cargo test --profile fastbuild --lib` | **4251 passed / 0 failed / 7 ignored** |
| `cargo clippy --profile fastbuild --all-targets -- -D warnings` | clean |
| `cargo fmt --check` | clean |
| dispatcher goldens (GAS 2.47.20260726) | 526/526 arms, generator exit 0 |
| `scripts/aarch64_doccheck_2026_09.py` | 526/526, 0 divergences, 0 unread bitdiffs |
| `scripts/aarch64_doccheck_2026_09.py --selftest` | 0 failures |
| `scripts/wrap_ring_sweep.py` (reference self-sweep, −O0/−O2) | 78/78 agree, exit 0 |
| `scripts/check_ci_gate_parity.py` | PASS (149 commands, 19 invocation contracts) |
| `scripts/ci_local.sh --fast` | **173 passed / 0 failed / 5 skipped — ALL GATES GREEN**, twice: once at `1f9c41b6`, then re-run at the delivered tree so the stamp's `tree` hash equals `git write-tree` of the head (847 PASS lines across the two runs) |
| `bash scripts/run_regression_suite.sh` | **PASS=847 / FAIL=0 / SKIP=8, AB-diff failures 0**, exit 0 (the boot gate skips without `KERNEL_DIR`, as documented) |

The full GitHub-only CI was not run, per instruction.

## 7. Open items, deliberately open

* `adrp`/`adr` reject an absolute page constant that GAS accepts (~3 580 corpus
  rows in the generator's diagnostic list). Needs a policy decision on
  symbol-free absolute addressing, not a patch.
* `fcmeq v0.4h/#0.0`: lccc encodes, GAS rejects under the default `-march`
  (needs an FPHP/`+fp16` gate) — same march-extension laxity family as F8.
* `fcmeq s0, s1, s2`: GAS routes it to the FP-compare class (`0x5e22e420`);
  pinned as an `is_err()` test naming the oracle word rather than silently
  emitting the wrong class.
* Blanket exact-arity conversion of the ~401 dispatcher arms was left out of §6:
  it is an operand-table architecture change, not a fix, and shipping it inside a
  repair branch would mix a redesign with correctness work.
* Generated-code perf backlog (PERF-10/-8/-9/-7/-5/-6/-4) unchanged from round
  5 (docs/FOLLOWUP-2026-10-09-ivwiden-theorem5.md); nothing in this round
  touches emitted code, which is why the Callgrind pair
  above is scoped to the macro path instead of re-running the corpus geomean.

---

# Follow-up adjudication (2026-10-11): red CI at `d6f7591d`, and the findings on it

PR #785 (`d6f7591d`) came back red from CI at
`Run 38054218876 / step 86 -- "AArch64 spelling differ and system-register
table"`.  A second Review AI pass over the same head produced findings F1-F11;
each is adjudicated below with the measurement that decided it.  Two of them
were right, one of those two was a miscompile I had introduced myself, and the
CI red was neither of our doing.

## The red CI was a gate-input bug on main, and the fix makes the gate stronger

`scripts/aarch64_sysreg_table.py` could read `opcodes/aarch64-sys-regs.def` only
from the binutils **tarball**, while `scripts/ensure_gas_247.sh` deliberately
returns before its download step when the tool prefix is already cached -- which
on CI it is, because `ci.yml` caches exactly those prefixes (added in PR #782).
Warm runner: no tarball, gate dies with exit 2, all nine later gates skipped.
Cold runner: passes.  `a9410968` was green, `1334bb8a` red, main red again since;
#785 inherited it.

Fix (`aaf23ea0`): the provisioning step now **installs the .def next to the tools
it builds** (`$prefix/share/opcodes/aarch64-sys-regs.def`, fatal if the source
file is missing), so the actions cache carries it and the .def is provably the one
that assembler was built from; the gate resolves it through a documented chain
(`--def` -> `$LCCC_AARCH64_SYSREGS_DEF` -> the `--as` prefix -> the provisioner's
source tree -> the pinned tarball locations) and **digest-checks every source
against a new `SYSREGS_DEF_SHA256` pin** (`7e7a9b2e5abc...d290`, measured from the
member inside the tarball whose own sha matches `GAS_TARBALL_SHA256`).  Editing the
script changes the CI cache key, so warm caches rebuild once.  Verified against the
exact failure condition -- both `GAS_DL_DIR` and `GAS_CACHE` pointed at an empty
directory, prefix warm:

    sysreg gate: .def source = .../share/opcodes/aarch64-sys-regs.def (assembler prefix), sha256 7e7a9b2e5abc... (pinned)
    aarch64_sysreg_table: 1619 registers agree with GNU as and target/fastbuild/lccc   [exit 0]

The general law, which is what makes this more than a path hack: **a gate must
resolve its inputs from the artifact it is testing, not from a side cache whose
contents depend on cache warmth.**

## F1 (agree, and it was my regression): the emitted wide step is now the proved delta

`normalize_step_const` returned `(signed, IrConst::I64(reduced))` -- the proof read
`.0`, the emitter read `.1`.  For a narrow *signed* type with a negative step those
differ, and measuring the shapes through `widen_ivs_in_function` showed the
consequence is not theoretical:

    u32 add 0xFFFFFFFF ugt : n=0 (declined)      i32 add -1 sgt : n=1  Add I64(4294967295)
    i32 add -2 sgt         : n=1  Add I64(4294967294)                exit test: Cmp Sgt phi, I64(8) ty=I64

Widening retypes the exit test into the wide domain, so a proven `-1` emitted as
`+4294967295` is a countdown replaced by an ascent that never reaches the bound.
The pre-§1 code was correct here (`widen_const` sign-extends), so the `dccb3d98`
commit introduced this while its own commit message claimed the two sides were
single-sourced -- and the helper's doc comment described this exact failure mode as
the thing it existed to prevent.  Fixed at the source of truth
(`IrConst::I64(signed as i64)`): unsigned rings and 64-bit rings are bit-identical
to before, so the change is provably confined to narrow signed negative steps.
Tests: an exhaustive-value pin on the helper, an end-to-end pin that asserts `n==1`
*before* checking the step and the retyped compare, and a pin of the *refusal* of
the u32 wrap shape -- the test it replaces said `if n == 0 { return; }` and so
executed no assertion at all.  Both new pins fail on the pre-fix code
(`proof delta -1 vs emitted wide step I64(4294967295)`).

Not reproduced from C: every spelling of `for (int i = n; i > 0; i += -1)` I tried
(`i += -1`, `i = i + (-1)`, `i = i - (-1)`, an 820-sum reference from gcc) produced
identical correct output, i.e. the front end canonicalises to `Sub`.  The audit's
supporting citation (`arith_transform.rs:347`, MulSub -> MulAdd) names a file that
does not exist in this repository (`find src -name arith_transform.rs` empty, no
`MulSub` rewrite anywhere), so the C-level exposure is unquantified -- which is a
reason to fix the contract, not to downgrade it.

## F2 (agree, extended): FCMP `#0.0` accepts exact zeros only -- and the new corpus found more

`fp_zero_spelling` tested `value == 0.0` on an f64 parse; Rust flushes a subnormal
underflow to `+0.0`, so `#1e-400` was accepted and encoded as the zero field -- a
silent substitution of a different constant.  Measured against GAS 2.47 on
`fcmpe s0,<spelling>`: GAS accepts `#0.0 #0 #00.0 #.0 #+0.0 #0e0 #0e400 #0.0e9`
and refuses `#-0.0 #1.0 #zero (0.0) #0x0p+0` plus every nonzero magnitude
(`#1e-400`, `#4.9e-324`, `#1e-323`, `#0.00000000000000000000000000000001`) --
i.e. it distinguishes by the literal's mantissa, not by a rounded double.  The
predicate now requires an all-zero mantissa on the float path.

The fix is pinned where it can rot: `scripts/aarch64_legality_probe_differ.py` had
**no cross product at all** for the one FCMP operand whose legality is a spelling
question, so both lists above were only hand-measured.  It gained a 114-row family,
and that family immediately found a second, older divergence -- GAS rejects *every*
half-precision FCMP, including the register form, at its default architecture
("selected processor does not support"), while the encodings agree bit for bit once
`-march=armv8.2-a+fp16` is given (`fcmp h0,#0.0` = 0x1ee02008 both sides).  A row
that pins an oracle limitation as a divergence in this compiler is worse than no
row, so `gen()` now yields per-row GAS arguments and applies the flag only to
`fcmp`/`fcmpe` rows naming an `h` register -- no existing row's verdict moves.
Result: **411 -> 525 spellings, 0 unpinned disagreement(s), 4 pinned**.

## F4 (agree, in both directions): the directive law was wrong twice

`directive_emits_data` (the predicate three x86 peephole walks rest on) treated the
alignment family as blanket-benign, but GAS lets the fill byte be chosen:
`.p2align 4,0xf8` fills with `clc` and `.p2align 4,0xcc` with `int3` -- bytes a flag
or control-flow walk must not step over.  Alignment directives are now benign only
without an explicit fill operand (`.p2align 4,,10`, fill empty and `max` only,
stays benign; fields split on commas because `", "` and `",,"` must not look alike).
`vector_copy::benign_directive` had the same blanket prefix and now shares the one
fill rule; the two classifiers remain separate because they answer different
questions, and cross-reference instead of being wrongly unified.

In the opposite direction the old list enumerated fifteen `.cfi_*` names, so
`.cfi_adjust_cfa_offset` -- emitted in every prologue -- counted as data and blocked
folds for nothing.  `.cfi_` is prefix-matched now.  Cost of both halves, from the
in-tree census (636 TU/opt/arch combinations, directives strictly inside function
bodies): `.cfi_adjust_cfa_offset` 0, `.zero`/`.skip`/`.space`/`.fill` 0, `.section`
68 (jump-table islands in `.rodata`, which is also why "section switchers" moved out
of the benign set -- they emit nothing here but they end the run of code the window
was parsed from).  `.Ltmp3:`-style labels are now answered directly instead of
falling into the data bucket.  Three tests, 85 directive texts, both directions.

## F6 (agree): RISC-V reached the shared ceilings by inspection only

`src/backend/riscv/assembler/parser.rs` calls `asm_preprocess::expand_macros` and
`expand_rept_blocks` -- the code carrying the growth ceilings and the nesting wall --
but the file had no test module at all, so nothing proved the wiring was through
that code rather than around it (the failure mode is an assembler that eats memory on
a 41 KiB `.s`, not a wrong answer).  Added: legal `.macro`/`.rept`/`.irp` expansions
still produce the exact instruction stream, the line ceiling and GAS's
`excessive count 18446744073709551615 for REPT - ignored` wording surface as
`Err` through `parse_asm`, the 101-frame chain assembles while the 102nd and direct
recursion hit `macros nested too deeply`, and the `#`/`//` comment style plus `.if`
arms are pinned at the same entry point.

## F9 (agree): no machine paths in shipped tools

`scripts/gas_fill_census.py` defaulted its GAS prefix to a session scratch
directory and its work directory to the same; both now follow the convention the
provisioner and CI already use (`$HOME/.cache/gas-2.47-<triple>`,
`$TMPDIR`/`tempfile.gettempdir()`), with the actionable "run
scripts/ensure_gas_247.sh ..." error on the miss.  `gen_aarch64_dispatcher_goldens.py`
lost its `/home/user/...` literals the same way.  The remaining `lccc-session`
strings in `scripts/` are the snapshot format identifier
(`lccc-session-bundle-v1`) and its output filename -- not paths.

## F1/F7/F8/F10/F11 (agree): prose that outran the code

`widen_const`'s doc comment had been duplicated verbatim by an earlier splice (a
compiling, linting, useless second copy): removed, and the surviving copy now states
which question it answers (ring-preserving widening) versus the latch step's
question.  `normalize_step_const`'s doc claimed single-sourcing while its `.1` was
the reduced pattern and its `.0` was not; it now describes what the function does
and why congruence is insufficient.  Both are the class of defect where the comment
is right and the code is the other half of the sentence, which no compiler check can
see -- hence the pins above rather than a note.

## Gate inventory at the head of this note

Every gate below was run on the tree it documents; where a later commit changed
only this table's numbers, that is stated in its own entry.  Timings are from a 1 GiB-RAM/8 GiB-swap VM with `kswapd`
active and are therefore wall-clock indications, not benchmark results.

| gate | result |
|---|---|
| `cargo test --profile fastbuild --lib` | **4259 passed, 0 failed, 7 ignored** (4251 -> 4259: three iv_widen pins, three classifier tests, one FCMP spelling test, two riscv tests; the vacuous iv_widen test replaced) |
| `aarch64_legality_probe_differ.py` (GAS 2.47 prefix) | **525 spellings, 0 unpinned disagreement(s), 4 pinned** |
| `aarch64_sysreg_table.py --check`, warm-cache simulation | **1619 registers agree**, `.def` from `(assembler prefix)`, exit 0 |
| `scripts/check_doc_links.py` | **OK: all markdown references resolve** |
| `scripts/check_ci_gate_parity.py` | **PASS** (149 commands, 19 invocation contracts) |
| `scripts/ci_local.sh --fast` | **173 passed, 0 failed, 5 skipped -- ALL GATES GREEN**, incl. `rustfmt` PASS (10 s) and `clippy` PASS (209 s, `-D warnings`, the CI lint contract), `codegen-quality-gate` PASS, `codegen-gate-self-test` PASS; pass stamp `mode=fast` for tree `ab98e8470ac4d7e30f8f315b468746092a6c00de` |

## Open items (updated)

* Unchanged from the section above: adrp/adr absolute-page constants (~3580 diagnostic rows),
  `.8h`/`.16h` march laxity, `fcmeq s0,s1,s2`, blanket exact-arity for the ~401
  arms not yet pinned, perf backlog PERF-10/-8/-9/-7/-5/-6/-4.
* New: **the FP16/NEON feature-gate policy.**  lccc's AArch64 encoder accepts every
  architected encoding unconditionally, while GAS gates the half-precision ones on
  `+fp16` (and the SVE/MTE/etc. families on their own features).  For *generated*
  code that is only a difference if codegen emits an ungated instruction for a
  `-march` that lacks it; for the *assembler front end* it means we accept source
  that GNU as would refuse at the same baseline.  Deciding this needs a target
  feature model in the driver, which does not exist yet; the legality differ now
  papers over the specific row-by-row case rather than pretending the policy is
  settled.
* New: the census/sweep tools (`gas_fill_census.py`, `wrap_ring_sweep.py`) are not
  in CI.  The census is a one-off evidence producer with a 636-combination runtime;
  the sweep is `--fast`-unfriendly for the same reason.  If either starts guarding a
  law that can regress, it needs a reduced CI mode rather than an off-CI script.

## Verification at `c513c703` / `700b6c3d` (2026-10-11)

The provisioner hardening and the hygiene/coverage commit were checked with every
gate that can see them, including the two GAS-backed AArch64 gates that were the
original failure:

| gate | result |
|---|---|
| `cargo clippy --all-targets -j2 -- -D warnings` | **rc 0** (7 m 03 s, dev profile) |
| `cargo fmt --check` | rc 0 |
| `cargo test --profile fastbuild --lib` | **4260 passed, 0 failed, 7 ignored** (the new x86 delegation test; 4259 -> 4260) |
| `bash scripts/ensure_gas_247.sh --self-test` | **every case lands on its verdict**, incl. the five new gate-input cases (no `share/opcodes` -> REBUILD, empty dir -> REBUILD, `.def` present -> PROVISIONED, riscv64 -> PROVISIONED, absent prefix -> REBUILD) |
| `scripts/aarch64_sysreg_table.py --check` with `GAS_DL_DIR` and `GAS_CACHE` on an **empty** directory | `.def source = .../share/opcodes/aarch64-sys-regs.def (assembler prefix)`, `1619 registers agree with GNU as and target/fastbuild/lccc`, rc 0 |
| `scripts/aarch64_legality_probe_differ.py` | 525 spellings, 0 unpinned disagreement(s), 4 pinned, rc 0 |
| `check_doc_links.py`, `check_ci_gate_parity.py` | rc 0 (PASS, 149 commands) |
| `scripts/ci_local.sh --fast` | **173 passed, 0 failed, 5 skipped -- ALL GATES GREEN** |

Two process notes, both of which came out of running things rather than reading
code:

* `cargo clippy --all-targets -- -D warnings -j2` fails with `rc=101` and an error
  that reads like a compile failure: after `--`, `-j2` goes to `clippy-driver`/rustc,
  not to cargo.  The flag belongs before the separator.  A lint gate that reports
  "could not compile `lccc` (lib test)" for a job-control mistake is worth
  recognising on sight, so it is recorded here.
* The same battery's first version printed `CLIPPY_EXIT=0` for that failure because
  the exit code was taken after a pipe to `tail`.  Both this table and any future
  one use `${PIPESTATUS[0]}` (or a redirected file plus a direct `$?`) for exactly
  that reason: a gate that reports the wrong exit status is worse than no gate,
  and this is the third time in this note that a *measurement harness* bug -- not a
  compiler bug -- was the finding (the vacuous `n == 0` escape and the
  cache-dependent `.def` being the other two).

The ASAP artifact for main is `git am`-able as two commits
(`Fix main's red gate...` + `ensure_gas_247: ...`), verified on a detached worktree
at `1334bb8a`: `git apply --check` clean, both commits applied, the self-test green,
and the previously-failing step exits 0 with the download directories emptied.

## Two attribution corrections, made by checking rather than remembering

* The duplicated `widen_const` doc block that the follow-up review listed as
  finding F11 pre-dates this PR: `git show <main>:src/passes/iv_widen.rs` contains
  the five-line comment twice, exactly as the PR head does.  The review's absolution
  ("not this PR's fault, but it was in the area it touched") is therefore right, and
  an earlier note here implied the PR's editing had produced it.  The removal stands;
  only the attribution was wrong, and it matters because "we introduced it" and "we
  passed it by" imply different obligations for the rest of the file.
* The follow-up review's list of nine gates that "never ran on this PR" is accurate
  about GitHub, but the local run covers eight of them: `aarch64-operand-legality`,
  `aarch64-operand-legality-encoder`, `aarch64-arm-doccheck`, `aarch64-family-sweeps`,
  `asmdiff-semantic-validation`, `encdiff-semantic-validation`/`encdiff-corpus`,
  `kernel-tool-identity`, `kernel-relocs-tool`, the `linker-suite`/`linker-oracle-*`
  and fuzz-smoke gates all run inside `ci_local.sh --fast` and passed (173 gates).
  The one that does not is the debug-assertions regression corpus, which `--fast`
  skips by design; `ci_local.sh --slow` runs it, and per the owner's standing
  instruction the slow gates are GitHub's job, so they are the remaining debt this
  PR cannot pay locally and must pay by landing on a green main.
