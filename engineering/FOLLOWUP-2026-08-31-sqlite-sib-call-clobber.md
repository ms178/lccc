# Follow-up — 2026-08-31: sqlite3 CREATE TABLE SIB call-clobber

**Base.** `ms178/lccc` main `1a9bf60` + local `7b5f265` (pp-join-map-rc).
sqlite 3.53.4 amalgamation, Arch PKGBUILD features, `-O2 -fPIC`.

## Bug

`CREATE TABLE t(a);` SIGSEGV in inlined `sqlite3DbStrNDup` from
`vdbeChangeP4Full` (`0x4c0800`). Post-memcpy:

```
movslq %r10d, %r10
movb   $0, (%rbp, %r10, 1)    ; 0x4c0b2f
```

`%r10` was the I32 Copy dest of `n` after the `n==0` diamond
(`if (n==0) n = sqlite3Strlen30(zP4)`). Lookaside + `memcpy` clobbered it.
The i64 widen of `n` was spilled to `0x30(%rsp)` and reloaded into unused
`%r11`; SIB used the stale `%r10`.

## Root cause

`resolve_index` peels Cast/Shl/Mul/Add (not Copy). Folded index = multi-def
Copy dest. RA stretch multi-def merged `required = [GEP.start, Store]`.
GEP is **after** memcpy, so memcpy ∉ required → not `call_spanning` →
Phase 2 caller-saved `%r10`. Copy/Cast not adjacent → not `never_materialized`.
Single-def last-segment stretch *would* have covered Cast→Store including
the call; sqlite is the multi-def case.

Do **not** peel Copy in `resolve_index` (ParamRef `n` dies at Copy / `%r15`
reused for `p->db`). Do **not** refuse all caller-saved SIB indexes
(vsprintf digit loop wants `%edx`).

## Fix

`src/backend/regalloc.rs`: multi-def required span starts at the last
IR-visible **segment** end of the index when it is not live at GEP.start
(the fat envelope of the join block already runs through memcpy+GEP+Store,
so `bounds_of[idx].end` is not the Cast). That fills `[Cast, Store]`
including memcpy. A covering segment at the GEP keeps historical
`[GEP, Store]` (vsprintf digit latch).

## Tests

- `phi_coalesce_tests::multi_def_peeled_index_covers_call_before_gep`
- `tests/regression/sqlite_strndup_sib_call.c` (`-O2 -fPIC`)
- sqlite amalgamation: `CREATE TABLE` + INSERT + SELECT

## Still open

- PF-17: 15 loop-rotate miscompile shapes; rotate stays opt-in
  (`CCC_LOOP_ROTATE=1`) until they are root-caused.
- ZSTD P0: userspace oracle MATCH on `90b6d87`; QEMU boot still the gate.
- Optional belt: refuse SIB if the index home is not live-after-call
  (needs call_points on X86Codegen). Not required once RA is sound.
- Do not enable MachInst on large loops; gzip `longest_match` stack-mem
  must not rise.
