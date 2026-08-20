# 2026-08-19/20 (session 21) — levkropp audit, adapted wins, AArch64 RA fix, first full validation gauntlet

**Base:** `origin/main` @ `53b425409` (Merge PR #136 — session-20 volatile work landed upstream).
**Snapshots:** `S01..S06`, latest `/home/user/ms178-1.patch` (76 039 bytes, APPLIES-CLEAN).

## levkropp/lccc Aug 19, 2026 — audit verdicts (10 commits)

| Commit | Verdict |
|---|---|
| 51285cd5e alias engine + GVN float CSE | **ADAPTED**: GVN F32/F64 load CSE enabled; `passes::alias` (LoopFrames, same-frame `forms_disjoint`) built on OUR engine (defs-map, target-aware byte_size). |
| 3fa030e50 redundant_loads | **ADAPTED with hardening**: volatile loads never merge (his base predates IR flags), AtomicLoad kills availability, `source_spans` kept parallel (his version desynced them — backend indexes spans by position under -g). |
| bbab1d125 DCE companion | **ADAPTED** (runs after redundant_loads). |
| 885f129ca quadratic_sr | **ADAPTED with span-sync fix.** Soundness re-proven: t(t+1) always even → division exact → trunc semantics irrelevant; identity holds mod 2^32 (ring homomorphism). Fires on x86-64 (div_by_const lowers /2 to the correction chain); verified vs gcc incl. negative t. i686 keeps SDiv (div_by_const disabled there) so the matcher legitimately stays cold. |
| 6e9552278 copy-loop vectorization | **REJECTED — already present** on this main in a more mature, restrict-aware form (`store_val == load_dest`, `roots_proven_distinct`). |
| a0aa48d70 docs | Task-file notes only. |
| 0c5134cc4, dd0fa7c58, 8a052b9a1, 9f3040507 (AArch64 quartet) | **BLOCKED, not adapted.** The AArch64 backend crashes on the baseline (see below); adapting RA-pool changes onto a crashing baseline violates the validation mandate. Re-fit against our 293-commits-newer RA is the follow-up once the crash is fixed. |

## Bugs found & fixed this session

1. **i686 pre-existing miscompile (alias-fuzz scenario 7):** copy aliases survived
   single-operand RMW ops (`negl/notl/incl/decl/bswap %reg`): `classify_line`
   derives dest from the post-comma operand — these have none — and
   `line_writes_reg_implicitly` lacked the family. `movl %edi,%eax; negl %eax;
   movl %eax,(%esi)` forwarded the un-negated `%edi`. Fixed in the implicit-
   writes table (heals propagate_reg_copies AND forward_slot_loads); unit
   regression `unary_rmw_breaks_copy_alias`.
2. **AArch64 indexed-addressing register overwrite (RA-invisible consumers):**
   `[base, index, lsl #N]` consumes the index at the Load/Store position, but
   the IR records no use there → allocator reused the register (`fa[i]` stores
   landing on `fa[seed]`). Fixed at the root: `RegAllocConfig::folded_index_uses`
   generated from the SAME `build_indexed_gep_map` the emitter uses; live
   intervals (fat + hole-aware segments) extended to the consumer's end.

## Open defects (characterized, fixtures in tests/gauntlet-repros/)

1. **x86-64 -O0-stable miscompiles in real code** — the session's #1 follow-up:
   - zlib-ng 2.3.3: gz_reset `mov %r15d,(%r15)` (zero materialization clobbers
     the live base pointer); compat-O2 decompress crashes in inflateInit2_
     (`mov %rsi,0x50(%rbx)`, stale rbx across the zalloc fn-ptr call).
   - sqlite 3.53.4 amalgamation: compiles (260k LOC, ~5 min) but openDatabase
     SIGSEGVs at -O0 AND -O2; register corruption precedes sqlite3SchemaGet
     (rdi = rodata-like 0x54e36f at the call).
   - Both survive `CCC_DISABLE_PASSES=all` → frontend/lowering/backend-emit
     class. Isolated single-pattern repros all PASS → creduce on the real
     sources (or amalgamation bisection via SQLITE_OMIT) is next. Prime
     suspect family: constant materialization / staging clobbering live
   registers (the same disease the i686 RMW fix and the AArch64 folded-index
   fix each caught in one habitat).
2. **AArch64 backend panic** `invalid ARM register index` (alu.rs emit3 with an
   FP-pool PhysReg on a GP binop) when float+double arrays + union bitcasts
   mix — pre-existing, repro = aarch64_fuzz scenario 0 (20/50 seeds). Blocks
   the levkropp AArch64 quartet.

## Validation gauntlet results (PKGBUILD versions from ms178/archpkgbuilds)

| Target | Build | Runtime |
|---|---|---|
| gzip 1.14 | ✅ CC=lccc | ✅ **`make check` 30/30 PASS** |
| expat R_2_8_2 | ✅ CC=lccc | ✅ 7-doc XML differential == gcc build |
| zlib-ng 2.3.3 + ms178-1.patch | ✅ static+shared, 0 errors | ⚠️ compress ✓; decompress crash (open defect 1) |
| sqlite 3.53.4 amalgamation | ✅ (260k LOC) | ❌ openDatabase crash (open defect 1) |
| linux-cachymod boot code | ✅ (earlier this session) | gate: _end=0x99c0 (6592 over, corpus 30950 vs gcc 13327) |
| glibc | no package in archpkgbuilds (394 checked) | corpus files remain the proxy |

## Battery (final tree)

933 unit / 50 correctness / 180 m32 fuzz (O0/O2/Os) / 256 alias adversarial /
alu torture MATCH / 21 regression / triangular differential MATCH /
aarch64 fuzz: 0 value mismatches (20 compile-crash seeds = open defect 2).

## Follow-up ranking

1. x86-64 -O0 miscompile family (zlib-ng gz_reset as the smallest live repro:
   full function, ~10 lines + one caller) — creduce, fix, re-run gauntlet.
2. AArch64 PhysReg-class panic → then adapt the levkropp quartet with the
   qemu-aarch64 differential as the gate.
3. Boot-gate 6592-byte plan from session 20 (unchanged).
