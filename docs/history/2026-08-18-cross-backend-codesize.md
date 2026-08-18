# 2026-08-18 (session 2) — Cross-backend code-bloat parity

**Base:** `origin/main` @ `89f1404` (post-PR #112; both prior batches merged)
**Branch:** `arena/codesize-x64` — snapshot `/home/user/ms178-1.patch`
**Mandate:** every code-bloat fix must land in BOTH i686 and x86-64 where technically feasible. Linker work delegated — untouched.

## Parity matrix after this session

| Improvement | i686 | x86-64 | Notes |
|---|---|---|---|
| direct-to-dest ALU (no accumulator round-trip) | ✅ (prev session) | ✅ (pre-existing) | x86-64 was the original; i686 was the port |
| memory-destination / slot-RMW ALU | ✅ (prev session) | ✅ (pre-existing `mem_dest_mnem`) | |
| GlobalAddr+Load/Store fold | ✅ absolute (non-PIC) | ✅ sym(%rip) (pre-existing) | |
| never-materialized fold preview (regalloc + slots) | ✅ (prev session) | ✅ **NEW this session** | x86-64 adds GOT-symbol refinement; byte-neutral on corpus (registers plentiful), contract parity + pressure wins |
| never-read-store DSE: frame-coordinate soundness | ✅ (prev session sp_down walk) | ✅ epilogue-chain relaxation **NEW** | x86-64's blanket RSP-shift bail disabled the pass for every FPO function |
| never-read-store DSE: escape discipline | ✅ **NEW this session** | ✅ **NEW this session** | indirect derefs don't bail unless a frame address escaped (leal-of-slot / raw sp value copy). i686 additionally gained frame-mode %ebp discipline fixing a LATENT unsoundness (data-%ebp stores could be deleted) |
| post-inc spill forwarding (store; modify src; reload → copy; modify) | n/a (i686 keeps slots; covered by other passes) | ✅ **NEW** | kills both per-iteration stack round-trips in `while(n--) s += *p++;` — loop body now matches GCC shape |
| register-base GEP fold → offset(%reg) addressing | ✅ **NEW this session** | ✅ (pre-existing) | i686 emitted lea-materialize+deref per access; now `movl 40(%ebx),%eax` |
| tail-call conversion | ✅ **NEW this session** | ✅ (pre-existing) | i686 rules: escape suppression, %eax/%edx protection, plain-ret only, restored-reg indirect rejection |
| zero-idiom + copy-test + imm-move fusion peepholes | ✅ (prev session) | ✅ (pre-existing equivalents: rax_is_zero tracking, compare_branch, fold_zero_extended_xor_moves) | |
| -mpreferred-stack-boundary honoring | ✅ (prev session) | n/a (SysV mandates 16) | |
| regparm=3 ABI | ✅ (prev session, complete) | n/a (SysV register args native) | |

## Measurements (validated at every step: 806/806 unit, 50/50 correctness, regparm/cdecl ABI green at -O0..-Os)

- **realmode corpus (kernel arch/x86/boot, REALMODE_CFLAGS)**: 37784 → **37578** (GEP fold −206; escape-DSE and tail-call correctly suppressed there: bioscall locals' addresses escape). GCC 12999 → ratio 2.89.
- **x86-64 benchmark corpus (`tests/benchmark/programs`, -O2 text)**: 25704 → 25711 (**+7**, the post-inc forwarding trades a store for a reg move; GCC = 25813, ratio .996).
- Canonical `*p++` loop: two per-iteration stack round-trips eliminated, body now `movq %r11,%rdi; leaq 1(%r11),%r11; movzbl (%rdi),…` = GCC shape.
- hash_table timing spread on the shared VM is ±9% on IDENTICAL binaries — never trust single wall-clock runs here; diff the asm.

## New regression/unit tests

- `tests/regression/i686_gep_fold_reg_base.c` (runtime, all modes)
- peephole unit: `test_tail_call_direct_i686`, `test_tail_call_suppressed_by_leal_slot`, `test_tail_call_indirect_not_through_restored_reg`, `test_ebp_pointer_store_kept_without_frame` (pins the latent-unsoundness fix), reworked `test_redundant_store_load` (now runs under a real FP prologue)

## Next levers (ranked)

1. **i686 %edx staging in leaf returns** (`movl counter,%edx; movl %edx,%eax; ret` — the Load lands in a phase-2 register then copies to the return acc; teaching emit_return to read the register directly, or biasing single-use load results toward %eax, saves 2 bytes/leaf).
2. **zlib_ng_adler32 x86-64 gap (1486 vs 683)**: the NMAX modulo block still materializes `movabsq $2147975281` twice and round-trips slots in the len_16 helper; needs GVN of the magic constant + the never-materialized preview extended to repeated `Copy` chains.
3. **strlen2-shape**: `movsbq (%r11),%r10; movsbq %r10b,%r8; testq` — double sign-extension of a fresh load; a redundant-ext peephole case (x86-64 `redundant_ext.rs` misses the reg-to-reg movsbq after a movsbq load).
4. **i686 callee-saved epilogue with FP**: `leal -N(%ebp),%esp` blocks the addl-form teardown recognition in some shapes — audit interplay with frame_compact-equivalent for i686 (none exists yet; x86-64 has `frame_compact.rs` + `callee_saves.rs`, i686's `eliminate_unused_callee_saves` is still disabled with a TODO).
5. **Kernel bzImage gate**: setup total was 45210 vs 32768 limit at last full build (before this session's fixes; re-measure — expect ~44900). The remaining 12K needs levers 1+4 plus the accumulator-bypass extension to Load/Cmp/Cast.

## Environment protocol (unchanged, harness wipes everything)

swap 10G first; rustup stable minimal; `cargo build --profile fastbuild --locked -j2`; apt bison/flex/bc/libelf-dev/cpio/kmod/qemu-system-x86/busybox-static/gcc-multilib/libc6-dev-i386; kernel = 6.18.44 + 26 patches from ms178/archpkgbuilds (PKGBUILD order; 1020 now carries the PAGE_POOL Kconfig fix upstream); `make defconfig && make prepare` with HOSTCC=gcc; REALMODE flags recipe in `/tmp/rmflags.sh` pattern (see 2026-08-18-kernel-codesize-session.md).
