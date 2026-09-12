# x86 slot-load dedup (load→load forwarding) — IMPLEMENTED, SOUND, **NOT SHIPPED**

Date: 2026-09-11 · Status: **negative result, direction closed** · Kill switch used: `CCC_NO_SLOT_LOAD_DEDUP`

## 1. What the oracle exposed

Compiler Explorer on `sha256_transform` at `-O2` (evidence: `engineering/evidence/godbolt-sha256-2026-09-11/`):

| compiler | insns | frame-slot refs inside the loop body |
|---|---|---|
| clang 23.1.0 | **126** | **0** |
| gcc 16.2 | 154 | 1 |
| **lccc** | **198** | **52 refs over 14 distinct slots** |
| icx | 1069 (fully unrolled) | — |
| icc | ERROR | — |

The hottest slot was `360(%rsp)`, touched 10 times. It holds the **first parameter
(`u32 *state`)**, stored once at entry (`movq %rdi, 360(%rsp)`) and then **reloaded
before every single use** in the epilogue that writes the accumulated words back:

```asm
    movq 360(%rsp), %rcx      ; reload the state pointer
    movl %r10d, %eax
    addl (%rcx), %eax         ; state[0] + a
    movq 360(%rsp), %rcx      ; reload it AGAIN
    movl %eax, (%rcx)
    movq 360(%rsp), %rax      ; and AGAIN
    leaq 4(%rax), %rax        ; &state[1]  <-- clobbers the register holding it
    ...
```

Nine reloads of one loop-invariant pointer in a straight-line block, where clang and
gcc hold it in a single register.

**Diagnosis (important):** the register allocator's decision to *spill* `state` is
**correct**. The pointer is used in two clusters — function entry and epilogue — with
the whole 64-round loop in between, so homing it in a slot is the right call. The
defect is purely that codegen re-reads the slot at every use instead of once. That is
reload-CSE, and the repo already documented that **x86 has no slot-load dedup while the
ARM backend does** (`eliminate_repeated_slot_loads`, `src/backend/arm/codegen/peephole.rs:2742`).

## 2. What was implemented

Not a port of ARM's text-scanning pass. x86 already has a *typed*, better-instrumented
pass with exactly the right safety machinery: `global_store_forwarding`
(`passes/store_forwarding.rs`), which tracks `slot → register` mappings from **stores**
and forwards them to later loads. The missing piece is the sibling direction: after
`movq slot, %reg`, the slot's contents are *also* live in `%reg`.

The change recorded that mapping, reusing the store path verbatim:

```rust
if is_valid_gp_reg(load_reg) {
    invalidate_reg_flat(slot_entries, reg_offsets, load_reg);   // the load redefines it
    if dedup_loads {
        gsf_handle_store(load_reg, load_offset, load_size, slot_entries, reg_offsets);
    }
}
```

Policy was threaded explicitly (`global_store_forwarding_with(.., dedup_loads)`) with a
thin env-reading wrapper, so **both arms are unit-testable without mutating the process
environment**.

### Soundness

The recording rests entirely on invalidation the pass already performs, which is
complete for frame slots:

* any instruction touching the slot — **including a read-modify-write such as
  `addl %eax, 32(%rsp)`** — carries `rbp_offset` and range-invalidates 16 bytes;
* `has_indirect_mem`, calls, returns, jumps, jump-target labels and inline asm
  invalidate everything;
* redefinition of the holder drops the mapping via `invalidate_reg_flat`;
* `parse_rbp_offset` covers `(%rsp)` as well as `(%rbp)`, so frame-pointer-less
  frames work, and `%rbp`-as-data-register is guarded by `rbp_is_frame`;
* the pre-existing width guard is untouched, so a dword load still never feeds a
  qword load.

Six tests were added, one per barrier: the win shape, the disabled arm, slot RMW
between loads, clobbered destination between loads, dword→qword, and call between loads.

## 3. Verification

