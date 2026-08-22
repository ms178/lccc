# Session 56 — Agent B/C patch audit + glibc 2.44 deep dive (2026-08-22)

## Executive summary

Rebased on `95015a2` (all prior session work merged upstream as content-
identical). Audited two competing patches, folded the keepers, rejected the
regressions, and pushed glibc 2.44 from "libc.so links" to "glibc's own
binaries (utmpdump, sprof) execute against the lccc-built libc.so + ld.so".

**Gates:** 406/406 regression corpus · 1079/1079 unit tests · all previously
failing glibc math TUs compile · sotruss-lib.so links · utmpdump//dev/null and
sprof run (exit 0) under our ld.so.

## Patch audit verdict: Agent C is the better patch

### Agent C (`rebase-new-main`) — ACCEPTED with rework
Surgical soundness fixes for the *silent-zero-fabrication* family (the same
disease class as the session-26 operand_to_rax hard gate), with correct root
causes and minimal blast radius:

1. `operand_to_rcx` last-resort `xorl %ecx,%ecx` → route to `value_to_reg`
   (torture 20000412-6.c: `&buf[k]`'s remat-GlobalAddr base became 0,
   `buf+3` decoded as integer 6).
2. `value_to_reg` None-arm → materialize rematerialisable GlobalAddr instead
   of panicking (absolute-address loads, GEP bases, fused int-madd shapes).
3. High-quality torture triage (T01–T08) with minimal repros.

**Rework applied on top of C's version:**
* TLS checked BEFORE `needs_got_for_addr` in the new
  `emit_global_addr_into_reg` — C's order emitted a plain GOTPCREL load for
  PIC external TLS symbols (latent wrong-address bug).
* Reused the existing `get_defining_instruction` idiom instead of a second
  hand-rolled function scan; sec-cache invalidated when %rcx is clobbered.

### Agent B (`torture-test-suite`) — ONE keeper, rest rejected
* **ACCEPTED:** bare `alloca` as a GNU-mode builtin (sema map, type_checker,
  DynAlloca lowering). Auto-hardened by the existing `is_defined` user-
  override guard; regression covers both the builtin and override paths.
* **REJECTED — stale baseline (already in main, would double-apply):**
  `&var` GEP+0 elision, simplify.rs Load/Store-through-GEP0 folds, the extra
  `promote_allocas_with_params` call, both `encoder/core.rs` files
  (byte-identical to main).
* **REJECTED — previously-rejected perf knobs resubmitted without workload
  evidence** (BACKLOG §4 explicitly lists them as lost experiments):
  `MAX_SMALL_INLINE_BLOCKS` 3→12, removal of the -Os single-site 40-cap,
  `split_call_spanning_ranges` on every function (`blocks.len() > 0`).
  The claimed 1389/1514 torture rate is unattributable across these knobs.

## New fixes beyond the audit

4. **hidden_proto inlining** (`inline.rs`): callee map registers gnu89
   extern-inline bodies under their `__asm__` label too; call-site accounting
   sums both spellings. Unblocked libc.so (`__GI___argz_next`,
   `__option_is_end/_short`).
5. **XMM-homed integer sweep** (x86): integer cmp/ALU/BitTest/select paths no
   longer request GPR sub-register names for values the RA parked in XMM
   (bit-punned float words, GET_FLOAT_WORD). Fixed sites:
   `emit_int_cmp_insn_typed`, alu dispatch + andn dest + mem-dest rhs +
   int-madd dest + bit-test base, `emit_alu_reg_direct` rhs, fused bit-test
   branch base/index, 4× cmp/select `dest_reg` in comparison.rs. Eight glibc
   math TUs (s_tanf, e_logf, e_powf, s_nextupf, ...) ICEd on these.
6. **Version-script exports for .symver symbols** (`emit_shared.rs`): the
   `local: *` filter matched the composed "fdopen@@GLIBC_2.2.5" name against
   the script's `fdopen` pattern — every explicitly-versioned export was
   silently dropped (libc.so: 2328 → 3185 dynsyms after the fix; GNU ld
   confirmed fdopen was truly absent).
7. **versym order desync** (`emit_shared.rs`): the versym table was emitted
   in pre-reorder symbol order; after the undef-first + .gnu.hash bucket
   sort, version indices landed on the wrong symbols. Rebuilt from the final
   dynsym order; single-`@` names now get the hidden (non-default) bit.

## glibc 2.44 status after this session

| Stage | Status |
|---|---|
| configure (`CC=lccc`, `LD=lccc-ld`) | ✅ |
| full compile phase (every TU incl. malloc, elf, nptl, math) | ✅ |
| ld.so (355 KB dynamic loader) | ✅ links, `-z defs` clean |
| libc.so (2.97 MB) | ✅ links; 3185 versioned dynsyms |
| rtld-libc.a / librtld.map machinery | ✅ |
| sotruss-lib.so, libc_malloc_debug prerequisites | ✅ |
| **glibc's own binaries run under our ld.so+libc.so** | ✅ utmpdump, sprof |
| libm.so | ❌ `__builtin_fma{,f}` + `__builtin_ia32_cvtpd2ps` unlowered (ms178 patch's math-use-builtins-fma) |
| ldconfig | ❌ `feof_unlocked` undefined (extern-inline/export gap) |
| externally-compiled PIE vs lccc-glibc | ❌ startup SIGSEGV (crt/TLS init triage needed) |
| `make check` | blocked on the three items above |

## Next-session priorities (in order)

1. Lower `__builtin_fma`/`__builtin_fmaf` to vfmadd (x86-64-v3 baseline) and
   `__builtin_ia32_cvtpd2ps` → unblocks libm.so.
2. `feof_unlocked` for ldconfig (likely same hidden_proto/export family).
3. External-PIE startup crash triage under lccc-glibc (LD_DEBUG walk).
4. `make check` harness (BACKLOG MS-11) once libm.so exists.
5. IFUNC end-to-end (LK-19) → --enable-multi-arch parity with the PKGBUILD.
