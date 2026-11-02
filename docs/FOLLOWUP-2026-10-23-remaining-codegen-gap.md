# FOLLOWUP 2026-10-23 — where the remaining codegen gap actually is

Status: evidence, not a fix. Nothing here changes codegen. It exists so the next
session does not have to re-derive the attribution, and so the next change is
aimed at the mechanism rather than at a symptom.

## What the synthesized branch delivered

`run_benchmarks.py` (49 programs, `-O2`, paired, GCC 16.2 as the reference),
`lccc/gcc` ratio, lower is better:

| arm | geomean ratio |
|---|---|
| `main` before the synthesis | 0.7906 |
| synthesized branch (merged) | **0.7897** |
| #926 alone | 0.7816 |

`#926`'s number is better only because that arm was measured in a different
session window; the paired runs (`/tmp/bench/synth926.json` at the time) put the
synthesis 0.50 % ahead of #926 alone, and no individual PR moved the corpus
outside its own noise band (CV 0.2–0.5 %). The corpus is therefore delivered;
what follows is the tail.

## Method: per-instruction Ir attribution

`callgrind_annotate` cannot attribute by opcode and its absolute-cost lines
repeat address blocks, so the aggregation must come from the raw dump:

1. `valgrind --tool=callgrind --callgrind-out-file=<f> --compress-pos=no ./prog`
   (`--compress-pos=no` is not optional: with position compression the dump is a
   ~63 %-unmapped artifact).
2. Join `positions: instr line` cost lines (`0xADDR LINE Ir ...`) with
   `objdump -d --no-show-raw-insn` on the same binary to get a mnemonic per
   address, de-duplicating repeated address blocks by maximum cost.
3. Aggregate Ir per mnemonic; the ratio of the two totals is the instruction
   count ratio, and the per-mnemonic delta is the attribution.

`zstd_count`: 104 710 123 Ir (lccc) vs 86 359 313 (GCC 16.2) = **+18.35 M
(+21.2 %)**. `mov*` is 33.5 M of the lccc total and **+13.87 M of the 18.35 M
gap — 76 % of it**. Everything else is noise: `cmp` −0.89 M, `imul` −0.30 M,
`lea` −5.2 M, `add` −0.9 M, `and` −4.7 M, `sub` −2.1 M, offset by `movzx`
+1.37 M, `shl` +1.37 M, `shr` +2.09 M, `jmp` +1.95 M, `jcc` +0.47 M.

Static per-query body, lccc vs GCC (2.1 M queries per run):

```
lccc                                        gcc
lea buffer,%rdx        <- symbol + reg     lea 0x404040(%r10),%r11
add %r12,%rdx                              (one instruction)
mov %r8d,%r15d         <- zext copy        and $0xffeff,%r10d
lea buffer,%rbx        <- symbol + reg     add $0x404040,%rcx
add %r15,%rbx
mov %r14d,%edx         <- shift-count       and $0x7,%ecx
and $0x7,%edx              chain            shl $0x3,%ecx
shl $0x3,%edx                               shl %cl,%rax
mov %edx,%ecx
shl %cl,%rsi
mov %r12d,%r10d        <- zext copy        xor %r10,%rax
xor %r10,%rsi
mov %rbp,0x10(%rsp)    <- loop-carried     (flag lives in %dh)
mov 0x10(%rsp),%r11d       spill + reload
```

Four mechanisms, all allocator-side:

1. **Zero-extending copies.** `mov %r8d,%r15d` exists because the IR keeps a
   `u32` value and its `zext` as different webs; on x86-64 the 32-bit
   definition already zeroes the upper half, so the copy is pure redundancy.
   `self_zext` removes the same-register re-extension and `copy_propagation`
   handles same-block copies whose source dies, but the surviving shape is a
   cross-web copy that needs interference-checked coalescing in
   `src/backend/regalloc.rs`, not another peephole.
2. **Symbol + register not fused into the `lea`.** `lea sym(%rip),%T; add
   %S,%T` is two instructions where non-PIC `lea sym(%S),%T` is one. The
   `Add(symbol, register)` fold that exists (`reg_off_sym_fold_ok`) only
   consumes the add when *every* use is a memory operand; here the pointer is
   also compared and subtracted, so the `Add` stays live and no fold runs. Only
   12 occurrences across the 49 corpus programs, so the corpus-level value is
   ~0.2 % — worth doing for fidelity, not for the ratio.
