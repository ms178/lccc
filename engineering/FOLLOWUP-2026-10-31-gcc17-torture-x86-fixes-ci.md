# 2026-10-31 — GCC 17-git x86 torture, two x86-64 codegen fixes, and the CI handoff

## Result

Two latent x86-64 miscompiles are fixed, each with a regression that fails on the
previous tree; the unwind model learned the encoding the second fix introduced;
and a revision-pinned GCC 17-git `gcc.c-torture/{execute,compile}` lane now runs
both x86 targets on main-branch changes, weekly, and on demand.

Baseline: `main` @ `219f74aeb7cf77659b7fd4f0e8e9dd03a3a5b38e`. The campaign
corpus is GCC trunk `c94b688a83fc84015c6fab8ab0ff142051598fc8`
(`gcc-17.0.0-git-c94b688a83fc84015c6fab8ab0ff142051598fc8`), provisioned by
`scripts/ensure_gcc_torture_git.py --ref c94b688a…`; `GCC17_TORTURE_REF`
reproduces an exact historical trunk revision rather than following master.

Correctness-first: no performance claim is made anywhere in this document. The
only instruction-count statements are the two local staging-path deltas measured
below.

## Full native matrix (`-O2`, two workers)

Execute legs link with the standalone `lccc-ld`; compile legs test assembly
production. Compile timeout 180 s, run timeout 10 s.

| Target / leg | Cases | Pass | LCCC compile-fail | LCCC run-fail | Reference compile-skip | Reference-fail | Unsupported |
|---|---:|---:|---:|---:|---:|---:|---:|
| x86-64 execute | 1,700 | 1,688 | 0 | 0 | 1 | 6 | 5 |
| i686 execute | 1,700 | 1,686 | 0 | 0 | 4 | 5 | 5 |
| x86-64 compile | 2,016 | 1,949 | 49 | n/a | 14 | 3 | 1 |
| i686 compile | 2,016 | 1,942 | 48 | n/a | 22 | 3 | 1 |

Reference failures are oracle failures, not candidate failures, and the reports
keep them separate: `pr118623.c` was confirmed by GDB 16.3 to abort inside `bar`
in Debian GCC 14.2's own `-O2` output, so it must not be counted here. The local
reference compiler for the campaign was Debian GCC 14.2; CI provisions GCC 16 and
verifies the oracle and multilib before either leg runs.

## Defect 1 — narrow integers crossing into the accumulator domain

LCCC lets a value's home hold the producer's representation. A signed 16-bit
producer may write with `movswl`, leaving bits 32..63 of the GPR zero while the
low 32 bits are canonical. Spill and reload are already width-consistent
(`value_to_reg_inner` reloads I8/I16 with `movsbq`/`movswq`), so the invariant
"narrow homes are 32-bit canonical" holds — until a 64-bit consumer reads the
whole register. Scalarized `i16` vector division does exactly that:

```asm
    movswl %ax, %eax     # producer: low 16 correct, upper half zero
    movq   %rax, %rcx    # 64-bit copy of a 32-bit-canonical value
    cqto
    idivq  %rcx          # 0x0000FFFF / -8 -> 0 rem 65535   (want 0 rem -1)
```

The `div64` fast path aggravates it: its "both operands non-negative" guard tests
bits 63..32, so a zero-extended negative value could take the *unsigned* 32-bit
`divl` path in a signed divide.

Fix: canonicalize at the two accumulator chokepoints (`operand_to_rax`,
`operand_to_rcx`) instead of at every producer. Eight edit points cover 148 call
sites across the 26 files of `src/backend/x86/codegen`; the register-cache
invariants (which reason about values, not widths) are untouched, and every path
for a type wider than 16 bits keeps its previous instruction stream byte for byte.

Measured, on `tests/regression/vector_sdiv_srem_short_signext.c` — an exhaustive
sweep of i8/i16 signed and u8/u16 unsigned lanes checked against the defining
properties of truncated division (`q*d + r == n`, `|r| < |d|`, remainder sign),
so the oracle never divides:

| Compiler | `-O2` result |
|---|---|
| `main` @ `219f74a` (pre-fix) | **67,673 failures / 1,840,896 checks** (exit 1) |
| this tree (post-fix) | 0 failures / 1,840,896 checks (exit 0) |
| GCC 16.2 `-O2` / `-O0` | 0 failures, identical stdout |
| Clang 23.1.4 `-O2` / `-O0` | 0 failures, identical stdout |

i686 is structurally immune and needs no twin fix: its division is 32-bit
`idivl`, which consumes exactly the 32-bit-canonical form the producers write.
Verified by inspection of `src/backend/i686/codegen/alu.rs`.

## Defect 2 — stack adjustment beyond signed imm32

