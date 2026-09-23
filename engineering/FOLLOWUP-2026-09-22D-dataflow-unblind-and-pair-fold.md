# Follow-up: unblinding the i686 narrow-fact dataflow, the zext-pair compare fold, and the RA evidence that ends the census argument

Date: 2026-09-23 (session 62)
Base: `fe2fd653` (`ms178/lccc` main; S06 merged upstream as `2b6fb4d1`,
PR #593; carries the user's `101601d9` SIB-composition/power-of-two-width
and `e8a9fa35` i128-urem work — both red-teamed, both sound, no
supersession needed).
Head: `764324e1` — S07.

---

## 1. Red-team of the merged upstream (101601d9, e8a9fa35)

`101601d9` fixed two silent miscompiles with the right engineering:
power-of-two constants now read at the operation's width (the
`0xffffffffffffffff0000000000000001` → `x << 0` bug), the LEA relay fold
composes SIB addresses through a real model that fails closed on
two-scaled-register/three-register/two-symbol shapes, and the integrated
assembler now *rejects* a fourth comma field instead of silently
truncating. `e8a9fa35` fixes the i128 urem mask with per-class guards and
exhaustive regression tests. Both carry the push/pop RSP-shift lesson for
text peepholes (now documented in `i128_ops.rs`): a text peephole does not
model `%rsp` movement, so emitters must not push/pop around windows whose
slots the peephole matches. Adopted as a standing constraint; no
competitors' design mistakes found to supersede.

## 2. The two blind spots in `eliminate_redundant_zext_i686`

The whole-function narrow-fact dataflow (is_byte/is_16 per family) — the
pass that deletes redundant `movz*` self-extensions — was running nearly
blind on real command lines, for two independent reasons:

1. **Directives sat in `is_barrier()`.** lccc's boot command lines carry
   `-g`, so the emitter writes a `.loc` per statement; every `.loc` is a
   `LineKind::Directive`, every directive was a barrier, and every barrier
   cleared all facts. Between two statements of real code there is always
   a directive — the dataflow therefore never accumulated a fact across
   one. The same REG_NONE fallthrough also hit
   `invalidates_all_narrow_facts`, whose fail-closed arm wipes *all*
   facts for any line whose destination register does not parse.

   The law is now: **a directive emits no instruction** — it writes no
   register, no memory, no flags — so it fences nothing. `is_barrier()`
   documents this; labels, calls, jumps, returns and inline-asm regions
   stay barriers. (Precedent in-house: `is_metadata_directive` and
   `is_debug_location` already treated `.loc`/`.cfi_*` as transparent in
   the slot-window passes; the barrier list was the outlier.) The x86-64
   `redundant_ext.rs` pass already skipped directives explicitly and even
   documents the fixed label-vs-directive soundness bug — the i686 side
   now matches it.

