# PF-TLS-1 — Local-Exec TLS direct access and the `sym@MOD+N` assembler defect

Review baseline: `6f8ace9c0bf98bb2c4df671342d89f983e0ab4c1` (upstream main).

## Task

Lower legal Local-Exec `__thread` accesses to the one-instruction
`%fs:symbol@TPOFF(+N)` form, and fix the assembler defect that makes the
addend-carrying spelling unusable.

## Hypothesis

Two independent defects keep every TLS access at three instructions:

1. **Codegen.** lccc always materializes the Local-Exec base
   (`movq %fs:0,%r` + `leaq sym@TPOFF(%r)`) and then accesses through the
   register. GCC and Clang fold the whole access into one instruction with the
   segment override and the TPOFF relocation inside the memory operand.
2. **Assembler.** Even if codegen emitted the folded form, the internal
   assembler mis-parses `sym@TPOFF+8`: `parse_displacement` tries
   `symbol+offset` before it considers a relocation modifier, so the
   instruction is recorded as a plain `R_X86_64_32S` against a symbol literally
   named `sym@TPOFF` — a wrong address, silently, with no diagnostic.
   Verified directly:

   ```
   $ lccc -c tpoff.s && objdump -dr tpoff.o
   12: R_X86_64_32S   b@tpoff+0      # lccc (before)
   12: R_X86_64_TPOFF32 b           # GNU as 2.47
   ```

## Code locations

- `src/backend/x86/assembler/parser.rs` — `Displacement::SymbolModAddend`,
  `split_sym_modifier_addend`, ordering inside `parse_displacement`, and three
  parser unit tests.
- `src/backend/x86/assembler/encoder/core.rs` — `symbol_mod_reloc_type` (shared
  by the plain and addend forms), `symbol_disp_parts`, the RIP-relative arm,
  `mem_expr_echo`.
- `src/backend/x86/assembler/encoder/mod.rs` — `label_to_disp`.
- `src/backend/elf_writer_common.rs`, `src/backend/i686/assembler/encoder/core.rs`
  — relocation/dot-rewrite and i686 displacement arms for the new variant.
- `src/backend/x86/codegen/memory.rs` — `tls_local_exec_access`,
  `tls_local_exec_operand`, `tls_direct_type_ok`,
  `try_emit_tls_direct_{load,store}_impl`.
- `src/backend/generation.rs` — load/store address-fold ladder hooks.
- `src/backend/x86/codegen/emit.rs` — trait delegation plus the MachInst
  bail-out (the MachInst lowering would otherwise claim the access and emit the
  two-instruction base).
- `src/backend/traits.rs`, `src/backend/mod.rs`, `src/backend/state.rs`,
  `src/driver/pipeline.rs` — a new `shared_lib` flag, because `pic_mode`
  conflates `-fPIC` with `-shared` and the two have opposite legality for
  Local-Exec.
- `src/backend/x86/codegen/globals.rs` — TLS model selection for shared output.

## Legality rule (the part that must not be guessed)

Local-Exec is a link-time-constant offset, so it is legal in every executable
(PIE included) and **illegal in a shared object**. GCC's observed behaviour,
measured on this host:

| flags | GCC form |
|---|---|
| (default) | `movq %rdi, %fs:a@tpoff` (Local-Exec) |
| `-fPIE` | `movq %rdi, %fs:a@tpoff` (Local-Exec) |
| `-fPIC` | `leaq a@tlsld(%rip)` (General-Dynamic) |
| `-fPIC -shared` | `leaq a@tlsld(%rip)` (General-Dynamic) |
| extern TLS, non-PIC | `movq e@gottpoff(%rip)` (Initial-Exec) |

lccc's own rule therefore is:

* fold only when **not** producing a shared object;
* fold only for a symbol this module owns (`local_symbols`), matching GCC's
  Initial-Exec choice for external TLS symbols;
* never fold at `-O0`, where phi elimination leaves non-SSA identities and
  `get_defining_instruction` may name a definition that does not reach the
  access (the same restriction the accumulator-address fast path carries).

## Pre-existing defect fixed on the way

`lccc -fPIC -shared` on any `static __thread` variable failed to **link**:

```
ccc: error: relocation R_X86_64_TPOFF32 against 'ul' can not be used
     when making a shared object; recompile with -fPIC
```

