# Cross-architecture TLS code-generation evidence

This note records the reproducible checks for the AArch64 and RISC-V TLS
lowering added in this change. Run commands from the repository root.

## Toolchain oracles

- Clang 23.1.3, commit `0d261d1ca552c95a8f007e061c787ac7132fbcbc`
- Compiler Explorer AArch64 GCC 16.1.0 (`carm64g1610`)
- Compiler Explorer RISC-V GCC 16.1.0 (`rv64-cgcc1610`)
- GNU cross linkers and QEMU user emulation from Debian

Compiler Explorer did not expose GCC 16.2 AArch64 or RISC-V cross targets when
this work was validated. Intel ICX 2026.2.0 targets x86-64 in this installation;
its driver rejects AArch64/RISC-V code generation, so it is not presented as a
cross-architecture oracle.

Generate exact Clang reference assembly with:

```sh
CLANG=/home/user/toolchains/llvm2313/bin/clang
for target in aarch64-linux-gnu riscv64-linux-gnu; do
  "$CLANG" --target="$target" -O2 -fPIC -S \
    tests/regression/tls_cross_models/consumer.c \
    -o "/tmp/clang2313-${target}-tls.s"
done
```

Generate the available GCC 16 cross references with:

```sh
python3 scripts/godbolt.py compile carm64g1610 \
  tests/regression/tls_cross_models/consumer.c --flags='-O2 -fPIC'
python3 scripts/godbolt.py compile rv64-cgcc1610 \
  tests/regression/tls_cross_models/consumer.c --flags='-O2 -fPIC'
```

The minimal RISC-V dynamic address function now has the same eight expanded
instructions as GCC 16.1 and Clang 23.1.3: a 16-byte RA-only frame, paired GD
address materialization, resolver call, and frame teardown. AArch64 dynamic
lowering uses the standard four TLSDESC relocations and Clang 23.1.3's exact
instruction shape, reading TP after `blr` instead of before it: only x0's
contract is normative, so the hoisted form would depend on every resolver
preserving x1 in exchange for at most the ~1 cycle latency of a
dependency-free `mrs` that an out-of-order core issues early anyway.

## Automated checks

```sh
cargo build --profile fastbuild -j2
CCC=target/fastbuild/lccc bash tests/regression/check_tls_cross_models.sh
```

The gate checks exact relocation counts and assembly properties for LE, IE,
GD/LD, rejects static TLS relocations in shared objects, verifies that the
hidden RISC-V resolver call saves `ra`, and rejects an unnecessary frame
pointer in the one-resolver address path. It also sweeps every generated
function for two register-discipline invariants that need neither a linker nor
an emulator, so they hold in CI wherever the runtime half has to skip: any
function that reads the frame pointer must establish it (`s0` on RISC-V, `x29`
on AArch64), and a function combining dynamic TLS with a hidden wide-type
libcall must park the resolver's result in a callee-saved register instead of
leaving it in `a0` for `__divti3` to destroy. When cross linkers and QEMU are
installed, it also links and executes AArch64 and RISC-V IE, dynamic and
wide-type producer/consumer combinations.
