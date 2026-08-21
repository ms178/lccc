# Session 7 (2026-08-21): struct_copy 3.1×→1.54×, i686 pair-load miscompile

## Base

Re-based onto `8393754` (PR #174, BitTest IR). Massive upstream progress since
#118: Tier-2 graph coloring default (P0-01b), RA-23/24/26/27, sema constraints
(P0-05), engineering/agent/BACKLOG.md is now the authoritative 170-item work
catalog. ALL session-6 i686 work survived the merge (verified by grep:
fnptr fold + linear seal, callee-saves shapes, cmp-mem fold, range fixes,
call *sym encodings). All 7 reproducers green post-rebase.

## Landed (single commit e8bcfcd)

### 1. IS-02 family: aggregate copy-forward — struct_copy 3.11× → 1.54×

The `build(tmp); memcpy(dst, tmp)` store-forwarding sub-pass was blocked
three independent ways on the canonical inlined-struct-return shape:

* **Stale param exclusion**: excluded "first params.len() entry allocas"
  positionally; after mem2reg deletes promoted scalar param homes the count
  excluded the first N *temporaries*. Now uses `func.param_alloca_values`
  (authoritative, maintained by lowering/mem2reg/tail_call_elim), positional
  fallback only when the list is empty.
* **Dest defined too late**: the destination GEP (`&g->particles[i]`) is
  emitted right before the memcpy — after the temp's first store — failing
  the def-before-first-use ordering check. Const-offset GEPs are pure
  address arithmetic: hoisted above the first root use when the base
  dominates (one hoist per run(); driver fixed-point handles chains).
* **Window over-rejection**: the dest-liveness window (zlib-ng fold_1 guard)
  rejected ANY instruction referencing a dest-rooted value — including the
  hoisted GEP itself. GEP/Copy/Phi are address-only; memory accesses through
  the aliases they create are still caught by the paths lookup on the
  accessing instruction. Load/Store/Memcpy/Call/etc. still fail closed.

All 4 struct-return memcpys in struct_copy now eliminated (was 0-1 of 4).

### 2. i686 pair load/store base-clobber MISCOMPILE (pre-existing)

Found by running the -m32 battery on the rebased tree: nbody and
struct_copy SIGSEGV at -m32 -O1+. The F64/I64 pair load through
`SlotAddr::Reg` emitted `movl (%eax),%eax; movl 4(%eax),%edx` when the RA
homed the pointer in %eax (PhysReg 6 = Phase 2e accumulator home, newly
reachable after the upstream RA work): the first load destroys the base and
the second dereferences the loaded LOW WORD. Fixes:

* load sites (memory.rs, emit_load_impl + emit_load_with_const_offset_impl,
  SlotAddr::Reg arms): order the pair so the base register is written LAST
  (base=%eax → load %edx-half first).
* store sites (emit_store_impl + emit_store_with_const_offset_impl): the
  value pair occupies BOTH accumulator halves so reordering can't help —
  stash an %eax/%edx base into the %ecx address scratch before staging.

25-line reproducer: 5-body force loop with `bodies[i].vx -= dx*bodies[j].mass`.

## Validation

991/991 unit, 50/50 correctness, 46/46 check scripts, regparm -O0..-Os,
x86-64 differential fuzz 147/150 (seeds 4/9/100 fail IDENTICALLY on
unmodified main — pre-existing, not investigated this session), **m32
differential fuzz 200/200**, benchmark output parity vs gcc at -m32 and
-m64 (nbody, spectral_norm, binary_trees, struct_copy, hash_table, qsort).

## Benchmark scoreboard after this session (VM, median of 5-9, -O2)

| kernel | ratio | note |
|---|---|---|
| struct_copy | 1.54× | was 3.11× at session start (21× in backlog) |
| expat_xml_scan | 1.90× | btq classify missing (OP-15/IS-06 wiring) |
| linux_find_bit | 1.86× | IS-11 andn+cmov chain |
| sqlite_varint | 1.77× | ISel branches |
| loop_patterns | 1.75× | vectorizer (OP-05) |
| zlib_ng_adler32 | 1.66× | RA-03 next-use |
| sieve | 1.26× | leave alone per backlog |
| hash_table | 1.20× | |
| gzip_crc32 | 1.07× | |
| qsort | 1.00× | |
| bitops | 0.69× | lccc WINS |
| matmul | 0.80× | lccc WINS |

## Notes for next session

* The 3 x86-64 fuzz seeds (4, 9, 100) mismatch on pristine main at -O2 —
  real miscompiles upstream should know about. Repro: tests/fuzz
  differential_fuzz.py, /tmp/fuzz_base kept the .c files (wiped; regenerate
  with --seeds 4:5 etc.).
* expat classify: GCC turns the xml_name_continue OR-chain into
  `andl $-33` (case-fold) + range + `subl $45; cmpb $50; btq $mask` bit
  cluster. range_check.rs handles pairs of compares; the btq cluster needs
  a new "sparse small-set membership" fold over the whole Select chain →
  BitTest IR (IS-06 landed the op + BT emitters; OP-15 is the wiring).
  This is the biggest clean structural item left on the parser row.
* struct_copy remainder (1.54×): GCC pairs the three field multiplies into
  mulpd (SLP over x/y + z scalar); lccc emits 3 scalar mulsd through
  slots. That's OP-05/IS-04 territory (vectorizer), not copy-forwarding.
* Kernel tree at /home/user/linux-6.18.44 was wiped mid-session (only
  include/generated survived); re-extract + re-patch per session-5 protocol
  before any realmode measurement.

## Session 7 continuation (post-wipe, same day)

Environment wiped mid-session; re-provisioned, re-based (main still
8393754), restored both session-7 commits from the snapshot.

### Landed additionally

3. **set_membership pass (OP-15, wiring IS-06)**: folds `x==C1 || x==C2
   || ...` CondBranch chains (>= 3 members incl. range_fold products,
   span <= 63) into GCC's classify idiom, final form two-branch:
   guard `ja miss` + BitTest branch. Kill switch
   CCC_DISABLE_PASSES=set_membership. 7 unit tests.

4. **THREE pre-existing miscompiles fixed** (all found by this work's
   validation):
   * redundant_ext removed byte re-extensions on upper32_zero alone
     (bits 8..31 not proven) — `(unsigned char)c >= 0xc2U` returned
     garbage at -O1. Byte form now requires is_byte; 16-bit twin
     hardened the same way.
   * redundant_ext movq→movl narrowing gated only on "next insn uses
     the 32-bit form" — a 32-bit read does not end a 64-bit live range
     (fuzz seeds 4/9/100: 64-bit hash upper half vanished). Now
     requires the SOURCE provably upper-32-zero; next-use kept as
     profitability filter. **Fuzz went 147/150 → 300/300 (O1+O2).**
   * emit_bit_test_reg_direct (IS-06) never zero-extended above the
     setc byte in 64-bit mode — result kept mask bits 8..63.
     Always movzbl now.

### Rebase checklist additions (now 9 reproducers)

8. `f(unsigned char c){return c>=0xc2U;}` at -O1: f(191) must be 0.
9. differential_fuzz seeds 0:150 at O1,O2 must be 300/300.

### Still open (next session)

* expat_xml_scan 1.87×: the bt clusters landed but the surrounding
  loop still spills; remaining gap is RA (segments) + the case-fold
  `andl $-33` trick GCC applies before the alpha range test.
* struct_copy 1.54×: SLP-pair the x/y field multiplies (OP-05/IS-04).
* set_membership currently keys on `>= 3` members; GCC also folds 2
  eq + 1 range. Measure before widening.
* BitTest emitters: the accumulator fallback path (non reg-direct)
  still stages mask+rcx with extra moves; only reg-direct was fixed
  for zero-extension — audit the fallback's movzbq too.

## Session 8 (2026-08-21, post-#175 re-base)

Base 594d1d5 (PR #175 = session-7 aggfwd + i686 pair fixes merged).
Re-applied patches 3-5 from snapshot cleanly.

### BitTest infrastructure overhaul

* **BitTest→CondBranch fusion** (both x86 backends): the fused-branch
  detector anchors on `BinOp{BitTest}`; terminators become `bt; jc`
  with zero materialization. Capability hook
  `supports_fused_bit_test_branch`, kill switch
  `CCC_NO_BT_BRANCH_FUSION`. i686 never-materialized mirror included.
* **4 more latent bugs fixed** (10 total found by this work so far):
  1. simplify built I64 BitTests on 32-bit targets → wide-int-path ICE
     (`(u64>>i)&1` at -m32 -O2, reproduced on pristine main). Legality
     gate; set_membership span cap 31 on -m32.
  2. i686 accumulator BitTest hardcoded `btl %ecx,%eax`, stale %ecx
     when the index had a register home ('/'-misclassification, m32).
  3+4. (session 7) redundant_ext byte/movq narrowing, BT zext.

### set_membership v2

Skip-tolerant clustering, case-fold merge (andl $-33), best-window
selection, head-prelude suffix parsing. One transform per call; driver
fixpoint. Tests now include an in-crate IR interpreter running
exhaustive 0..255 equivalence of the folded Expat chain.

xml_name_continue trajectory: 45 (no pass) → 50 (v1 single-block) →
42 (two-branch) → **28** (v2). GCC: 20; the remaining 8 are phi-copy
materialization (`movq $1,%r8` per arm + join copies) — RA/phi
coalescing, not classify structure. Layout now GCC-shaped:
case-folded alpha test → one bt cluster → tail compare.

### Process incident

During ICE triage a `git checkout 594d1d5 -- .` silently reverted the
COMMITTED session-7 fixes in the working tree; caught because the O1
classify reproducer failed right after. Rule: after any tree-wide
checkout, re-verify the reproducer sweep before continuing.

### Validation (final)

999/999 unit, 50/50 correctness, 46/46 checks, x64 fuzz 300/300,
m32 fuzz 200/200, regparm all levels, exhaustive classify parity
-m64/-m32, nbody -O0 exact, benchmarks output-identical vs gcc.