* `cargo test --lib`: **2394 passed, 0 failed** (2388 baseline + 6 new).
* The 30 affected corpus TUs include the historically fragile shapes —
  `i128_pair_store_load`, `switch_i128_high_half`, `i128_stack_argument`,
  `vararg_indirect_va_list`, `va_arg_wide_struct`. All five were assembled, linked and
  **executed** with dedup ON and OFF: identical exit codes and identical stdout
  (`ok const 00000030000000000000000000000003`, `PASS`, `OK`, `68719476736 38`).
  The deleted lines were genuinely redundant reloads; in `switch_i128_high_half` the
  dedup correctly cascaded into dead-store elimination of the now-unused spills.

**The change is sound.** That is not why it was rejected.

## 4. Why it was NOT shipped: it does not pay

Measured by compiling every benchmark with and without the kill switch:

| metric | result |
|---|---|
| benchmark programs compiled | 51 |
| programs whose asm changed | 3 (`fannkuch`, `linux_rbtree`, `sha256_transform`) |
| **total instructions removed** | **2 of 8414 (0.024 %)** |
| frame-slot loads removed | 1 of 297 |
| corpus TUs changed | 30 of 807 |
| sha256's gain | **exactly 1 instruction** |

Mechanism for why the win is ~zero, straight from the emitted asm: each ~6-instruction
group of the epilogue contains a **genuine indirect write through the very pointer being
reloaded** (`movl %eax, (%r8)`), which must invalidate all mappings; and
`leaq 4(%rax), %rax` **clobbers the register that would hold the mapping**. Only one
adjacent same-register pair (`movq 360(%rsp), %rcx` twice) was actually dedupable.

Shipping a function-wide behavioural change to a soundness-critical pass for a 0.024 %
static gain fails the mission bar (§30: a P0 is done only if a realistic benchmark
*wins*). Risk without reward. **Reverted; the tree is byte-identical to the shipping
binary again.**

## 5. Corrected lead — the real target is P0-C, not reload-CSE

The oracle gap is not "we reload too often", it is "**we materialize addresses instead
of using addressing modes, and we clobber the base while doing it**":

```asm
    movq 360(%rsp), %rax      ; reload base
    leaq 4(%rax), %rax        ; base+4 INTO THE SAME REGISTER -> base is gone
    movq %rax, %r8
    movl (%rax), %eax
```

clang emits `movl 4(%rbase), %eax` / `movl %eax, 4(%rbase)` with the base held in one
register across the whole block — zero reloads, zero `lea`. Two concrete leads, in order:

1. **Addressing-mode selection:** prefer `disp(base)` (and `disp(base,index,scale)`)
   over materializing `base + k` into a register, especially when the materialization
   overwrites the base itself. This is the P0-C "Address/IV/SIB" item and it removes
   both the `lea` *and* the reload, which reload-CSE can never do.
2. **Register residency for spilled-but-cluster-invariant bases:** when a spilled value
   is reloaded from its home slot ≥ N times within one basic block with no intervening
   call, admit it to a callee-saved register for that block. This is the P0-A "global
   location allocation" item, and it is what actually produces clang's `0` in-loop slot
   refs.

### Adjacent finding, deliberately not attempted

`has_indirect_mem → invalidate_all_mappings` is over-conservative for **read-only**
indirect operands: `addl (%rcx), %eax` cannot change a frame slot's contents, yet it
kills every mapping (this is what blocked dedup at sha256's first pair). Refining it to
"invalidate on indirect *write*, not indirect *read*" is sound in principle but must
exclude `xchg`, `xadd`, `cmpxchg*`, `lock`- and `rep`-prefixed forms and the implicit-
destination string ops (`stos`/`movs`/`scas`/`cmps`). That widens the same risk surface
across every slot-tracking pass (dead stores, frame compaction, store forwarding) — and
§4 shows the payoff is nil. Not attempted; recorded so the reasoning is not re-derived.
