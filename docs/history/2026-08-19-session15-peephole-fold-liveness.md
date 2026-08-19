# 2026-08-19 (session 15) — i686 memory-fold liveness soundness fix

**Base:** `origin/main` @ `33dd2117` (unchanged — session 14 not yet merged upstream)
**Snapshot:** `/home/user/ms178-1.patch`

## Re-base

Upstream still at `33dd2117` (PR #128). Session 14's two doc commits were
recovered from the artifacts bundle; `main` is them + this session's fix.
Kernel + archpkgbuilds re-extracted/re-patched (wiped again); boot baseline
re-measured at 33526 bytes total object text.

## The fix: fold_memory_operands must prove the loaded register is dead

The i686 peephole's memory-operand fold rewrites `movl SLOT,%eax; cmpl
%eax,%edi` → `cmpl SLOT,%edi` and NOPs the load. It never verified that %eax
is not read again. `propagate_reg_copies` extends a slot load's register
across several consumers:

```
movl SLOT,%eax; movl %eax,%edx; cmpl %edx,%edi; subl %edx,%ebp
  →  movl SLOT,%eax; cmpl %eax,%edi; subl %eax,%ebp   (copy now dead, removed)
```

The fold then deleted the load, leaving `subl %eax,%ebp` reading a **stale
register**. The m32 differential fuzz caught it (seeds 0/116 — rotate +
select + div loops) the instant a within-32-bit `Cast` hazard relaxation made
the cast result register-resident.

### The fix

Before folding, scan forward from the consumer:
- **continue past conditional jumps** — their fallthrough (a select arm) may
  still read the register;
- **stop (safe) at** a label / call / ret / directive — the accumulator cache
  is invalidated there, so the loaded value is provably dead (the emitter
  reloads);
- **stop (safe) at a pure write** to the register — `movl %src,%reg`,
  `movl $imm,%reg`, `leal ADDR,%reg`, `popl %reg`, `setCC %reg` (a
  read-modify-write like `addl %ebx,%reg` READS %reg and is correctly unsafe);
- **unsafe** on any read (explicit or implicit, e.g. `idivl` reads %eax:%edx).

Mirrors the x86-64 backend's proven `fam_read_after` guard (which already
does this — the i686 fold was the only backend missing it).

Regression test: `tests/regression/check_memory_fold_liveness.sh` (the exact
rotate+select+div program, run + exit-code compared against gcc).

## Result

- **Size-neutral**: 33526 bytes total boot-object text before and after (the
  pure-write refinement recovered the one fold the naive scan blocked).
- 918 unit tests, 50/50 correctness, 342+6 regression, **600-case i686
  differential fuzz clean**.

## Structural status (the honest big-picture)

The within-32-bit cast hazard relaxation is now **sound** (verified: red_f2
197=197 and 600 fuzz cases with relaxation + fix), but it nets **+100 bytes**
on the boot corpus alone — a 16-bit-mode frame/encoding interaction (fewer
callee-saved pushes change slot offsets and thus instruction lengths), not an
allocator-priority issue. It needs the caller-saved-priority/frame work to be
a net win. This session's fix removes the last *correctness* blocker, so the
relaxation + `%eax` allocatability can now proceed on size, not soundness,
grounds.
