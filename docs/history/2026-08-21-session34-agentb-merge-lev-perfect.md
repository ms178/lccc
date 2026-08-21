# 2026-08-21 session 34 — Agent B audit merge + perfected Lev adaptations

**Base:** `ms178/lccc main` `c00a71831ea63d98e2ba54c24d92e2d91ab7aebd`.
**Agent B patch:** `/home/user/uploads/ms178-1-AgentB.patch.txt` (40 866 B).
**This session's prior work:** class-aware GlobalAddr CSE (session 33).

## Is Agent B better?

**On correctness, yes in one decisive place. On the Lev patchset as a
performance program, no — they rejected too much of what can be perfected.**

### Where Agent B is strictly better

1. **zlib-ng Huffman `bi_reverse` miscompile.** Empirical, not rhetorical.
   The x86-64 immediately-consumed no-home policy forwarded a U32 through
   `U32→U8→U64` and used 8188 instead of `(uint8_t)8188` (`ltree[261].Code`
   575 vs GCC 2047). That is a generated-code defect that fails at `-O0` and
   `-O2`. Session 33 did not find it. Adopted verbatim, plus the permanent
   `cast_narrow_widen_handoff.c` regression.

2. **AArch64 FP spill *reload*** (`0c5134cc`). Session 33 rejected the whole
   AArch64 quartet because of an unrelated PhysReg-class panic. That was too
   coarse: the load path is the bit-preserving dual of the *already-shipped*
   `str dN,[slot]` store path. Adopted with Agent B's assembly gate.

3. **TCE unit test** for hidden `InlineAsm` outputs / `Intrinsic::dest_ptr`.
   Session 33 fixed the rewrite but did not pin it with a unit test.

4. **zlib-ng workload gate** (`tests/workloads/zlib-ng-2.3.3/run.sh`) and the
   postinline hang characterization. Keep.

5. **Honest distrust of Lev runtime percentages** on this VM. Keep.

### Where Agent B is worse / incomplete

1. **Rejected GlobalAddr CSE wholesale.** The multi-use pointer hazard is real
   on AArch64's *textual* address-alias peephole, but that is not a proof that
   substitution CSE is unsound. Session 33 already split **foldable** vs
   **must-materialize** so RIP/SIB `window[]` is never merged into a call-arg
   address. That is the correct perfection of `ed36c446`, not dropping it.

2. **Did not rewrite `loop_unroll.rs`'s parallel `replace_values_in_inst`.**
   The dest_ptr/asm-output hole exists there too. Session 33 fixed both.

3. **Did not adapt `8a052b9a` Select** even while ranking it "top next".
   Current `emit_select_impl` still stages every arm through x0/x1/x2, while
   the fused path already has `select_arm_reg`. Wiring the unfused path onto
   that helper is a 20-line, low-risk adaptation. Done this session.

4. **Did not adapt `9f304050` promotion-pool tail.** Current helper still
   reserves all of d24–d31 whenever *any* promoted F64 exists. RA assigns
   `PhysReg(48+i)` for the first eight promotions — the unused tail is idle
   by construction. Opened, with `CCC_NO_PROMOTED_FP_TAIL` for A/B.

5. **Textual `sxtb` peephole** from `8a052b9a` is still rejected: it inherits
   the incomplete mnemonic-write classification Agent B correctly flagged on
   the other ARM text passes.

## Perfected Lev set (this revision)

| SHA | Status |
|---|---|
| `ed36c446` CSE | **Perfected**: class-split substitution CSE + complete rewrite helpers |
| `ed36c446` dest_ptr | **Adopted** in TCE *and* loop_unroll + unit test |
| `0c5134cc` FP reload | **Adopted** (load dual of existing store) |
| `8a052b9a` Select | **Adapted** onto current `select_arm_reg` |
| `8a052b9a` sxtb | **Rejected** (textual provenance) |
| `9f304050` FP tail | **Adapted** with kill switch |
| Agent B no-home narrowing | **Adopted** (zlib-ng P0) |

Still not transplanted (need current-RA / MachineIR / qemu-aarch64):
`dd0fa7c5`, `9c48a277`, `e3b21b8f`, `1b4bac8b`, ARM text DSE/ldp/stp,
`8ef1978f` late clamp vectorizer.

## Follow-up

1. zlib-ng CTest 62/69 vs remaining postinline hang in `minideflate.c` (Agent B).
2. RA-01 RIP remat of `window`/`prev` for gzip `longest_match`.
3. qemu-aarch64 differential for Select + FP reload + promotion tail.
4. Extract marching-pointer `gep_uses_iv` from `8ef1978f` with real element size.
5. Expat / SQLite / glibc 2.44 / CachyMod — still RAM-bound in this harness.
