# 2026-08-22 session 52 — rebase, Agent A/B audit, workload gates

Base: `a61e417a41697ccf60fbc723d9124f5a52869992` (PR #193). Latest
`ms178/lccc` main at session start.

## Agent A — agree in part, reject the RA formulation

Agent A correctly identified the zlib-ng `loop_iv_across_call` class:
phi-coalesce must not put a call-spanning backedge source in a
caller-saved home. Their guard

```text
caller_saved(dest) && liveness.live_across_any_call(src) → refuse
```

is the right *invariant* stated too coarsely:

- `live_across_any_call` is function-wide. A source live across a
  *different* call, with a call-free (def, copy) window, is refused even
  though inheriting a caller-saved dest home is sound.
- They do not keep the source eligible, so Phase 1 cannot give it its
  own callee-saved home.
- They do not allow inheriting a *callee-saved* dest across the call.

We keep the window-local `phi_window_clobbers_caller_saved` check:
refuse inherit iff a Call/CallIndirect/Asm/Memcpy/Va* sits between
source def and the latch Copy *and* the dest home is not in the
callee-saved pool. Eligible is preserved when the window clobbers so
Phase 1 can home `++i` in `%rbx`. Agent A's self-contained
`parse(argc, argv)` regression shape is already what we ship.

## Agent B — take IR hygiene, reject heuristic sledgehammers

| Change | Verdict | Why |
|---|---|---|
| `.base_ref` → `"base"` | reject | noise |
| `MAX_SMALL_INLINE_BLOCKS` 3→12 | reject | size; no oracle |
| drop `-Os` 40 cap | already on main | n/a |
| `split_call` on every function | reject | not the IV bug; inflates IR |
| `testl` operand swap | reject | both AT&T forms are legal TEST |
| GEP+0 on `&local` | **take, generalize** | all lvalues; GEP+0 is a no-op and poisons mem2reg after inlining |
| Load/Store through GEP+0 | **take** | complements lowering |
| mem2reg after fold2 | **take** | GEP+0 copies become promotable |
| i686 fold any dead GPR | **take** | existing deadness scan; skip ESP/EBP |

## Fixes in this patch (on `a61e417`)

1. **RA (correctness).** Phi-coalesce will not inherit a caller-saved
   home across a call in the (def, copy) window. zlib-ng CVE-2018-25032
   ×6 + GH-382.
2. **Lowering + simplify (correctness/quality).** `&x` is the alloca,
   not `GEP x, 0`. Load/Store of GEP+0 fold to the base. Extra mem2reg
   after post-inline fold2.
3. **Sema (correctness).** `int (*cb)(void*, T*)` parameters are typed
   via `fptr_params`, not as `int *`. Unblocks SQLite 3.53.4 amalgamation.

## Gates (this session)

| Workload | Result |
|---|---|
| zlib-ng 2.3.3 + ms178 patch, CTest | **69/69** (was 62/69) + minigzip roundtrip levels 1/2/6/9 |
| gzip 1.14 `-O3 -march=x86-64-v3` | **30/30** + compress/decompress roundtrip 1/6/9 |
| SQLite 3.53.4 amalgamation (`sqlite-src-3530400`) | compiles, links, CTE/float/IN self-check **exit 0** |
| glibc 2.44 tarball + `ms178-glibc.patch` | extracted and patched; internal TUs need generated config headers (no sourceware git). `glibc_memcmp` kernel compiles. Full glibc configure/make exceeds 2 GiB RAM. |
| `cargo test --lib` | **1078 passed, 0 failed** |
| regressions `loop_iv_across_call`, `addr_of_inline_mem2reg`, `fnptr_param_assign` | pass vs GCC |

## Not a miscompile (left as RA/ISel work)

- gzip `longest_match` stack-mem vs GCC (RA-01/RA-02 remat+RIP).
- Adler DO8 `sum2`/`n` on stack (RA-03/RA-06 reload-at-next-use).
- sqlite `yy_shift` cmp-replay slot-only (IS-09, conservative).
