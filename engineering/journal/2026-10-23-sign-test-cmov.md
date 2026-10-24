# 2026-10-23 — fold a sign-only conditional off SUB's flags

## Source and baseline

Base: `4ba567022b7d358837aa13be4eb999621871c9f9` (PR #941), refreshed from
`origin/main` before the work. The current codegen-round reports identify
scalar min/max/abs lowering as a remaining secondary gap. A small reproducer
from that family is in `tests/regression/sign_test_cmov.c`:

```c
int nonnegative_difference(int a, int b) {
    int difference = a - b;
    return difference < 0 ? 0 : difference;
}
```

At `-O2 -fwrapv`, baseline LCCC used `sub; test; mov $0; mov; cmovge; ret`
(6 actual instructions). GCC 16.2 used `sub; mov $0; cmovns; ret` (4 actual
instructions); Clang 23.1.3 and ICX 2026.2 used 5 actual instructions. The
TEST is redundant for this exact condition: TEST clears OF, so `ge` is `ns`
and `l` is `s`; SUB and TEST set the same result sign bit at equal widths.
This is bit-exact even for wrapping arithmetic, so it does not rely on the
compiler's signed-overflow assumptions and remains valid under `-fwrapv`.

## Change and legality boundary

`fold_arithmetic_test_sign_cmov` runs next to the existing arithmetic
self-test fold. It only considers equal-width q/l self-tests whose nearest
flag writer is an arithmetic producer with matching destination, then maps
only `cmovge -> cmovns` or `cmovl -> cmovs`. Any other flag reader/writer,
control-flow boundary, unknown instruction, mismatched width, or later flag
observer before a complete redefinition refuses the rewrite. `g`/`le` are not
rewritten: their predicates also depend on ZF. The exact output and source are
pinned by `check_sign_test_cmov.sh` and six unit tests covering both positive
forms, strict-positive refusal, mismatched widths, a later `pushfq` reader,
and read-only staging.

## Evidence

- `cargo test --profile fastbuild --locked -j2 arithmetic_test_sign_cmov_tests`:
  6 passed.
- `tests/regression/check_sign_test_cmov.sh` with GCC 16.2.0, Clang 23.1.3,
  and ICX 2026.2.0: output and exit status agree at `-O0`, `-O1`, `-O2`,
  `-O3`, and `-Os`, including `INT_MIN`/`INT_MAX` combinations under
  `-fwrapv`. The strict-positive negative control retains TEST; disabling the
  peephole restores TEST on the primary witness.
- `scripts/godbolt.py compare` with `-O2 -fwrapv`: LCCC 5, GCC 16.2 4, Clang
  channel 5, local ICX 2026.2 5 instructions (tool count includes `ret`). The
  CE Clang channel resolved to 23.1.0; exact Clang 23.1.3 evidence is from the
  local regression gate, not that moving channel.
- No runtime-speed claim is made: one static instruction removed is not a
  cycle measurement. Broader scalar min/max/abs shapes and runtime effect
  remain open.
- The local `arithmetic-test-sign-cmov` gate uses the existing hosted
  `check_self_test_after_arith.sh` entry point with `--sign-cmov-only`. The
  existing hosted invocation also runs this regression by default, so the
  workflow file stays byte-for-byte unchanged while gate parity is preserved.

The delivery receipt reports the final fast-CI, rustfmt, and strict Clippy
results after refresh/rebase; no full/slow CI is run.