Reproduced on the review baseline (`6f8ace9c`), i.e. not introduced here.
lccc emitted Local-Exec for local TLS symbols under PIC; the linker rejects it
for `ET_DYN`. Shared output now routes local TLS symbols through Initial-Exec
(`@GOTTPOFF`), which is valid there, and the gate test proves the result
round-trips through `dlopen`.

## Measurements

Host: 2-vCPU Xeon KVM sandbox, 1.9 GiB RAM, 6 GiB swap, `-j2`, fastbuild
(`-O1`) compiler build. Static counts at `-O2`, `ra_quality_census`
methodology (`.type`-delimited function bodies).

| Function (`.size`-delimited body) | lccc before | lccc after | gcc | clang |
|---|---:|---:|---:|---:|
| `set_all` (new regression test) | 54 | **36** | — (inlined) | 39 |
| `sum_all` (new regression test) | 41 | **28** | 19 | 20 |
| `tls_pass` (`tls_seg_access` benchmark) | 36 | **32** | 20 | 31 |
| `main` (`tls_seg_access`) | 24 | 24 | 17 | 21 |
| `main` (`tls_local_exec_direct`) | 28 | **27** | 14 | 17 |
| **total, 4 functions above** | **129** | **111** | 70 | 89 |
| whole TU, `tls_local_exec_direct.c` | 123 | **91** | 50 | 56 |
| `set` (3 TLS stores, micro test) | 12 | **6** | 6 | 6 |

Rows 1-6 are `scripts/ra_quality_census.py --include-main` (the repository's
standard per-function counting, `.type`/`.size` delimited); the whole-TU row
counts every instruction line of the emitted `.s`.

`%fs:`-operand forms: `movq`, `movl`, `movw`, `movb`, and the addend-carrying
`%fs:arr@TPOFF+8/16/24` all appear at `-O1..-O3/-Os`.

Assembler: for a hand-written `movq %rax, %fs:b@tpoff+0` lccc now emits exactly
the relocation GNU as 2.47 emits (`R_X86_64_TPOFF32` against `b`), verified by
diffing `objdump -dr` output of both assemblers (identical relocation lines).

### Runtime — NOT established

The scaled benchmark (`-DPASSES=20000000U`, ~0.8 s/arm, 7 interleaved rounds,
output identical across gcc/before/after) shows per-binary spreads of
0.67 s–0.94 s. The before/after medians (≈0.87 s vs ≈0.84 s) are inside that
spread, so **no runtime claim is made**: per the repository's INF-HARNESS-1
rule this is reported as a static win only, and it remains
**UNVERIFIED ON TARGET** (i7-14700KF).

## Correctness

* `tests/regression/tls_local_exec_direct.c` + `.flags`: GCC output/exit parity
  over `-O0 -O1 -O2 -O3 -Os` × {default, `-fno-pic`, `-fPIE`, `-fPIC`}, every
  width the fold supports, plus constant-offset TLS array slots and a
  pointer-typed TLS slot read back through the stored address.
* `tests/regression/check_tls_model_selection.sh` (`ci_local` gate
  `tls-model-selection`, mirrored in `.github/workflows/ci.yml`): the parity
  matrix, the `-fPIC -shared` link + `dlopen` round-trip, and a
  mechanism-fires assertion (`-O2` must contain `%fs:…@TPOFF`; `-shared` must
  not).
* Full regression corpus: **841 passed, 0 failed** (13 GCC-uncompilable
  skip-compare).
* Existing TLS gates still green: `globals_tls`, `tls_local_pic`,
  `check_i686_tls_ie_relax.sh`, and the `tls_seg_access` benchmark output is
  byte-identical to GCC's.
* Three new assembler parser unit tests pin the addend form, the bare form, and
  the non-modifier `@tail` case.

## Defect found and fixed during validation

The first fold implementation used `reg_for_type(phys_reg_name(reg), ty)` for
the register-homed store. That helper only knows rax/rcx/rdx/rdi/rsi/r8/r9 and
answers `"rax"` for every other register, which produced
`movl %rax, %fs:ui@TPOFF` for a value homed in `%r11` — silently storing the
wrong register. lccc's assembler rejected the width mismatch loudly, and the
fold now uses `phys_reg_name_32` / `typed_phys_reg_name`. This is recorded
because the same helper is used elsewhere and the failure mode is silent
wrong-register selection, not a diagnostic.

## Decision

IMPLEMENTED. Static wins as tabulated; runtime unverified on this host and
unverified on target. Follow-up: `%gs:` (kernel/x86-32) coverage, and re-measure
`tls_seg_access` on the i7-14700KF with ≥200 ms arms.
