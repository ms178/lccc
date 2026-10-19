# Follow-up: S05 rebased onto origin/main d8f79a6b (PR #868)

Host: Debian 13 VM (rebuilt after a workspace reset), 2 vCPU, ~2 GiB RAM, 8 GiB swap, 25 GB disk.
No PMU. Toolchains: GCC 16.2.0-3 and binutils 2.47-6 (Debian Sid), Clang 23.1.3
(`harness-cache-4`), ICX 2026.2.0.20261001 (Intel oneAPI APT, SHA-256 verified), Rust 1.97.0
(`rust-cache-1.97.0`, SHA-256 verified), mold 2.42.1 (`harness-cache-1`), QEMU 8.2.2
(`harness-cache-3`). Every cache artifact was checked against its tag's `MANIFEST.sha256`.

## Done and validated (tree `cf701203`, then `e517296c`)

| Item | Result |
|---|---|
| Rebase of the S05 delta onto `d8f79a6b` | `git apply --check` and `git apply` rc 0, no conflicts. |
| Fastbuild (`cargo build --profile fastbuild`, `-j2`, swap on) | 0 warnings. |
| `ci_local.sh --fast` | `SUMMARY: 338 passed, 0 failed, 6 skipped`, `ALL GATES GREEN`, 0 warnings. The 6 skips are the documented `--fast` exclusions. |
| `cargo test` pass_policy / extern_admission filters | 11/11 pass. |
| Punning fuzz vs LCCC (seeds 0:400, O0 and O2) | 800/800 pass. |
| 4-compiler runtime check of the libcall shape (see below) | LCCC matches the C semantics; GCC, Clang and ICX do not. |

### Changes in this revision, with the reason

1. **`Libcall` table from one macro** (`src/common/pass_policy.rs`). The enum, `ALL` and `name()` are generated from a single list. Before, `ALL` was a hand-written array: a variant missing from it stayed inlinable, and the exhaustive admission test silently skipped it. The array length is now derived from the same list, and a `const` assert keeps the table within the `u32` mask. Tests added: names are unique, each variant owns one mask bit, and `from_name` is exact (no prefix, case or `__builtin_` aliasing).
2. **Linker suite `sh()` fails loudly on a missing tool** (`tests/linker/run_linker_tests.py`). It raises a named `RuntimeError`. A silent return code was rejected: many setup steps ignore their return code, and negative cases expect a link to fail, so a missing toolchain would pass as "expected failure".
3. **rustfmt** on the new test code (the `rustfmt` gate caught it).
4. **Stale comments** in `run_regression.py`, `test_gcc_reference_link_mode.py` and `check_libcall_synthesis_no_selfcall.sh`. They now state the measured facts: default-PIE `-m32` runs with binutils 2.47, and the `puts` self-call comparison below.

### Failures that were not compiler bugs

- The first `ci_local --fast` attempt on this tree reported 15 `elf_writer_common` test failures and a `rustfmt` failure. The `cargo test` failure was `No space left on device` (the disk was at 100%). After removing duplicate `.part` files, old binutils build trees and stale `target/debug` and `target/redteam`, the same tree passed.
- The `rustfmt` failure was my own test code. Fixed with `rustfmt --edition 2024` on that file only.

## Benchmarks (`-O2`, same checksum from all four compilers)

Kernels: table CRC-32 over 1 MiB, popcount by clearing the lowest bit over 64 K u64, int32 dot product, `strlen` of 4 KiB. Each iteration mutates its inputs, so pure calls cannot be hoisted. Minimum of 7 interleaved runs. This is a 2-vCPU VM with no PMU: treat these as indicative only.

| Kernel | LCCC | GCC 16.2 | Clang 23.1.3 | ICX 2026.2 |
|---|---|---|---|---|
| CRC-32 (200 × 1 MiB) | **376 ms** | 428 ms | 444 ms | 483 ms |
| popcount loop (200 × 64 K) | 256 ms | 233 ms | **218 ms** | **218 ms** |
| dot / strlen | ~6 ms / ~2 ms | ~6 / ~2 | ~4 / ~2 | ~5 / ~2 |
| whole program | 654 ms | 637 ms | 632 ms | 777 ms |

Correction: an earlier revision of this file reported LCCC as 2.7× slower, then different
whole-program numbers. Both came from benchmark artifacts (hoisted pure calls, and a
mutation that was applied to only one of the two source files). They are retracted.

**Open missed optimization (measured):** LCCC keeps the per-set-bit loop of
`while (x) { x &= x - 1; n++; }`. The loop is `lea`, `and`, `test`, `jne` per set bit, a serial
dependency chain. Clang, ICX and GCC emit a fixed-cost population count for the element, and
that is the 14–17% popcount gap. LCCC lowers explicit `__builtin_popcount` already, so the
missing piece is loop-idiom recognition (`x &= x-1` counting → `ctpop`, then `popcnt` under
`-mpopcnt`, or the SWAR sequence otherwise). This was not implemented in this revision.

## Runtime semantics: TU-defined `puts` that calls `printf` (verified)

Program: a TU defines `int puts(const char *s) { printf("hello\n"); return s ? 0 : -1; }`
and `main` calls `puts("x")`, `-O2`, run with a 3-second timeout.

| Compiler | Result | Mechanism (disassembly) |
|---|---|---|
| LCCC (this build) | prints `hello`, returns 0 (correct) | calls `printf` |
| GCC 16.2 | SIGSEGV | `printf` → `puts` fold, self-recursive `call puts@PLT` |
| Clang 23.1.3 | timeout (infinite loop) | body becomes `jmp puts` (self-jump) |
| ICX 2026.2 | timeout (infinite loop) | body becomes `jmp puts` (self-jump) |

The libcall gate's runtime section pins the LCCC behaviour. This is the reason
`check_libcall_synthesis_no_selfcall.sh` exists, and it is why a fold that creates a library
call must not target the TU's own definition.

## Open items (not done in this revision)

1. Loop-idiom recognition for population count (see above). Expected to close the popcount gap, not measured yet.
2. Benchmark on the real corpus (zlib-ng, gzip, expat from `ms178/archpkgbuilds`). This revision uses synthetic kernels only.
3. Full 5000-case punning matrix and the mutation test on this base. This revision ran 800 cases.
4. Measured cost of the libcall carve-out (`extern_inline_admission`) on a real workload. Only codegen evidence exists.
5. Re-run the `-m32` link audit by executing every line in `/tmp/m32_links.txt`. This revision tested representative link shapes only (PIE and `-no-pie`, `-lm`, object link, SSE).
6. Clippy with `-D warnings` on the whole crate is part of the gate; it passed here, but the gate's log should be checked on each rebase.
