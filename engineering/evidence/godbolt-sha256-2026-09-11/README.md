# Compiler Explorer oracle: `sha256_transform` at `-O2` (2026-09-11)

Reproduce with:

```
scripts/godbolt.py compare tests/benchmark/programs/sha256_transform.c \
    --local target/fastbuild/lccc \
    --oracles gcc16.2,clang23.1,icx,icc \
    --function sha256_transform \
    --artifact-dir engineering/evidence/godbolt-sha256-2026-09-11 \
    --json engineering/evidence/godbolt-sha256-2026-09-11/manifest.json
```

| compiler | insns | frame-slot refs inside the loop body | artifact |
|---|---|---|---|
| clang 23.1.0 | **126** | **0** | `clang23.1.s` |
| gcc 16.2 | 154 | 1 | `gcc16.2.s` |
| lccc (shipping) | 198–206 | **52 refs over 14 distinct slots** | `lccc.s` |
| icx (latest) | 1069 | — (fully unrolled) | not retained; count in `manifest.json` |
| icc (classic) | ERROR | — | — |

## The distilled finding

clang and gcc keep **zero** in-loop stack traffic; lccc has 52 slot references over 14
distinct slots. The hottest slot, `360(%rsp)` (10 accesses), holds the first parameter
`u32 *state` — stored once at entry and reloaded at *every* use in the write-back
epilogue:

```asm
    movq 360(%rsp), %rax      ; reload base
    leaq 4(%rax), %rax        ; base+4 INTO THE SAME REGISTER -> base is gone
    movq %rax, %r8
    movl (%rax), %eax
```

clang holds the base in one register and emits `movl 4(%rbase), %eax` directly.

Two conclusions, both recorded against `engineering/tasks/TASK-RA-06A-RELOAD-AT-USE.md`:

1. The allocator's decision to *spill* `state` is correct — it is used in two clusters
   with the 64-round loop between them. This is a codegen gap, not an RA gap.
2. The gap is **addressing-mode selection plus register residency**, not reload-CSE.
   Reload-CSE was implemented, proven sound and measured at 2 instructions out of 8414
   (0.024 %); see `engineering/FOLLOWUP-2026-09-11-x86-slot-load-dedup.md`.

Oracle target for the task: drive in-loop slot refs 52 → **0** and insns 198 → **126**.
