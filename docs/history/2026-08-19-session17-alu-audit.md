# 2026-08-19 (session 17) — critical review of the Grok i686 alu.rs audit

**Base:** `origin/main` @ `93a09491` (Merge PR #130 — session 16 register-source fold)
**Snapshot:** `/home/user/ms178-1.patch`

## Verdict on the Grok report — finding by finding (all empirically checked)

| # | Finding | Verdict | Evidence |
|---|---|---|---|
| 1 | F32 neg = 5 SSE ops → `xorl $0x80000000,%eax` | **AGREE** — implemented | `negf` emitted movd/movd/xorps/movd + 2 movl; F64/F128 already go through x87 `fchs`, so `emit_float_neg_impl` is F32-only. Now 1 insn, bit-exact vs gcc. |
| 2 | CLZ I8/U8 wrong (bare lzcntl) | **Agree (latent)** — fixed defensively | C promotes `__builtin_clz(u8)` to int, so the narrow branch is **unreachable from C** (verified: exit codes match C semantics); still wrong if ever reached. |
| 3 | CTZ narrow wrong for 0 | **Agree (latent)** — fixed (`orl $0x100`/`$0x10000` stop bit) | Same reachability. |
| 4 | Popcount narrow un-masked | **Agree (latent)** — fixed (`andl $0xff/$0xffff`) | Same reachability. |
| 5 | Bswap I8 → no-op; I16 needs `movzwl` | **Agree (latent)** — fixed | rolw leaves bits 16–31 stale. |
| 6 | Direct-path shift imm unmasked | **AGREE** — implemented | `shl300` emitted `shll $300` (imm8 overflow, GAS-incompatible). Now masked `&31`. |
| 7 | Direct mul 3/5/9 uses imull not LEA | **AGREE** — implemented (subset) | LEA forms are 3 bytes = same size, 1 cyc vs 3. Also added 0/±1/2 and *4/*8-in-place (all ≤ imull size). |
| 8 | No memory-destination ALU | **REJECT** — unsound as written | Grok's `operand_mem_ref` = bare `get_slot`. Reproduced: 2 fuzz failures (seed 11) — the codegen fold changes the read shape so the peephole's store-forwarding eliminates the materializing store, leaving `divl 52(%esp)` reading garbage. The peephole folds slots correctly post-materialization. |
| 9 | No memory-source ALU | **REJECT** — same as 8 | Identical mechanism. |
| 10 | Mul strength reduction incomplete (LEA chains) | **Reject for -Os** | *6/*7/*10 chains are 2 insns vs 1 `imull` = size regression. Implemented only the ≤-imull-size subset. |
| 11 | No const div/rem (magic, pow2) | **Reject for -Os** | Magic udiv ~10 insns vs `divl` 3; signed /2^k bias 5 vs `idivl` 3 — size regressions. Unsigned pow2 is already shrl/andl in the IR. A speed-only build wants these behind a switch. |
| 12 | Commutative imm only on RHS | **Moot** — IR canonicalises `5+x`→`x+5` (verified). |
| 13 | No 3-operand LEA add | **AGREE** — implemented | `leal (%a,%b),%d` 3 bytes vs movl+addl 4+. |
| 14 | Variable shifts never direct | **AGREE** — implemented | Direct path now handles `shll %cl,%d` when dest is a register (≠ %ecx). |
| 15 | ALU identities ignored | **Moot** — IR already folds `+0/|0/^0/&-1/<<0` (verified: f1–f5 all emit bare movl). |

**Also rejected**: Grok's `emit_mul_imm_into_reg` uses `%ecx`/`%edx` as scratch in the
`same`-register *7/2^k±1 cases — that **clobbers live caller-saved registers** inside
the direct path (whose contract is to leave them untouched). A correctness bug in the
proposal; not ported.

## Shipped

`src/backend/i686/codegen/alu.rs` (rewritten): F32 xorl neg, masked shift counts,
single-instruction mul strength reduction, 3-operand LEA add, variable-shift direct
path, narrow clz/ctz/popcount/bswap correctness. Regression test
`tests/regression/check_alu_audit_fixes.sh`.

## Numbers

- boot object text **33481 → 33445 (−36)**, no object regressed (printf −7,
  video −6, string −10, early_serial −3, cpucheck −3, video-vesa −2, video-bios −2).
- 918 unit / 50 correctness / **344**+6 regression / **600-case** i686 differential
  fuzz clean. Only i686 files touched (x86-64 byte-identical).

## Remaining (the real structural work, unchanged)

1. Slot-operand folding in codegen needs the copy-alias/coalesced-slot read
   redirection (the value's *live* materialisation, not its nominal `get_slot`).
2. `%eax` allocatability + caller-saved-first Phase 2 ordering (push/pop tax).
3. A `-Os`/`-O2` size-vs-speed switch to admit magic division and LEA chains
   only when size does not matter.
