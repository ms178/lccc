# TASK-LK-19 — IFUNC end-to-end

IDs: LK-19 · Priority: **P0** · Base: f657de55 · Gate for the ms178
PKGBUILD's multi-arch glibc build (string-function dispatch performance).

## Objective

Support GNU IFUNC from source to dynamic linking:
1. `__attribute__((ifunc))` accepted in sema, lowered to the right symbol
   type;
2. `.type sym, %gnu_indirect_function` emitted by the assembler;
3. `R_X86_64_IRELATIVE` relocations for data words in exec AND shared
   links;
4. ld.so-side resolution semantics (call the resolver at startup, patch
   the GOT).

Probe result recorded in the glibc campaign: `.quad ifunc-sym` produced no
IRELATIVE and the attribute produced no IFUNC symbol — both halves are
missing.

## Files

`src/frontend/sema/analysis.rs` (attribute), `src/ir/lowering/`
(irelative symbol class), `src/backend/x86/assembler/` (`.type` directive),
`src/backend/linker_common/` + `emit_exec`/`emit_shared` (IRELATIVE), ld.so
semantics in the test runtime.

## Acceptance

- A minimal `static void *resolve(void) {...} void f(void) __attribute__((ifunc("resolve")));`
  probe yields an IFUNC symbol + one `R_X86_64_IRELATIVE`, and the resolved
  target runs.
- glibc configure accepts `--enable-multi-arch` with `CC=lccc`; the
  capability gate may then be dropped from the pinned recipe.
- `readelf -r` parity vs GNU ld on the probe.

## Validation battery

linker test suite (`tests/linker/run_linker_tests.py`) · new exec+shared
IRELATIVE tests · glibc configure + compile phase with multi-arch ·
`make check` spot check of `string/` tests.

## Do not

- Do not weaken the `--disable-multi-arch` capability gate before the
  end-to-end probe passes (pinned recipe, DECISIONS.md process section).
- Do not emit IRELATIVE for pre-relocated/statically-known symbols without
  the GNU-ld parity check.
