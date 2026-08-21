# Session 41 — x86 ISel / peephole sweep + oracle hardening (2026-08-21)

Base: `ms178/lccc` main `aa8bc98` (PR #178). Build: `target/fastbuild`
(`opt-level=1`, no LTO, 2 jobs, 8 GB swap). Host: 2-core VM, no PMU —
all evidence is assembly diff + instruction count + VM runtime screening.

## TL;DR

Five low-risk, high-frequency codegen fixes landed this session. Each one
removes an instruction (or a full staging copy) from hot integer loops on
every x86-64 build, with zero semantic change:

| # | Fix | File(s) | Effect | Gate |
|---|-----|---------|--------|------|
| 1 | Zero-extend tracking broke on SIB memory operands | `redundant_ext.rs` | re-enables `movzbl %al,%eax` removal after `movzbl (base,idx),%eax` | unit + 1010 lib |
| 2 | Constant multiplies staged through `%rcx` | `alu.rs` | `imull $imm,%eax,%eax` (1 insn) vs `movq $imm,%rcx; imull %rcx,%eax` | unit + 389 regr |
| 3 | BT bit-index staged through `%rcx` | `alu.rs` | `btl %rN,%dest` direct (BT has no fixed count reg) | unit + 389 regr |
| 4 | 32-bit ALU writes not tracked as upper-32-zero | `redundant_ext.rs` | `movl %esi,%r12d` vs `movq %rsi,%r12` after addl/subl chains | unit + 389 regr |
| 5 | Linker oracle: pin `MOLD_LTO=OFF`, record resolved revisions | `setup_oracles.sh` | reproducible oracle revisions | bash -n |

Validation: **1010 unit tests, 389 regression + 1 skip, 50/50 correctness,
1008→1010 `cargo test --lib`, byte-identical runtime checksums** — all green.
No gzip `longest_match` stack-mem regression (the RA veto).

## 1. Zero-extend tracking broke on SIB memory operands (IS-08)

**Root cause.** `redundant_ext::self_update` split operands at the **first**
comma. For a SIB load `movzbl (%rcx,%r12),%eax` the first comma sits *inside*
the address, so `dst` parsed as `"%r12), %eax"`, `family_of_reg` returned
`None`, and the `is_byte`/`upper32_zero` fact for `%rax` was never recorded.
The redundant re-extension `movzbl %al,%eax` (emitted by the I8→U32 cast
after an indexed byte load) therefore survived in every table-lookup loop.

**Latent soundness bug fixed in the same change.** An opaque 64-bit SIB load
`movq (%rcx,%r12),%rax` hit the same parse failure in the `movq` handler and
**failed to clear** the stale byte fact for `%rax` — a later `movzbl %al,%eax`
could have been wrongly removed (upper bits of a loaded pointer survive).

**Fix.** Split at the **last** comma (AT&T puts the destination last) in every
operand split inside `self_update`. `rfind(',')` is unambiguous for the
2-operand forms these handlers accept.

**Tests added.** `removes_byte_reextension_after_sib_memory_load`,
`removes_16bit_reextension_after_sib_memory_load`,
`sib_memory_movq_load_clears_byte_fact` (soundness control).

## 2. Constant multiplies staged through `%rcx`

`i * 1664525` (gzip LCG fill) and the Adler DO8 weighted sum emitted
`movq $7,%rcx; imull %ecx,%eax`. IMUL has a 3-operand immediate form.
Emit `imull $imm,%eax,%eax` (32-bit) / `imulq $imm,%rax,%rax` (64-bit)
directly in the general binop path and the fused multiply-add path.

**Width correctness.** The 32-bit form accepts any imm32 bit pattern
(`N ≡ -1 mod 2^32`, so the low 32 bits are exact); the 64-bit form is
restricted to the signed i32 range because imm32 sign-extends. Uses the
existing `const_as_imm32_typed`.

## 3. BT bit-index staged through `%rcx`

`(x >> i) & 1` lowering forced the bit index through `%rcx`
(`movq %r10,%rcx; btl %ecx,%eax`) because the comment wrongly claimed BT has
a "fixed count register" (that restriction belongs to variable shifts/`%cl`).
BT's index is an ordinary r/m operand. Consume the index straight from its
own register (`btl %r10d,%eax`), guarding the dest-aliasing case in the
register-direct path. Also removed a **dead** `movq %r10,%rcx` the text
peephole had already bypassed in the Expat name-scan classify.

`tests/regression/check_bit_test_canonical.sh` updated to assert the general
register-index BT shape instead of the staging register (oracle update, not
regression-hiding: the canonical BT still fires).

## 4. 32-bit ALU writes are upper-32-zero

`addl/subl/andl/orl/leal/imull/sall/shll/shrl/sarl/cmovl*` write the low 32
bits and zero-extend to 64 on x86-64, but the tracker only recorded
`movl/xorl/movzbl/movzwl`; every other 32-bit write fell into the
"clear everything" fallback. So `addl %eax,%esi; movq %rsi,%r12` never
narrowed to `movl %esi,%r12d` (Adler DO8). Added a writer whitelist;
`cmpl`/`testl` deliberately excluded (flags-only, leave the value alone).

## 5. Linker oracle hardening

`tests/linker/setup_oracles.sh` already honoured the build/version policy
(mold+wild from git HEAD, `-DMOLD_TARGETS='X86_64;I386'` fast preset,
mimalloc off, bfd 2.47 pinned). Added `-DMOLD_LTO=OFF` (explicit — LTO would
slow the constrained-host oracle build) and a resolved-revision record
(`ORACLE_REVISIONS.txt`) so each session's docs cite the exact mold/wild
commit instead of an unreproducible "HEAD".

---

## Remaining gaps (do not re-derive — this is the map)

All measured on this session's build; instruction counts are `-O2
-march=x86-64-v3` whole-file unless noted.

### A. Adler-32 kernel — 1.53× GCC (deepest codec gap now)

`tests/benchmark/programs/zlib_ng_adler32.c`, DO8 loop (`LBB18`). Root cause:
the reassoc pass expands `sum2 += sum1` ×8 into the closed form
`sum2 += 8*sum1 + 8*b0 + 7*b1 + … + 1*b7`, which **materialises all eight
bytes simultaneously** and spills `sum1` (`128(%rsp)` reload twice per
iteration) and `sum2` (`104(%rsp)`).

GCC/ICX instead interleave a **parallel prefix-sum**: keep `sum1` as the
running prefix (`esi`), keep `sum2` in a register, and accumulate the weighted
sum into a second register with a tree (`a0=s1+b0; b1'=a0+b1; r11=b1'+b2; …
eax=a0+b1'+r11+r10+…; sum2+=eax`) — ~26 insns, 0 stack. This is a
**delayed-accumulation / prefix-sum reassociation**, not an RA fix. The
current lccc closed form is *correct* but pressure-hostile. Candidates:

1. In the `reassoc_accum` (or whichever pass produces the closed form), refuse
   the transform when the number of materialised byte temporaries exceeds the
   allocatable-GPR budget (register-pressure cost model, §25 of the mandate).
2. Or recognise the `s += a; a += b` shape and emit the prefix-sum tree.

Also present but secondary: a dead `movq %rax,24(%rsp)` spill-store of the
`4*b4` intermediate, and `movq %rdx,%rcx` dead staging copies in the sum2
tail — symptoms of the same pressure problem.

### B. struct_copy — 1.53× GCC (ABI + SROA, OP-02 blocked)

`tests/benchmark/programs/struct_copy.c`. The by-value `Particle`/`Group`
path keeps every field in stack slots (39 stack-mem) and re-loads per field;
GCC passes aggregates in XMM/YMM and copies 64 B with 2× `ymm` pairs.
This is `OP-02` (dominator-aware SROA copy-out, **BLOCKED** — hangs
`structs_bitfields`, `simd_sse_float` without a dominance/ABI proof) plus
`AB-01`/`AB-02` (SysV SSE-class return/args). Do **not** enable
`CCC_SROA_COPYOUT`; the block is real.

### C. Expat name scan — 1.90× GCC (classify + branch structure)

`expat_utf8_name_length`. The classify is already BT-based after OP-15, but
GCC lowers the *whole* `xml_name_continue` to a single 256-bit table probe
(`btq %rax, table(%rip)`), while lccc still splits start/continue into ~28
insns with per-case `movq $1,%r; jmp` blocks. Merge the two bit-tests into one
table, then fold the multi-byte width ladder (`c2..df → 2`, `e0..ef → 3`,
`f0..f4 → 4`) into range arithmetic. The 1.90× is branch-prediction shaped
(the VM has a weak predictor), so instruction-count reduction alone may not
close it — needs PMU on the 14700KF.

### D. RA-01 / IS-26 — PIE-mode `leaq sym(%rip)` re-materialised in loops

In PIC/PIE mode the indexed-symbol base cannot fold into a memory operand
(no RIP-relative SIB), so `emit_load_indexed_sym_impl` / the register form
materialise `leaq sym(%rip),%base` **inside the loop** (gzip CRC `.LBB12`
has two per-iteration `leaq`s). `classify_site_local_indexed`
(`src/passes/global_addr_cse.rs:94`) deliberately keeps variable-index bases
site-local to preserve the non-PIE `sym(,%idx,4)` fold — but in PIC/PIE mode
that fold is impossible, so hoisting is correct *and* profitable.

Fix sketch: plumb the code model into the pass pipeline (`run_passes` takes
`target` but not pic/pie); when position-independent, let
`classify_site_local_indexed` hoist the base to the innermost preheader (the
`choose_placement` machinery already implements preheader hoisting for
non-indexed bases). Then `emit_load_indexed` (register form) consumes the
hoisted base. Kill-switch `CCC_NO_GLOBAL_ADDR_REMAT` already exists for A/B.
Expected: CRC loop 16→14 insns, and the gzip `longest_match` 36 stack-mem /
`leaq window(%rip)` per-iteration is the same defect.

### E. RA-05 / RA-06 — the #1 remaining perf gap (do not half-do)

`live_range.rs` still scans **fat** `intervals`; `segments` (hole-aware) is
built but only consumed by `regalloc.rs` for call-spanning. Wimmer-style
in-place splitting + reload-at-next-use is the 3–5× spill-traffic fix for
xmltok/inflate (12×/15× stack-mem). This is the single highest-value item and
the most delicate; gate on `enable_splitting`. Do not attempt without the
full differential battery (600 phi + 540 alias + gzip 30/30).

### F. Accumulator staging cleanup (easy wins, same class as §3)

Several kernels still emit dead staging copies that the text peephole leaves
behind: Adler `movq %rdx,%rcx` (dead), CRC `movl %eax,%edi` before
`andl $255,%edi` (could be destructive `andl $255,%eax`), and the
`movq→movl` narrowing's single-instruction lookahead misses
`movq %rax,%r9; movl %ebx,%eax; movl %r9d,%ecx`. Extend the narrowing
profitability filter to the first consumer of the destination register
within the block (stop at labels/calls/64-bit consumers) — provably safe on
top of the existing upper-32-zero proof.

---

## How to reproduce

```bash
scripts/ensure_swap.sh && scripts/build_lccc_fast.sh
python3 tests/benchmark/run_benchmarks.py --only gzip_crc32,zlib_ng_adler32,struct_copy,expat_xml_scan --compilers lccc,gcc --lccc ./target/fastbuild/lccc --opt=-O2 --reps 11
python3 scripts/godbolt.py compare tests/benchmark/programs/gzip_crc32.c --local target/fastbuild/lccc --oracles gcc16.2,clang,icx --flags '-O2 -march=x86-64-v3'
cargo test --profile fastbuild --lib
CCC=./target/fastbuild/lccc bash tests/regression/run_regression.sh
```

## Scoreboard snapshot (this session, VM screening, 11 reps, paired median)

| Kernel | LCCC | GCC | ratio |
|---|---:|---:|---:|
| gzip_crc32 | 181.67 ms | 169.69 ms | 1.07× |
| zlib_ng_adler32 | 62.79 ms | 40.27 ms | 1.56× |
| struct_copy | 45.24 ms | 29.45 ms | 1.53× |
| expat_xml_scan | 77.77 ms | 41.25 ms | 1.89× |

Checksums byte-identical on every kernel. No PMU on this host; treat ratios
as screening, not hardware claims.