`add/sub r64, imm` encodes only a sign-extended imm32, so a legal large automatic
object had no encoding for its frame delta. `emit_stack_adjust_x86` keeps the
one-µop immediate form for deltas that fit and otherwise materializes the
constant into caller-saved `%r11` and uses the register form.

Three encoding ranges, all exercised by `tests/regression/large_stack_frame_codegen.c`:

| Frame delta | Encoding | Size |
|---|---|---|
| `0x7ffffff8` | `subq $imm32, %rsp` | 7 B |
| `0x80000008` | zero-extending 32-bit `mov` into `%r11`, then `subq %r11, %rsp` | 6 + 4 B |
| `0x10000000000` | `movabs` into `%r11`, then `subq %r11, %rsp` | 10 + 4 B |

Measured: the pre-fix compiler emits `subq $1099511627784, %rsp`, which `gas`
rejects (`Error: operand type mismatch for 'sub'`, four sites); the current tree
assembles cleanly.

Scope boundary: frame *offset* synthesis beyond signed disp32 (`20031023-4.c`
reads `0x80000007(%rbp)`) is a different limitation with a different fix and is
not claimed here.

## Follow-on fix — the unwind model learns the materialization

The frame fix introduced a real defect in the unwind information, found by
red-teaming the patch rather than by any test that existed: `cfi_synth` derives
`%rsp` deltas from `add/sub $imm, %sp` only, so `subq %r11, %rsp` landed in its
"unmodelled stack pointer write" arm and the CFA was declared **absent** — not
merely imprecise.

```asm
# before (frame fix without the model fix)
    subq %r11, %rsp
    .cfi_undefined %rip          # unwinding through the function is gone
# after
    subq %r11, %rsp
    .cfi_def_cfa_offset 1099511627792
```

`materialized_stack_adjustments` resolves `mov $imm, %reg` + `add/sub %reg, %sp`
per line. The relation is deliberately local — the constant must be written by
the immediately preceding *instruction*, only CFI directives may intervene, never
a label (a branch target derives its state from its own edges) — and it is
register-*family* and width exact, because `movl $-1, %eax` leaves
`0x00000000FFFFFFFF` in `%rax` while `movq $-1, %rax` leaves all ones. Every shape
it must refuse (label, intervening instruction, different register, overwritten
register, 8/16-bit destination, direct `%sp` write) has a unit test.

`emit_stack_adjust_x86` also states the delta it applied (`# LCCC_STACK_ADJUST
<±n>`) so the model has an authoritative source instead of only an inference: the
marker is preferred, the textual `mov`+`add/sub` relation is the fallback, and a
disagreement between the two is treated as unknown rather than described wrongly.

Measured on `large_stack_frame_codegen.c`: `.cfi_undefined %rip` ×2 before,
×0 after, with exact `%rsp`-relative offsets in both the prologue and the
epilogue, and the CFA bytes now exact rather than truncated (above).

## Follow-on fix — staging cost

Two accumulator paths copied a narrow value and then extended the copy
(`movq %rcx, %rax`; `movswq %ax, %rax` — or `movq %rax, %rcx`; `movswq %cl, %rcx`).
Both now extend directly out of the origin register, which is one instruction
where the previous code was two, and never worse: assembled output shows
`movswq %r8w, %r11` for the register-home case. Wide types still take the plain
`movq` branch, so nothing else changed.

## Follow-on fix — narrow divides run at 32-bit width

A narrow value's canonical form is its 8/16-bit value extended to **32** bits, so
both operands of a narrow divide already fit in `%eax`/`%ecx`. `emit_div_narrow`
therefore uses the 32-bit divide:

| Form | Sequence | SKL latency / throughput |
|---|---|---|
| before (all narrow widths) | `cqto; idivq %rcx` | 42 / 21 (signed), 36 / 21 (unsigned) |
| after | `cltd; idivl %ecx` | 26 / 6, 22 / 6 |

The quotient cannot trap: the dividend is bounded by the narrow width, so
`INT32_MIN / -1` — the one `idiv` input that raises `#DE` independently of a zero
divisor — is unreachable. The 32-bit divide leaves `%rax`/`%rdx` in the narrow
type's canonical form (a 32-bit write zero-extends), which is exactly what every
narrow consumer re-extends from, so no canonicalization instruction is added.
Both the standalone and the fused div/rem-pair lowering take the new path.

## Follow-on fix — the accumulator's canonical form is remembered, not re-derived

Extending `%rax`/`%rcx` on every narrow staging is correct but redundant when the
register already holds the value's full 64-bit form (an extending slot load, an
extend emitted earlier, or a zero-extending load of an unsigned narrow value).
`RegCache` now carries `acc_canonical64`/`sec_canonical64`, cleared by exactly the
choke points every `%rax`/`%rcx` write already goes through (`set_acc`,
`invalidate_acc`, `invalidate_all` — the same ones the debug epoch machinery
enforces), and re-established only where the canonical form is known. A stale
mark is therefore impossible: the mark names the value the cache says is in the
register, and parking any value drops it.

