# Follow-up 2026-10-10: `__seg_gs` declaration qualifier (srcu boot blocker)

Status: fix landed in the working tree; `--fast` gate run recorded in the
snapshot ledger. The kernel build is NOT done yet (see To-do).

## Scope and policy decisions this session

- **Gate policy (owner decision):** a local `ci_local.sh --fast` pass is the
  snapshot gate. The full run is not required locally; GitHub CI runs the slow
  gates. `scripts/lccc-snapshot.sh` now accepts `mode=fast` and `mode=full`
  stamps for the same tree, recorded as `ci_local-<mode>-PASS@<os>` in the
  ledger. `scripts/ci_local.sh` comment updated to match.
- Rejected by the owner: the full CI run (`ci_local.sh` without `--fast`).

## Done

1. **Root cause, parameters.** `parse_param_list_inner` never read the
   declaration's named address space. `long f(struct c __seg_gs *p)` emitted
   `movq (%rdi),%rax`. HEAD confirmed (binary built from the pre-fix tree).
2. **Root cause, declarations.** `analyze_declaration` (sema) built
   `full_type` and never applied `decl.address_space`. Locals, globals and
   typedefs were typed generic, so a correct `u(p)` call was rejected as
   "incompatible pointer type", or the `%gs` access was lost when the pointer
   came from a call.
3. **Root cause, return types.** `apply_declaration_address_space` had no
   `CType::Function` arm, and `parse_function_def` built its return type with
   `build_return_type` without the qualifier. Prototypes and definitions of
   `struct c __seg_gs *f(void)` therefore returned a generic pointer. This is
   the `srcu_read_lock_fast` shape.
4. **Stale-slot leak (found during this work).** The first fix read
   `parsing_address_space` in the parameter loop. The declaration's own slot was
   never cleared, so `struct c __seg_gs *lk(struct s *ssp)` gave `ssp` the
   `%gs` qualifier. Fixed by:
   - `ParsedDeclAttrs::decl_address_space`: the external-declaration path moves
     the qualifier into this field right after the type specifier, before any
     declarator or parameter list is parsed (`parse_external_decl`). Bare-type
     and `parse_declaration_rest` read this field.
   - The parameter loop clears `parsing_address_space` before each parameter's
     type specifier.
   - `apply_decl_address_space_to_spec` (declarations.rs) applies the qualifier
     to a function definition's return type.
