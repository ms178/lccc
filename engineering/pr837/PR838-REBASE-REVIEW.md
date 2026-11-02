# Post-PR836 rebase: execution and open contracts (2026-10-15)

This is a **separate follow-up**, not a rewrite of PR834/PR837. Base:
`2f477ce01749824ccab73ead7ccac90abd77fa08`. The validated, pre-rebase
PR837 S79 checkpoint is preserved separately. No result on that historical
source is relabelled as a result on this rebase.

## First frozen acceptance, RED

Frozen HEAD `4b526731457098c79c2af9923889a12c5797f81d`, tree
`ada56c2ac123a8f58b51f3eb1f80aad34bc8e52e`: `scripts/ci_local.sh
--fast` reported **258 PASS / 3 FAIL / 6 SKIP**. The failures were
`cargo-test`, `ci-gate-parity` and strict `clippy`. `rustfmt` and `msrv-check`
passed. The recorded HEAD/tree, six binary hashes and tool version lines match
before/after; the acceptance tree was not edited while the run was active.
Exact logs and identities:
`/home/user/artifacts/pr834-followup/pr838-frozen/{before.txt,after.txt,fast.log}`.

The Rust test failure was not a production TempFile fallback: two new RISC-V
assembler **test** cases from the PR837 follow-up still called the removed
`TempFile::new` after PR836 changed the API to fail-closed `try_new`. Strict
Clippy also builds tests, hence the same compilation error. Both negative and
positive witnesses now reserve paths with `try_new(...).expect(...)` and keep
the lease live until after checking the object. The targeted unit filter ran
both tests successfully (2/0); log: `pr838-riscv-test-repair.log` beside the
frozen logs. `ci-gate-parity` failed because the new ext-vector gate existed
locally but not on hosted CI; the hosted *test* job now runs the same gate and
requires i686 execution. The parity checker subsequently passed with 235
commands; log: `pr838-parity-repair.log`.

## Independent vector-width RED / repair / red-team

On the rebased baseline `2a2a5a09`, actual O0/O2 executables returned 2 for
m32 and 3 for m64, while Clang 23.1.4 returned 0 in all four comparable
cases (`pr838-a14-13/red.log`). The declaration path had the right scalar
width; compound literal/type-name construction had an LP64 guess and a
four-byte fallback. Target-specific scalar widths, typedef-chain resolution,
checked multiplication and invalid-element diagnostics replace that guess;
zero, negative and runtime-only element counts are diagnosed. i686 long-double
vector lane storage stays 16, distinct from the 12-byte scalar object.

The first repair passed the targeted m32/m64 × O0/O2 positive and five negative
arms (`pr838-a14-13/green-targeted.log`). Adversarial review then found the
*new* strict parser rejected a valid `ext_vector_type(sizeof(alias_long))`
count on both m32 and m64, which Clang accepted (`red-typedef-count.log`).
Reusing the existing typedef-aware constant evaluator fixed both declaration
and type-name count forms; the expanded targeted gate passed (`green-redteam.log`).
`tests/regression/check_ext_vector_size.sh` is part of local and hosted FAST.

**Still open:** `_Bool ext_vector_type(4)` stores 4 bytes in LCCC versus 1 in
Clang 23.1.4, and a three-lane `unsigned long` vector stores 12 bytes on
LCCC m32 versus Clang's padded 16. Logical lane count and physical storage
must be represented independently before rounding either path; this repair
**does not claim** complete vector semantics. `_Static_assert` accepted a
source whose runtime sizeof was wrong: check parser-time folding separately.
A14-01, A14-16, A14-35, wider RISC-V matrices, independent all45/worst15
comparisons, unresolved fannkuch regression, 14700KF native/PMU and the
20-PR operational cohort remain open. No native speedup claim is made.

## Gate status

The repaired tree has targeted checks and a clean syntax/format pass, but has
**not yet** completed a second whole FAST acceptance. Full/slow CI is reserved
for GitHub and was **not** run on this VM. A green result must include a new
frozen receipt for the exact final tree; this failed run must remain failed.
