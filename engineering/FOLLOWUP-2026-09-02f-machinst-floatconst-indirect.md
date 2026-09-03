# Followup: float-const stores, wide-imm call args, typed indirect calls

Session 2026-09-02 (trace 1a063f55bd44e4bd), S26 on top of PR #358 (6fcbb1d2).

## What the census ranked (at -O2, 571-file corpus)

| class | before | after | disposition |
|---|---:|---:|---|
| Store(float) | 123 | 13 | **typed** — 1-instr immediate forms; residual = x87/F128/va_arg/param-home shapes (legitimately text-path) |
| Call(imm-too-wide) | 13 | 0 | **typed** — movabsq staging into the arg register |
| CallIndirect | 30 | 0 | **typed** — CallTarget::Indirect, staging move inside the atomic call |
| Memcpy | 94 | 94 | deliberate non-migration: mature path is oracle-comparable with measured ymm/SSE trade-offs |
| Store(other) | 79 | 79 | i128/F128 pair stores (x86-64) — next round |
| ParamRef(needs-code) | 46 | 46 | unchanged |
| VaStart/VaEnd | 71 | 71 | deliberate non-migration: mature path already GCC-shape |
| InlineAsm | 37 | 37 | by nature text-path |

## The regression the suite caught (and why it matters)

fp_arith at -O2 crashed (exit 139) after the float-const landing. Root cause chain:

1. The hi half of a split F64 store was spelled as the raw u32 (`4294443008`
   for 0xE0000000) instead of the sign-extended imm32 (`-536870912`).
2. That left the imm32 window, so the `Mov` emitter's wide-immediate memory
   relay fired — and that relay (pre-existing, integer-shaped) always stored
   8 bytes (`movq %rax`), regardless of the operand size.
3. The extra 4 bytes clobbered the neighbouring slot.

Fixes: (a) the lowering spells both halves `as i32` — the C7 /0 id form takes
a 32-bit operand and writes exactly 32 bits; (b) the relay honors the operand
size (`movl %eax` for S32) so the emitter defines the behavior instead of
trusting typed IR never to produce the shape. The executed probe's layout
stores a non-zero constant directly after a sign-bit-hi pattern so this class
can never mask again (the first probe version masked: the clobbered bytes
were zero anyway).

## Immediate-form shapes (the oracle gap)

For float constants the bit pattern IS the data. GCC 15/16.2, Clang 23.1 and
ICC 2021.10 all materialize non-zero float constants through the rodata pool
(`movss/movsd .LCn(%rip), %xmm0` + SSE store). The typed lowering:

- F32/D32: `movl $bits, mem` — ONE instruction; every u32 is a valid imm32
  operand (C7 /0 id sign-extends, the store writes the full 32 bits).
- F64/D64 == 0 (bit pattern!): `movq $0, mem` — ONE instruction; beats the
  oracle xorps+movsd pair. -0.0 is NOT zero (high half set).
- F64/D64 fitting i32 as i64: `movq $imm32, mem` — ONE instruction; covers
  -0.0 (0xFFFFFFFF80000000 == i32::MIN sign-extended) and small denormals.
- otherwise: two `movl` halves at slot/slot+4 or off/off+4 (frame-mode
  independent), covering exactly the bytes the one wide store covered.

No rodata entry, no load-port traffic, no xmm dependency, no partial-register
false dependence. Live scoreboard: parity-or-better on all const-store probes;
the +1-instruction residuals are float-local slot-homing + double reload on
the return path (pass-level SROA/return forwarding — backlog below).

## Typed indirect calls

`CallTarget { Direct, Indirect(PhysReg) }` unifies both call forms in the ONE
atomic `CallTyped` instruction. The callee pointer is staged into r10
(fallback r11) by one of the instruction's own moves; the identical
reader-before-writer rule protects it:

- self-homed callee: staging elided;
- callee in an argument register: staging ordered before that register's writer;
- argument sourced from the target register: ordered before the staging;
- r10 register-exchange cycle: falls back to r11 (pinned by test);
- alloca-address callee: refused when an argument reads the candidate register.

Out of scope (mature path keeps them): retpoline external thunk (`call
__x86_indirect_thunk_r10` — the register is IN the thunk symbol), inline
thunk mode, and the paravirt patchable `call *sym(%rip)` (kernel
alternatives rewrites it at boot).

## Backlog (census-ranked, next round)

1. **Store(other) 79** — i128/F128 pair stores on x86-64 (switch_i128_high_half
   21, f128_softfloat 20, glibc_f128_builtins 10, ...). The pair-store shapes
   are bounded and testable the same way.
2. **CallIndirect(callee-unrepresentable)** — the dominant real indirect shape:
   a GOT load with no allocator home leaves the pointer in %rax and the mature
   path relays it into r10 (`movq %rax, %r10` — 2 instructions where GCC's
   GOT-load-into-scratch is 1). Fix = use-count-driven GOT-load destination
   override (load directly into r10 when the pointer's single use is the
   call), which needs allocator coordination. Now census-visible.
3. **ParamRef 46** — the register-copy and stack-load subsets.
4. **float-local SROA / return forwarding** (pass-level) — the +1-instruction
   residual on const-return probes: `float x = 1.5f; sinkf = x; return x`
   reloads x twice. mem2reg for scalar float locals feeding stores.
5. **Tail-call elimination + frame-reservation trimming** (pass-level) —
   callw residual: `subq $8; ...; call; addq $8; ret` vs GCC's sibcall `jmp`.
6. **__int128 on i686** — full feature project: frontend gate
   (frontend/parser/types.rs:258), IR i128 lowering, i386 ABI (8-byte-aligned
   stack pairs), __udivmodti4 libcalls. Seven test families. Not attempted
   in-session.
