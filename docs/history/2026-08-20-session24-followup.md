# FOLLOW-UP WORK — session 24 (2026-08-20) — zlib SIGSEGV fixed; sqlite root-caused; boot-gate path

**Base:** `origin/main` @ `6d2ac62` (PR #140). Deliverable `/home/user/ms178-1.patch`
(snapshot S21). Everything below is validated unless marked OPEN.

---

## 1. DONE this session (validated)

### 1a. zlib-ng SIGSEGV — ROOT-CAUSED and FIXED (GEP-fold soundness)
`Store{val, ptr:GEP(base,off)}` const-offset folding had TWO soundness holes in
`src/backend/generation.rs`, crashing zlib-ng decompress (`gz_reset`/`inflateInit2_`):
1. `compose_const_gep_folds` ran BEFORE `propagate_stable_aliases`, so a GEP
   whose base was a *propagated alias* never composed to the ultimate base and
   dereferenced a never-materialised value. **Fix:** re-run composition after
   alias propagation.
2. `can_const_addr_fold` accepted ANY register base, but a register base dies
   across an intervening call (caller-saved clobber) → store dereferenced a
   dead register (`movq $0,(%r8)` after `deflateInit2_`). **Fix:** restrict
   `can_const_addr_fold` to **alloca bases only** (always re-computable via
   `lea`); register-based GEPs generate at their definition point.
**Result:** zlib-ng bit-exact round-trip at -O0 AND -O2 (5 MB random).
**Cost:** corpus 31218→31483 (+265 B) — the previously-"applied" register-base
folds were producing a CRASHING binary; a liveness-aware refinement could
recover those bytes (see §3).

### 1b. Full battery green on the fixed tree
959 unit · 50/50 correctness · 352+6 regression (known gcc-14-oracle set) ·
900/900 m32 differential fuzz · 750/750 slot-RMW fuzz · 180/180 alias fuzz.

### 1c. Tooling added
`LCCC_DBG_FOLD` / `LCCC_DBG_STORE` instrumentation (fold decisions + store base
resolution) used to pin the zlib root cause to the exact instruction.

---

## 2. OPEN DEFECT — sqlite crash (NEXT PRIORITY)

**Repro:** sqlite-amalgamation 3.50.4, `sqlite3_open` + CREATE/INSERT/SELECT,
lccc -O0. SIGSEGV in `sqlite3OsSectorSize` (`p->pMethods->xSectorSize(p)`),
`p->pMethods` (FIRST field of `sqlite3_file`) = NULL while the REST of the
struct holds valid pointers/flags. So the struct is initialised EXCEPT the
set-last `pMethods`.

**Distinct from the zlib bug** (that was a store through a clobbered register;
this is a NULL field that was either never stored or overwritten). Two
hypotheses to pursue, in order:
1. **Lost store:** the `p->pMethods = pMethods;` store at the END of `unixOpen`
   is dropped/mis-addressed. Instrument: instrument that specific store (it is
   `Store{val:<io_methods ptr>, ptr:GEP(p,0)}`) — check whether it is a
   GEP-fold candidate that my alloca-only restriction now correctly defers, or
   a separate store-addressing bug.
2. **Unhandled open error:** `unixOpen` fails partway (sets early fields, not
   pMethods) and the error is not propagated, so the caller proceeds with an
   unopened file. Check the error branch control flow for a miscompiled jump.
**Debug recipe:** gdb backtrace already captured (sqlite3OsSectorSize <-
sqlite3SectorSize <- setSectorSize <- sqlite3PagerOpen <- sqlite3BtreeOpen <-
openDatabase). Bisect by building sqlite with `-DSQLITE_OMIT_*` to shrink, or
creduce `sqlite3.c` on the open path.

---

## 3. Boot-gate path (32 KiB) — how to close the remaining ~9.5 KB

Gate: `_end <= 0x8000` → `.text` budget ≈ 23 330 B. Current lccc boot text
32 844 (corpus 31 483) vs gcc corpus 14 669 (2.13×). Dominant cost is SLOT
TRAFFIC (values homed to stack instead of kept in registers): lccc 2108
esp-slot refs vs gcc 373. Ranked levers to close it:
1. **Register-allocation pressure relief** (biggest): keep more values in
   registers so fewer round-trip the stack. The single largest structural
   lever; requires allocator work to reduce spills.
2. **Liveness-aware GEP register-base folds** to recover the +265 B: fold a
   register base only when no call intervenes between base def and use.
3. **Indexed-addressing (`(base,index,scale)`) emission** for array accesses
   (`can_indexed_addr_fold` exists; verify it is sound across calls before
   relying on it — same clobber class as the zlib bug).
4. Continue per-file hotspots: printf, string, video are the largest.

---

## 4. Standing environment notes (harness wipes EVERYTHING between sessions)
- swap: `sudo fallocate -l 8G /swapfile && chmod 600 && mkswap && swapon`.
- rustup stable minimal under `/opt/rust` (RUSTUP_HOME/CARGO_HOME there; keeps
  ~15k toolchain files out of the 10k-file workspace snapshot cap).
- build: `cargo build --profile fastbuild --locked -j2` (≈1.5 min).
- kernel: linux-6.18.44 tar.xz + 26 patches from
  ms178/archpkgbuilds/packages/linux-cachymod-6.18 (PKGBUILD order), `make
  olddefconfig && make prepare`, then the boot stubs (voffset.h/zoffset.h/
  utsversion.h/capflags/cpustr.h — see scripts/build_kernel_boot.sh). Kernel
  tree is NOT persisted (too large); re-extract each session.
- gcc-multilib + libc6-dev-i386 needed for the -m32 regression/fuzz binaries
  (they "cannot execute" if absent — environment, not code).
- sqlite-amalgamation-3500400 from sqlite.org/2025 (3530400 404s); driver
  `sqlite-test/sqmin.c` opens+CREATE/INSERT/SELECT.

## 5. Oracle / preference audit (unchanged, re-verified)
mold `-DMOLD_TARGETS='X86_64;I386'`, binutils pinned 2.47, git-HEAD
mold/wild, `-march=native` — all intact in tests/linker/setup_oracles.sh.
