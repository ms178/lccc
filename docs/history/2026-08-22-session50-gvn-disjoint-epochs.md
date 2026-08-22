# Session 50 — GVN per-object epochs (OP-32) + battery hardening + IV-across-call miscompile found

Base: `ms178/lccc` main `a06c720` (PR #191, which upstreamed the session-49
dedup + N-accumulator work). Build: `fastbuild`, Rust `-O1`, no LTO, two jobs,
8 GB swap. Host: constrained VM, no PMU.

## 1. GVN per-object epochs for disjoint allocas and restrict parameters (OP-32)

GVN's load CSE / store-to-load forwarding previously keyed per-object epochs
ONLY for named globals: a store through any other pointer (alloca-derived or
parameter-derived) bumped the global generation and invalidated every cached
load.  This session extends the mechanism to two more provably-disjoint object
classes:

- **Non-escaping allocas** — an alloca whose address provably never leaves the
  function (`find_nonescaping_allocas`: owner-set fixpoint through
  address-preserving ops, escape on Store-value / call / asm / terminator).
  With no escape, every pointer that can reach the alloca is tracked by GVN,
  so a store through it bumps only that alloca's epoch.
- **`restrict` pointer parameters** — `find_noalias_params` reads
  `IrParam.noalias` (plumbed from `restrict` by the frontend).  A restrict
  parameter cannot alias any other object, so its stores invalidate only its
  own epoch.

Root propagation (`track_ptr_base`) is single-source and fail-closed:
`GlobalAddr` / non-escaping `Alloca` / noalias `ParamRef` roots, propagated
through `GetElementPtr`, pointer-to-pointer `Cast`, `Copy`, and integer-width
pointer arithmetic (`Add` with exactly one rooted operand, `Sub` with a rooted
left operand).  Phis/selects with mixed bases, loads of pointers, non-pointer
casts and two-pointer arithmetic stay `Unknown` and fall back to the global
generation.

Soundness (re-derived; the same facts as the session-42/45 alias work): two
allocas are distinct frame objects; an alloca and a global are frame vs static
storage; an alloca and any parameter are distinct (a parameter's value is
fixed at entry, before the frame exists); a restrict parameter is disjoint
from everything by contract; distinct globals are disjoint (pre-existing GVN
assumption, GNU aliases canonicalized).  Crucially, stores through `Unknown`
pointers still bump the global generation, which invalidates every load —
including alloca-rooted ones — so a non-escaping alloca cannot be reached by
an untracked pointer.

### Evidence

```
int f(int n, int *restrict x, int *restrict y) {
    int s = 0;
    for (int i = 0; i < n; i++) { s += y[i]; x[i] = i * 2; s += y[i]; }
    return s;
}
```

Base emits two `movslq (%rdx,%r12,4)` loads of `y[i]`; with the change the
second load is value-numbered to the first (one memory load per iteration
eliminated).  The plain (non-restrict) control still reloads `y[i]` after the
`x[i]` store, and `f_plain(n, x, x)` (aliased) is runtime-correct — pinned by
`tests/regression/check_gvn_disjoint_epochs.sh`.

Validation: **1075** unit, **404** regression (+1 new) / 2 skip, **50/50**
correctness, **600/600** O2/O3/Os differential fuzz, 36-file benchmark/pattern
corpus byte-identical vs base (the optimization targets allocas/restrict
shapes the corpus mostly lacks; the win is structural, not corpus-visible).

## 2. Battery hardening

The gzip runner's `make check -j2` was **not** the flaky part — the actual
root cause of two `make check` failures (one gcc, one lccc, both 30/30 in
isolation) was a full `/tmp` tmpfs (a leftover 846 MB worktree build).  Two
fixes:

- `tests/workloads/gzip-1.14/run.py` now runs `make check` serially and
  retries once before failing (the gzip suite's `timestamp`/`atime` tests are
  filesystem-granularity-sensitive and must not race under `-j`).
- Battery runs are documented to use a disk-backed `TMPDIR` (`/home/user/tmp`)
  instead of the 993 MB tmpfs.

Battery on the modified compiler: **gzip 1.14 30/30 for both lccc and gcc**,
byte-identical streams (lccc ~1.74× gcc geomean on compression — the known
codec gap, reported honestly); **zlib-ng 2.3.3 roundtrip byte-identical,
62/69 CTest pass**.

## 3. Pre-existing miscompile found: loop IV carried in a caller-saved register across a call

The 7 failing zlib-ng CTest cases (`CVE-2018-25032` ×6, `GH-382`) are a real
lccc miscompile, **pre-existing at `a06c720` and independent of this session's
GVN change** (a GCC build of the same zlib-ng passes 100%; the base lccc build
fails identically).

Root cause: in

```c
for (int i = 1; i < argc; i++)
    if (strcmp(argv[i], "-m") == 0 && i + 1 < argc)
        mem_level = atoi(argv[++i]);
```

the `++i` SSA value (`i1 = i + 1`) is both the array index and the loop
induction value feeding the backedge.  The x86 backend computes it in `%edi`
BEFORE the `strtol` call (for `argv[++i]`), then the shared loop latch does
`movl %edi,%eax; addl $1,%eax; movq %rax,slot` — but `%edi` is caller-saved
and clobbered by the call, so the latch stores a garbage induction value and
the loop runs past `argc` into `environ` (zlib-ng's minideflate then tries to
`fopen("E2B_TEMPLATE_ID=...")`).

Standalone reproducer: `tests/gauntlet-repros/loop_iv_across_call.c`
(expected `1 -15 2 6 0 0 4 1 1 1`, lccc gives `1 0 0 6 0 0 0 1 1 1`).

The fix belongs in the backend latch/register allocation: a value live across
a call must not ride a caller-saved register, or the latch must reload the
induction slot instead of trusting `%edi`.  This is the #1 follow-up (see
BACKLOG); it is deliberately NOT patched here because it is a pre-existing RA
defect that needs the full multi-session battery, not a rushed change.

## Follow-up

1. **Loop-IV-across-call miscompile** (above) — top priority; the reproducer
   is committed in `tests/gauntlet-repros/`.
2. Re-run the zlib-ng full CTest after fixing it (expected 69/69).
3. The GVN epochs could extend to `noalias` parameters used via GEP chains in
   the vectorizer's `VecLoad*` address space (the same `ptr_base` facts).
4. glibc 2.44 and expat/sqlite end-to-end gates remain open — the pinned
   expat 2.8.2 / sqlite 3.53.4 tarball checksums are in
   `tests/benchmark/WORKLOAD_PROVENANCE.md`.
