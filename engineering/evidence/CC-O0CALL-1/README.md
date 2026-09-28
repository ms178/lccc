# CC-O0CALL-1 — falsified hypothesis, repaired secondary-cache clobber

Review baseline: `6f8ace9c0bf98bb2c4df671342d89f983e0ab4c1`.
Historical proposal: `88c5d8e5a4f50f88a8bd35c29240d8534f7047ab`.

The historical `disable_regalloc` guard in `memory.rs` does **not** fix the
Csmith 20260945 reproducer on this baseline: both original and guarded
compiler binaries exit by SIGSEGV. Its proposed regression passes the broken
baseline, so it is not a discriminating regression. The guard is not adopted.

The reduced `call_secondary_cache_clobber.c` fails on the baseline both with
and without `CCC_NO_PEEPHOLE=1`. The pointer `&x` is cached in `%rcx` by a
comparison. Argument 4 then writes a scalar zero to `%rcx` without invalidating
that secondary cache. Argument 5 materializes the aggregate address via
`movq %rcx,%rax; movq (%rax),%r8`, reading through NULL. This is not a
multi-definition/SSA problem, nor an accumulator-address-load problem.

Repair: invalidate the secondary cache after a GP argument writes ABI slot 3,
before staging the next argument. Classification includes scalar, integer
pairs, and integer-containing aggregates; the next-iteration placement also
covers early-continue staging paths without discarding the current source
before it is consumed. No allocator policy or optimized-load gate is changed.

Initial validation (2026-09-28, fastbuild O1 j2, 2 GiB active swap):
- Reduced reproducer: baseline SIGSEGV; candidate exit 0; GCC exit 0.
- Full original Csmith reproducer, `-O0 -I/usr/include/csmith`: baseline
  SIGSEGV; candidate and GCC output `checksum = 39DEBCF9`, exit 0.
- Broader validation is recorded in the session portfolio review; this initial
  autosave is not itself a full-CI claim. No runtime performance claim.
