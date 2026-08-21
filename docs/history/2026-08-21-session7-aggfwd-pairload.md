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