5. **Regression gate:** `tests/regression/check_seg_named_decl_qualifier.sh`,
   wired into `ci_local.sh` as `seg-named-decl-qualifier` and into the hosted
   workflow (`.github/workflows/ci.yml`, step "Verify declaration-level __seg_gs
   qualifier codegen"); `ci-gate-parity` requires both. It checks the
   emitted assembly for `%gs:` on params, locals, prototype and definition
   returns, and typedefs, plus a negative control on a plain pointer. It fails
   on the HEAD binary (verified) and passes on the fix.

### Verified results (lccc vs GCC 14.2 on this host)

| case | HEAD | fix |
|---|---|---|
| param `p->v[0]` | `movq (%rdi)` (no %gs, miscompile) | `movq %gs:(%rdi)` = GCC |
| prototype return `lk()->v[0]` | `movq (%rax)` (miscompile) | `movq %gs:(%rax)` = GCC |
| definition return `mk()->v[1]` (inlined, base 0) | `movq 8(%rdi)` (miscompile) | `movq %gs:8` = correct for base 0 |
| local `struct c __seg_gs *p = lk()` | `%gs:` (already right) | `%gs:` |
| `u(lk())`, `u(p)`, typedef param | accepted, codegen wrong | accepted, codegen right |
| `lk2(struct s *)` passed plain `struct s *` | (correct) | accepted (GCC accepts) |

Unit tests: `cargo test --profile fastbuild` = 4225 passed, 0 failed.
Existing seg gates (`check_segfs_typeof_nosteal.sh`,
`check_seg_prefix_rmw_encoding.sh`) pass.

## Environment incidents this session (for future agents)

- The workspace was restored from a snapshot mid-session. `.git` was gone, so
  it was rebuilt by `git clone` + `git reset --mixed a941096…` with
  `.git` moved in. 395 files lost their exec bit in the restore and were
  restored with `chmod +x` based on `git diff --summary`. Check
  `git diff --summary` after any restore before trusting `git status`.
- `rustup` shims were missing; `~/.cargo/bin/{cargo,rustc,...}` now symlink to
  the stable toolchain (1.99.0).
- The host had no swap, no clang/mold/lld/qemu after restore. Reinstalled
  with `ensure_swap.sh` (8G) and apt.
- The `-m32` gates need `libc6-dev-i386` and `gcc-multilib`. The linker suite's
  i386 C++ link needs `g++-multilib`. Without these, three gates
  (`ivsr-integer-domains`, `ivwiden-exposed-latch`, `fp-extract-homes`) and
  `linker-suite` fail for host reasons, not compiler reasons. Consider adding
  these to `ensure_cross_toolchains.sh`.
- `pkill -f <pattern>` matches the calling shell's own command line. It killed
  the tool shell. Use `pgrep -x` or pid files.

## Codegen findings (missed optimizations, not fixed)

- **Segment addressing modes are not folded.** For a `%gs`-qualified pointer,
  lccc emits `leaq 8(%rax),%rcx; movq %gs:(%rcx),%rax` where GCC emits
  `movq %gs:8(%rax),%rax`. Likewise `shlq $3,%rsi; leaq (%rdi,%rsi),%rcx;
  movq %gs:(%rcx)` where GCC emits `movq %gs:(%rdi,%rsi,8)`. The generic path
  folds both (`movq 8(%rax)`, `movq (%rdi,%rsi,8)`). The kernel per-CPU
  pattern makes this a performance item, not only a size one.
  Owner: segment lowering in the address-mode selector. Measure before fixing.
- `kernel` per-CPU accesses should be checked for the same loss once the
  kernel builds.

## Known open items in the front end

- `n2.c` (`__seg_gs`-to-generic member assignment): lccc accepts, GCC rejects.
  Left as is; the diagnostic should be added with its own regression.
- `struct c __seg_gs *(*fp)(void)`: `apply_declaration_address_space` still
  qualifies the function-pointer pointer, not the return pointer. Not used by
  the kernel. Documented, not changed.
- `seg1.c` and `seg2.c` in `/home/user/work/lccc-repro` are invalid as GCC
  references (they redefine `__seg_gs` as an attribute macro). Use `q*.c`,
  `r*.c`, `cg*.c` instead.

## To-do (kernel path, not started in this session's final state)

1. Re-run `scripts/prepare_kernel_tree.sh` with the fixed lccc. The kernel
   tree at `/home/user/target/kernel-work` was lost in the restore and must be
   re-extracted (tarball sha256 f410638061a165c12f42ab871d2f3fcd525515359b5faeee80969cff84524df9).
2. Fetch and verify kernel.org `sha256sums.asc` for 6.18.55 (not yet done).
3. Update the 6.18.52 defaults in `build_kernel_boot.sh`, `prepare_kernel_tree.sh`
   and the other scripts listed in the workspace notes. Leave historical comments.
4. Host gcc to lccc for `mkcpustr`, `HOSTCC`, `mkpiggy`. The no-GCC rule for
   kernel TUs still holds; `prepare` currently uses `HOSTCC=gcc` for host tools.
5. Build boot code under the 32K gate (`build_kernel_boot.sh`), then the full
   kernel (`build_kernel_full.sh`), then `qemu_boot_test.sh`.
6. Config decision: `build_kernel_full.sh` disables STACKPROTECTOR, OBJTOOL,
   ORC, FUNCTION_TRACER, DYNAMIC_FTRACE and DEBUG_ENTRY; the PKGBUILD config
   keeps ftrace. Decide and document. Recommended: use the PKGBUILD config as
   the "default" and document each deviation.
7. Version gaps to record: Debian clang 19.1 vs reference 23.1; mold 2.37.1
   vs 2.42.1; qemu 10.0.13. Look up the mold X86/i686 CMake option in the mold
   source before using the build preset.
8. Benchmark corpus extraction from archpkgbuilds (gzip, zlib-ng, expat, SQLite,
   glibc, Linux) and the godbolt oracle comparison (GCC 16.2, Clang 23.1, ICX,
   lld 23.1, bfd 2.47, mold 2.42.1) are not started.
9. Refresh `/home/user/ms178-1.patch` and the snapshot after each validated
   step (`scripts/lccc-snapshot.sh "<slug>" "<desc>"`).
