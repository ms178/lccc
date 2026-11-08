# SIMD load forwarding: pointer provenance and remaining work

## Findings and choices

The SIMD temporary load-forwarder derives object roots to decide whether an
intervening write can clobber a loaded value. Its root worklist treated every
`Cast` as pointer-preserving. A pointer truncated to U32 and then converted
back is not necessarily the address of the original alloca. The existing
`ir::provenance::chain_step` contract already follows only Ptr-to-Ptr casts.
The SIMD analysis now uses that same cast restriction. `Copy` remains an
identity and is still followed; the frontend erases some pointer/integer
conversions, so this is defensive IR hardening, not a claim to solve all
integer-pointer provenance problems.

An IR regression demonstrated an actual rewrite decision: a load from an
absolute address followed by a write via a truncated alloca pointer was
forwarded (one rewrite), although the write could alias the load (zero
required). Before the fix this assertion failed; afterward it passed. A
five-case test also retains the Ptr-to-Ptr positive case and rejects casts
involving U32 or U64. We did not reproduce a C-level misscompile.

A second IR regression with an alloca-backed source still failed after the
cast change: an unrooted write was treated as disjoint based on nonescape and
absence from the source's address-influence graph. Integer-derived pointers
can have an unknown address even without a syntactic edge from the source.
The forwarder now treats unrooted writes as potentially aliasing and removes
the no-longer-used per-function owner graph construction. Proven distinct
rooted writes remain forwardable; the existing positive controls cover them.
This is intentionally conservative: a missed forward can cost generated-code
performance. A future precision improvement needs an explicit provenance and
escape proof, not pointer-ID inequality or a missing graph edge.

## Verification and limits

* Fastbuild (`-O1`, warnings denied, one memory-bounded Cargo job on a 3.8 GiB
  host, active 4 GiB swap), audit census and formatting passed.
* Both new negative IR regressions failed before their respective fixes and
  passed after; vector-temp unit suite 47/47 and snapshot suite 5/5 passed.
* AVX FP contracts: 72 build/runtime/frame checks passed with LCCC, GCC 16.2,
  and Clang 23.1.5 on x86-64 and i686. Snapshot-alias fixture: ten AArch64/RV64
  QEMU executions across five optimization levels passed. These do not prove
  concurrency behavior or native hardware performance.
* Comparing the commit immediately before the unknown-write guard (`f8884be`)
  to the guarded version (`876a7fc`), all 24 benchmark kernel assembly files
  were byte-identical. `avx256_fp_private_frame.c` objects were byte-identical
  as well, at 234 disassembly rows for LCCC's `avx_private_frame`; same-run
  GCC 16.2 and Clang 23.1.5 had 45 and 47 rows. These static counts are not
  runtime measurements; the samples do not rule out regressions elsewhere.
* ICX 2026.2 was unavailable locally: the authenticated Intel provisioner
  failed because the required host is outside the sandbox network allowlist.
  Hosted ICX integration is not a substitute for differential execution of
  these particular cases.

## Remaining work

Run the same-tree full local fast suite and hosted CI, repeat architecture
coverage and measurements after rebasing onto the final main, and retain a
gated snapshot. Investigate the existing AVX frame/round-trip gap using real
workloads and physical-host timings before proposing a more permissive alias
proof. Audit the other vector-temp transformations and worklist root rules,
including integer laundering, multi-definition and inline-asm boundaries;
these tests do not establish complete correctness. Real AArch64/RV64 atomic
interop and memory-model testing and the broader backlog remain open.