## Follow-on fix — large CFA offsets were truncated by the assembler

The unwind model produced the right number and `gas` encoded it correctly, but
LCCC's **own** assembler truncated it: `CfiDirective::DefCfaOffset(i32)` parsed
`.cfi_def_cfa_offset 2147483664` through `i32`, which both wrapped the value to
`-2147483632` and switched the opcode to `DW_CFA_def_cfa_offset_sf` (a *factored*
operand, so an unwinder reads `rsp - 17179869056` instead of `rsp + 2147483664`).
`lccc -c` is the default path, so every >2 GiB frame had a wrong unwind table even
with the model fixed. Both `DefCfaOffset` and `Offset` now carry `i64`, and a unit
test asserts the emitted bytes: `0x0e` followed by ULEB128 `90 80 80 80 08`, and
no `0x13` anywhere in the section.

Note for anyone verifying this by hand: `readelf --debug-dump=frames[-interp]`
prints a CFA offset through 32 bits, so it shows `-2147483632` for a correct
encoding of `2147483664`. Decode the bytes (`objdump -s -j .eh_frame`) instead;
`tests/regression/check_narrow_int_codegen.py` does exactly that.

## The lane

`scripts/ci_gcc17_torture.sh` drives one architecture, both legs, with
revision-pinned corpus provisioning and schema-2 JSON plus failure logs written
even when a leg fails. `.github/workflows/gcc17-torture.yml` builds one fastbuild
artifact, then calls `.github/workflows/gcc17-torture-leg.yml` twice — explicit
jobs, not a `strategy.matrix`: `scripts/check_ci_gate_parity.py` models a hosted
workflow as a job DAG and rejects any job carrying a `strategy` block, so a
matrix would have failed a mandatory fast gate on every PR (measured: the parity
gate exits 1 and `scripts/test_ci_gate_parity.py` reports seven errors with a
matrix present; both pass once the leg is a reusable workflow). The lane stays off
the PR critical path; a full corpus is minutes of wall clock.

## The gate that pins all of it

`tests/regression/check_narrow_int_codegen.py` runs in the fast-contract shard
beside `check_cfi_unwind.sh` and asserts, in the emitted text and in the
`.eh_frame` bytes rather than in behaviour alone:

1. narrow divides never emit `idivq`/`divq`/`cqto`, and signed narrow indices are
   sign-extended (the fixture's index lanes are negative, where a zero-extending
   copy reads the wrong element);
2. the same narrow value staged twice is re-canonicalized at most once;
3. `large_frame_probe_boundary.c` — a 12 KiB frame that crosses three probe pages,
   calls out of the probed region and is called — runs at `-O0`, `-O2` and
   `-O2 -fno-omit-frame-pointer`, and the probe loop is closed by the
   register-form `subq %r11, %rsp`;
4. both `>2 GiB` CFA offsets are present as `DW_CFA_def_cfa_offset` + the exact
   ULEB128, never as the factored form;
5. both fixtures are clean at `-O0`…`-Os` × {-g, no -g}, with the pinned program
   outputs confirmed against `gcc` and `clang` first, so the expectations are an
   oracle rather than a snapshot.

The probe fixture also carries a `.flags` sidecar (`-O2 -fno-omit-frame-pointer`),
so the ordinary corpus run — not only the shape gate — executes a probed
prologue and compares it against the same oracle.

Shape assertions cannot tell a correct `movsbl` from a wrong one, because
`movzbl %dil, %eax; cltd; idivl %esi` is well-formed and returns 127 for
`-1 / 2`. `machinst_tests::narrow_divide_the_selector_builds_computes_the_narrow_quotient_when_executed`
therefore runs the selector's own instruction sequence for I8/I16/U8/U16 and
compares the truncated quotient against the C semantics of the same type, on
inputs whose divisors are nonzero in every narrow interpretation and that never
reach the one narrow input that traps.

## Known gaps

1. The 49 x86-64 / 48 i686 compile-leg failures are candidate failures —
   front-end parse, semantic and lowering gaps, inline-asm constraints and
   remaining backend limits. The per-test `.failures.txt` files are the
   authoritative inventory; do not collapse them into one "unsupported" bucket.
2. Frame offsets beyond signed disp32 (`20031023-4.c`) remain unimplemented.
3. The local comparison compiler for the campaign was GCC 14.2, not the trunk
   checkout; per-test identities should be re-derived against GCC 16 in CI.
4. This harness is not DejaGnu: target directives, nested suites, `.S`, LTO and
   diagnostic suites are not covered.
5. No physical-PMU performance claim is possible on shared virtualized hardware;
   the fastbuild / two-job policy is unchanged.
