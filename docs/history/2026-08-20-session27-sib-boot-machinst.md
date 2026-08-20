# 2026-08-20 (session 27) — i686 SIB indexed addressing, boot-corpus re-measure, MachInst boundary audit

**Base:** `origin/main` @ `08fd9c37` (PR #146; session-26 fixes merged via PR #144).
**Deliverable:** `/home/user/ms178-1.patch` (auto-saved), artifact S25+S26, git bundle.

## Item A — i686 SIB indexed addressing (IMPLEMENTED, validated)

The session-25 hooks (`supports_indexed_addr`, `emit_{load,store}_indexed`,
`supports_indexed_sym_base`, `emit_{load,store}_indexed_sym`) now have i686
emitters:

- **Register-base SIB**: `movX off(%base,%idx,scale),%dst_or_eax` and store
  dual.  Load path is staging-free (any base/index registers legal — the
  single instruction computes the address before writing).  Store path:
  immediate/register-direct values stage nothing; accumulator-staged values
  require base AND index in callee-saved GPRs (PhysReg 0..=3, staging never
  touches ebx/esi/edi/ebp).
- **Symbol-base SIB**: `movX sym(,%idx,scale),…` — folds never-materialised
  GlobalAddr bases straight into the memory operand.  Non-PIC only
  (`supports_indexed_sym_base = !pic_mode`); GOT/TLS/absolute symbols are
  refused upstream by `rip_rel_blocked`.
- **Recursive scale-chain resolver** (`build_indexed_gep_map::resolve_index`):
  peels Shl/Mul-by-power-of-2 and self-addition doubling (`Add(x,x)` = ×2)
  recursively under a total-shift ≤3 cap, reaching the homed loop IV under
  chains like `Shl(Add(i,i),2)` (= ×8).  Without this, the homeless
  intermediate (acc-flow) blocked the fold entirely.
- **Liveness**: `collect_folded_gep_links_all` now extends **base AND index**
  intervals of indexed folds (plus const-fold bases) to the consumer — also
  closes a latent ARM hole (arm previously extended only the index).
- **Guaranteed-fold offset-producer skipping**: when an indexed GEP folds
  into a LOAD, its single-use scaling chain (the peeled intermediates) is
  skipped at generation — otherwise the 3–5 instruction offset
  materialisation survives inside the hot loop.
  - SOUNDNESS GUARD 1: only load-only consumers qualify; indexed STORE hooks
    can refuse at the access (staging class), and the rematerialise path then
    needs the offset producers.
  - SOUNDNESS GUARD 2 (found via -O0 differential segfault): the walk must
    stop AT the fold's index value — `resolve_index` may return an UNPEELED
    terminal (a const-materialised offset: `movl $132,%edi; leal (%esi,%edi)`
    → `movl (%esi,%edi)` reads the index at the access).  Skipping that
    producer fabricated a garbage index (gt[33] load crashed).
- **Load→cmp-mem priority** (boot-corpus finding): enabling the indexed map
  made the load→cmp-mem exclusion fire for GEPs that never fold, demoting
  `cmpb $imm,(mem)` to `movsbl+cmpl` (+15 B in early_serial_console).
  cmp-mem outranks SIB: `load_cmp_ptrs` guards now keep such GEPs
  un-skipped at the GEP skip site, relax the cmp-fold condition, and mirror
  in `build_folded_value_set`.

**Validation:** sib.c differential vs gcc 6/6 (-fno-pic/-fpic × O0/O2/Os) ·
unit 959/959 · correctness 50/50 · regression 352+known-6 · m32 fuzz 900/900
· alias 360/360 · slot-rmw 750/750 · sqlite -O0/-O2 DDL + big harness
bit-exact · zlib-ng -O0 roundtrip.

## Item B — boot-corpus re-measure (kernel tree regenerated)

linux-cachymod-6.18.44 tree re-prepared (26/26 patches, `prepare_kernel_tree.sh`);
boot corpus (20 files, `-m16 -Os -march=i386 -mregparm=3`), text bytes:

| Configuration | Text |
|---|---|
| folds OFF (no regbase, no SIB) | 31 930 |
| regbase ON only | 31 672 (−258) |
| regbase + SIB + cmp-priority (current) | **31 658 (−272)** |
| gcc corpus (recorded) | ≈14 669 |
| gate budget (`_end ≤ 0x8000`) | ≈23 330 |

The gate still fails (setup >64 sectors): remaining gap ≈ 8.3 KB.  The
dominant cost remains slot traffic / register pressure (session-20 analysis
holds: 2 108 esp-slot refs vs gcc 373) — SIB/regbase are real but
single-digit-percent levers; the register-allocation quality gap is the
decisive one.

## Item C — MachInst/mature-boundary audit (session-26 follow-up)

- MachInst results resolve through the MAIN `reg_assignments`/slot state
  (`resolve_stack_vregs`): mature consumers and MachInst agree by
  construction; values without homes trigger a FULL fallback replay through
  the default path (no instruction loss; program-point bookkeeping
  replay-checked with assert).
- `reg_cache.invalidate_all()` after every flush prevents cache pollution.
- Empirical: regression + correctness identical with
  `CCC_MI_DISABLE_KINDS=all` except `check_machinst_fallback_replay` (the
  MachInst-specific test — expected).
- **Finding: `machinst_regalloc.rs` (635 lines) is vestigial** — referenced
  nowhere; the live pipeline never runs the MachInst-private allocator.
  Candidate for removal or future activation (documented, not deleted).

## Open items (session 28)

1. **Register-pressure/slot-traffic lever** — the decisive boot-gap closer:
   fewer slot refs via better RA (interval splitting, rematerialisation of
   cheap values, callee-saved promotion heuristics).
2. **i686 acc-cache audit** — complete the cache contract so the
   operand_to_eax hard gate (session-26, x86-64) can land on i686.
3. **x86-64 SIB indexed folds** — same emitters for the 64-bit backend
   (general codegen quality; boot corpus is i686-only).
4. Decide machinst_regalloc.rs fate (delete vs activate).

## Procedure state (DAUERAUFGABE honored)

Every validated fix this session committed + `ms178-1.patch` regenerated +
artifact copy + git bundle immediately after validation (S25, S26 entries in
`artifacts/SNAPSHOT_LEDGER.md`).  A mid-session harness crash destroyed the
first SIB commit (71cd10c5) and its saves; the work was reconstructed,
re-validated end-to-end (and improved: two soundness guards found on the
second pass), and triple-saved this session.
