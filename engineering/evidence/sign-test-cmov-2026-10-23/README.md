# SUB/TEST sign-condition fold evidence (2026-10-23)

**Witness:** `tests/regression/sign_test_cmov.c`, compiled `-O2 -fwrapv`.
The LCCC emitter materialized `a - b < 0 ? 0 : a - b` as `sub; test; mov-zero;
mov-copy; cmov; ret` (6 actual instructions). The new x86 peephole removes `test`
and changes `cmovge`/`cmovl` predicates derived from TEST into `cmovns`/`cmovs`
respectively. This is a one-instruction reduction to 5 actual instructions at
the representative function; it does not eliminate the separate
zero-initializer copy.

## Oracle results

- The sign-CMOV section of `tests/regression/check_self_test_after_arith.sh`
  passed with the local, exact compilers: GCC 16.2.0, Clang 23.1.3, and ICX
  2026.2.0. The standalone helper is
  `tests/regression/check_sign_test_cmov.sh`; the existing hosted entry point
  invokes it by default, while the local `arithmetic-test-sign-cmov` gate
  selects it with `--sign-cmov-only`. At each of `-O0`,
  `-O1`, `-O2`, `-O3`, and `-Os`, the runtime fixture includes
  `INT_MIN`/`INT_MAX` combinations under `-fwrapv`; all outputs and statuses
  matched. The strict-positive `> 0` control retains `test` and its
  original condition. `CCC_PEEPHOLE_SKIP=self_test` is the mutation/sensitivity
  control and restores the `test` in the candidate assembly.
- `scripts/godbolt.py compare` ran for `nonnegative_difference`, `-O2 -fwrapv`,
  with GCC 16.2, the Clang channel, and local ICX 2026.2. The recorded channel
  resolved Clang to **23.1.0**, not the requested 23.1.3; treat that row only as
  a secondary current-channel sample. The exact Clang 23.1.3 result is the
  local differential/codegen gate above. The full compiler outputs and CE
  manifest are retained alongside this note.
- Godbolt function instruction counts (including `ret`): LCCC 5, GCC 16.2 4,
  Clang channel 23.1.0 5, ICX 2026.2 5. Local exact-version
  compiler outputs are `gcc16-local.s`, `clang23.1.3-local.s`, and
  `icx2026.2-local.s`; `lccc-after.s` and `lccc-self-test-disabled.s` show the
  enabled and disabled shapes. Only trailing whitespace and blank lines at EOF
  were normalized in the saved `.s` artifacts; instruction text is unchanged.

No runtime-speed claim is made from this static instruction change. Callgrind Ir
and instruction counts are not cycle measurements; target-hardware PMU/runtime
confirmation remains out of scope for this small peephole receipt.