2. **AND was not modeled as monotone.** `andl $imm` *reassigned* the
   facts from the immediate's range: any mask outside [0, 65535] dropped
   `is_16` even when the value was already known < 2^16. But AND can only
   clear bits — a narrow value stays narrow under every mask. The law is
   now `fact = fact || imm_covers_range` (monotone), which is what makes
   video-mode.c's `movzwl 60(%esp),%eax; andl $-32769,%eax;
   movzwl %ax,%eax` finally lose its identity re-extension (the mask
   0xFFFF7FFF spans bits above 16 and used to kill the fact).

Measured on the boot corpus (21 `arch/x86/boot` TUs, identical command
lines): **5460 → 5442 instructions, 24368 → 24290 bytes** vs gcc's
3461/14270 — including recovering every deletion of a now-revoked local
peephole attempt (see §4).

## 3. `fold_zext_pair_reg_compare` (new, `CCC_NO_ZEXT_PAIR_CMP_FOLD`)

    movz/sext S1,%A; movz/sext S2,%B; cmpl %A,%B; jcc/setcc
      →  cmp{b,w} OP1,OP2; jcc/setcc      (both extensions deleted)

Each compare operand is the extension's **memory source verbatim**
(base-only indirect — the compare re-reads the byte-identical address
expression, fault-equivalent, unchanged because only directives/nops sit
between) or the **narrow view of its destination**. Flag law, unit-tested
at the wrap rows: ZF and CF are width-invariant for zext/zext pairs (both
operands in [0, 2^w)) and for sext/sext pairs (sign-extension is monotone
in *both* signed and unsigned order — proved via the pattern-map
monotonicity argument in the pass comment); **signed readers refuse
zext/zext** (the 32-bit view compares two non-negative values while the
w-bit view compares two's complement — they disagree exactly on the
top-bit row); **mixed zext/sext refuses outright** (no condition class
survives both). Adjacency scans hop directives, never labels.

Honest measurement: the pass fires **zero times** in the v6.18.52 boot
corpus — its one candidate shape (`movzwl %bx,%edi; movzwl 24(%esi),%eax;
cmpl %eax,%edi; jl`) is a signed reader and the refusal is correct. It is
kept because the law is fully pinned (ten unit tests, including an
asm-routed encoding test per the S59 lesson) and the shape class is what
the RA work (§6) will surface once the staging relays that break its
adjacency are gone. The runtime guard (§5) exercises the shape end-to-end
so the law cannot silently rot.

## 4. The revoked attempt (do-not-retry ledger)

A local (adjacency-walk) `delete_masked_self_zext` was built first —
zext-def + AND-run + self-zext, hopping directives. Once the dataflow's
two blind spots were fixed, the whole-function pass subsumed it *and
found more* (barriers other than directives aside, the dataflow tracks
facts through copies and arbitrary clearing ops). Revoked before commit;
the whole-function pass is the architectural home of this transform.

## 5. Guard corpus extension and validation

`tests/regression/i686_narrow_cmp_fold.c` gains the pair shapes:
u16-vs-u16 and memory-vs-register comparisons across the top-bit row
(0x7FFF/0x8000/0xFFFE), each function driving signed *and* unsigned
readers — a wrong fold flips the exit-status checksum. `EXPECT_A`
569 → 683, re-derived from gcc `-O0/-Os/-O2` agreement.

- `cargo test --lib`: **3191/0** (ten new pair tests, dataflow pins).
- Runtime guard: rc=0 (lccc =O2/-Os = gcc -O0/-Os/-O2).
- 256-value byte-map sweep: **0 divergences**.
- Boot census: 5460 → **5442 insn / 24290 B** (gcc 3461/14270).
- `ci_local.sh --fast`: **60 passed, 0 failed, 3 skipped — ALL GATES
  GREEN** (one gate more than S06's 59: upstream added it).
- Full CachyMod 6.18.52 rebuild by lccc/lccc-ld: SUCCESS; boot gate
  `_end = 28928` (was 28944 pre-merge; the −16 B is this session's boot
  work; the kernel-wide `.text` 11894550 → 11879078 also carries the
  upstream i128/SIB improvements).
- `qemu_boot_test.sh`: **16/16 PASS, zero WARNING/Call Trace/ACPI noise**.

## 6. The census argument is closed: everything left is the RA

The godbolt/local oracle work this session closed the narrow-compare
front: cpucheck.c now compiles to **equal `movz*` counts vs gcc** (1/1),
and the corpus mnemonics are at or inside striking distance. What remains
is one defect class with a number on it:

    cpucheck.c (i686 -Os, boot flags):   lccc 350 insns   gcc 157
      slot-stores:   lccc 43    gcc 0
      slot-loads:    lccc 30    gcc 0
      reg-relay movl: lccc 14    gcc 0

Corpus-wide: store-to-frame **515 vs 46**, load-from-frame **354 vs 67**,
reg-to-reg movl **447 vs 226**. Not a single remaining peephole class —
pure register residency. The `LCCC_DBG_RA` dump for `check_cpuflags`
names the mechanism:

    values=22 assigned=10
      v36 [0,32]  slot uses=21 elig=false   ← the accumulator, whole-range
      v10 [8,10]  slot uses=10 elig=true    ← hot, short, spilled anyway
      v13 [10,16] slot uses=10 elig=true
      v16 [12,14] slot uses=10 elig=true
      v21 [16,18] slot uses=20 elig=true
      r0 [1,30] r1 [6,17] r2 [5,17] r3 [7,17] …  ← pool spent on long ranges

The greedy scan allocates by live-range **start point**, so long low-use
ranges take all ~6 physical registers before the short hot values are
considered — values the pass itself marks `elig=true`. S59 falsified the
naive flip (`CCC_NO_I686_ACCUM_NOHOME`, +391 B); the defect is not the
no-home policy but the **allocation order and the absence of eviction**.

Next session's opening move, with a falsifier per step:
1. Order candidates by spill cost (uses per live-range unit), not start
   point — hot short values claim registers first.
2. Add second-chance eviction: a waiting high-cost range may evict a
   resident low-cost long range (paying its reload) when the cost model
   says so; the segments/holes infrastructure and `RA-EXPLAIN` dumps
   already exist for the A/B.
3. Keep the accumulator as the canonical repro: `orl` in the loop must
   become register-resident (gcc: `orl $130, %ebx`, one instruction vs
   lccc's five-line load/modify/double-store/reload staging).

## 7. Godbolt oracle status

Self-contained distillation continues on the guard corpus (gcc 16.2,
clang 23.1.0, icx latest, icc 2021.10; artifacts under
`artifacts/godbolt-s06/` — lccc 90 vs gcc 128/161/161/198 still stands as
the outright win). Kernel TUs are not self-contained; they distill via
`scripts/boot_size_oracle.sh` + per-class census on this machine
(§6 numbers: cpucheck lccc 350 vs gcc 157 insns, movz* at parity 1/1).

<!-- src: engineering/FOLLOWUP-2026-09-22D-dataflow-unblind-and-pair-fold.md -->