3. **Loop-carried spill.** A loop-invariant-ish flag is stored to `0x10(%rsp)`
   and reloaded every iteration (a store-to-load forwarding chain in the loop
   carry) while GCC keeps it in `%dh`. Two instructions and a few cycles per
   iteration in a 16-instruction loop.
4. **Fixed-register shift chains.** The `%cl` shift count is computed in a
   scratch register and copied into `%rcx` instead of being computed *in*
   `%rcx`, because the producer chain is an SSA web the allocator never
   re-homes. GCC reuses `%ecx` from the start.

## The strongest single signal: the move census

Counting `mov*` as a fraction of all static instructions over 67 corpus
programs, the means are almost identical (lccc 0.332, GCC 0.328) — but the
distribution is not. The programs where lccc is slowest against GCC are exactly
the programs where it emits the most moves:

| program | lccc mov/total | GCC mov/total |
|---|---|---|
| `percpu_ptr_walk` | 92/136 = 0.676 | 27/57 = 0.474 |
| `lz4_compress` | 149/312 = 0.478 | 98/284 = 0.345 |
| `libm_round_family` | 113/257 = 0.440 | 68/216 = 0.315 |
| `hot_web_loop` | 35/92 = 0.380 | 17/66 = 0.258 |
| `llvm_scalar_facts` | 95/260 = 0.365 | 63/256 = 0.246 |
| `i686_alu_chains` | 161/446 = 0.361 | 72/361 = 0.199 |
| `volatile_reduce` | 30/88 = 0.341 | 20/77 = 0.260 |
| `switch_dispatch` | 31/94 = 0.330 | 17/72 = 0.236 |
| `zstd_count` | 37/116 = 0.319 | 18/91 = 0.198 |
| `llvm_word_counts` | 65/204 = 0.319 | 25/123 = 0.203 |

`hot_web_loop` is the same story with the mechanism already identified in
`docs/FOLLOWUP-2026-10-22-ipa-ra-callee-clobbers.md`: the loop body carries four
register copies per iteration that exist only to move recency-defined values
into callee-saved homes, because a `static noinline` callee is assumed to clobber
every caller-saved register. GCC keeps the same values in caller-saved registers
and needs no copies. That document already specifies the fix (post-allocation
per-callee written sets, reverse topological order, caller-saved fallback) and
its definition of done.

## What this rules out

* **More peephole copies fixes.** `lea_narrow` (this branch) is worth 2
  instructions in the corpus, and it is the *last* shape of its family:
  `self_zext` already removes nearly every redundant re-extension (3 surviving
  `mov %Xd,%Xd` across the whole corpus, all of them load-bearing).
* **Instruction-count tuning as a goal.** `arith_loop` executes 0.808× GCC's
  instructions and still runs 1.171× slower; `nbody` is 1.196× in both. The
  native ratio is not a function of Ir here, so an Ir-driven micro-optimization
  cannot be validated by counting instructions alone — it has to be measured
  paired, and the measurement host's noise floor is ±1–2 % on unchanged code.
* **Vectorization as the next step for `byte_load_reassembly`** is the only
  remaining structural gap there (GCC's SSE2 `pcmpeqd/pshufd/punpck/paddb`
  buffer init and 4-wide decode), and it is a project, not a patch.

## Reproducers

* `zstd_count` single loop body: `tests/benchmark/programs/zstd_count.c`.
* Zero-extension copy chain: `tests/benchmark/programs/zstd_count.c` (the
  `mov %r8d,%r15d` / `mov %r12d,%r10d` pair), minimal form `unsigned f(unsigned
  *p, u64 base) { unsigned x = (p[0] >> 12) & 0x7ffff; return base + x; }`.
* Symbol + register: `lea sym(%rip),%T; add %reg,%T` in
  `tests/benchmark/programs/sqlite_varint.c`, `zstd_count.c`,
  `lz4_compress.c`.
* Move census: compile every `tests/benchmark/programs/*.c` with both compilers
  and count `^\s+mov[a-z]*\s` against `^\s+[a-z]`.
