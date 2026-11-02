# Follow-up: cross-architecture TLS codegen hardening (PR #884)

Host: sandboxed Debian VM, 2 vCPU, ~4 GiB RAM plus a 4 GiB swap file
(`scripts/ci_enable_swap.sh`), 25 GB disk. No PMU. Toolchains: Rust 1.97.0
(`rust-cache-1.97.0`), Clang 23.1.3 (`harness-cache-4`, commit `0d261d1ca552` —
the exact oracle `docs/tls-cross-evidence.md` cites), QEMU 8.2.2 user-mode
(`harness-cache-2`), mold 2.42.1 (`harness-cache-1`). Every cache artifact was
extracted only after `sha256sum -c` against its tag's `MANIFEST.sha256`.

No cross GCC and no cross sysroot were available: `harness-cache-1`'s
`gcc-16-bookworm-x86_64` is host-only x86_64, `binutils-2.47` is a source
tarball, and `debs-bookworm` carries x86 multilib packages only.
`scripts/ensure_cross_toolchains.sh` needs `apt-get download`, and
`scripts/ensure_gas_247.sh` fetches from `ftp.gnu.org`; neither host is reachable
from this sandbox. Cross execution was therefore done with zig's bundled clang
plus musl (`-target <arch>-linux-musl -static`) linking lccc's own objects, then
running them under `qemu-<arch>`. That path executes real cross binaries without
a sysroot, and it is what produced the miscompile evidence below.

## Done and validated

| Item | Result |
|---|---|
| Rebase onto `origin/main` `455c834` | Clean; `scripts/ci_local.sh` was the only overlapping file and merged without conflict. |
| `cargo build --profile fastbuild --locked -j2` | Exit 0, **0 warnings** (`grep -ci warning` on the log). |
| `cargo clippy --profile fastbuild --locked --all-targets -- -D warnings` | Exit 0. |
| `cargo fmt --all -- --check` | Exit 0. |
| `cargo test --profile fastbuild --locked` | 5440 tests, 0 failed (5430 pre-existing lib tests plus the 2 new relocation tests). |
| `scripts/check_ci_gate_parity.py` | **PASS** — was FAILED, because the new gate existed only in `ci_local.sh`. |
| `scripts/check_ci_workflow_shell.py` | ok (7 workflow files). |
| `tests/regression/check_tls_cross_models.sh` | PASS. Runtime half SKIPs locally (no cross GCC); it now runs in CI, which installs `gcc-aarch64-linux-gnu` and `gcc-riscv64-linux-gnu` next to the `qemu-user` it already had. |
| Corpus asm differential (pre-fix vs post-fix compiler, 226 sources) | 304 asm files on both sides, **299 byte-identical**; the 5 that differ are exactly `consumer.arm`, `fp.arm`, `fp.riscv`, `wide.arm`, `wide.riscv` — the intended TLSDESC register change and the two frame fixes. This is the empirical proof that folding `riscv_is_call_free` into the shared allowlist changed no other codegen. |

### The miscompile that motivated the register-discipline work

`__thread unsigned long ext_tls;` plus `probe()` taking `&ext_tls` and then
dividing two `__int128` globals. On RISC-V the i128 division lowers to a hidden
`call __divti3`, which clobbers `a0`–`a7`. The resolver's result had been homed
in `a0` on the strength of an IR-visible-call scan alone, so the wide call
destroyed the TLS address and `probe` returned the quotient twice.

Executed under `qemu-riscv64` after a zig static-musl link of lccc's object:

| arch | lccc before | lccc after | clang 23.1.3 control |
|---|---|---|---|
| riscv64 | **exit 1 — miscompile** | exit 0 | exit 0 |
| aarch64 | exit 0 | exit 0 | exit 0 |

After the fix the address is parked in a callee-saved register
(`call __tls_get_addr` / `mv s1, a0` … `call __divti3` … `mv t1, s1`) and the
function gets a real `s0` frame.

AArch64 was checked for the same hazard and is sound by construction: with 16
values live across the TLSDESC `blr` the allocator spills to the stack
(`str x0, [sp, #24]`) rather than homing call-spanning values in `x4`–`x8` or
`x13`/`x14`, so nothing is live across the hidden call.

Also verified against the oracle rather than assumed: clang 23.1.3 and lccc emit
the *same* four TLSDESC relocation kinds, 4 each, for `consumer.c`. aaelf64
requires `R_AARCH64_TLSDESC_LDR`/`_ADD` (567/568) only for an LDR/ADD "that has
no other relocation", and the value-carrying 563/564 already mark both, so their
absence in both producers is correct.

## Deferred, with reasons

1. **Allocator-native TLS homing.** The `a0` homing still mutates
   `reg_assignments` after the fact. The idiomatic home for it is
   `RegAllocConfig::reg_hints`, which would let the allocator place the value
   with full liveness knowledge instead of being corrected afterwards. Deferred
   because it is a register-allocation change whose blast radius reaches every
   RISC-V function, not a TLS change; the post-hoc form is now guarded by a
   `debug_assert!` that `a0` is unassigned, and that invariant is provable —
   `a0` is absent from `available_regs` (s1–s11) and enters the pool only via
   `caller_saved_regs`, which `riscv_caller_pool_eligible` keeps closed for any
   dynamic-TLS function.
2. **AArch64 IE load folding.** Clang folds the TP offset into the access
   (`mrs x9, TPIDR_EL0` / `ldr x0, [x9, x8]`, 4 instructions); lccc emits
   `add x0, x1, x0` then a separate load (5). Worth one cycle per IE *load*,
   but it requires teaching the load emitter a (base, index) TLS form, so it
   belongs with the addressing-mode work rather than here.
3. **RISC-V `LocalDynamic`.** Still lowered as one GD sequence per symbol
   instead of a true module-level LD entry with per-symbol `%tls_ldo_hi/lo`.
   Correct, and optimal for a single TLS access per function; it costs one extra
   resolver call per additional symbol in the same function.
4. **`.tlsdesccall` in `scripts/directive_legality_matrix.py`.** The matrix has
   no row for it. Adding one requires cross GAS 2.47 in the environment;
   `ensure_gas_247.sh` downloads from `ftp.gnu.org`/`mirrors.kernel.org`, which
   this sandbox cannot reach. The directive is exercised in CI through the
   gate's own assembler and `readelf` path.
5. **Pre-existing, out of scope: a discarded `__builtin_return_address(0)` still
   emits its load.** `void *f(void){ unsigned long *p=&t;
   __builtin_return_address(0); return p; }` emits `ld t0, -8(s0)` whose result
   is dead. The frame gate now guarantees `s0` is established whenever that
   happens, so the load is safe — but a dead load that can fault is still a
   missed DCE in the intrinsics path and should be fixed there, not by a TLS
   change.
