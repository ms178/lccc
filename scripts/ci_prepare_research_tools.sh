#!/usr/bin/env bash
# Private, current-toolchain research bootstrap. Invoked through ci_test_report.
set -euo pipefail
# The -m32 compiler needs the i386 UAPI asm/ headers, not only libc stubs.
sudo dpkg --add-architecture i386
sudo apt-get update
sudo apt-get install -y --no-install-recommends python3-yaml autoconf automake libc6-dev-i386 linux-libc-dev:i386 libc6-dbg
# Bootstrap with a current binary linker and Clang's integrated assembler.
git fetch --depth=1 origin tag harness-cache-1
mkdir -p "$RUNNER_TEMP/current-linker"
git show harness-cache-1:mold-2.42.1-x86_64.part00 > "$RUNNER_TEMP/current-linker/ld"
echo "86b918f8af9444aed086ea73b593b88e910d49829d2cae979cbb84509b64dc76  $RUNNER_TEMP/current-linker/ld" | sha256sum -c -
chmod +x "$RUNNER_TEMP/current-linker/ld"
echo "$RUNNER_TEMP/current-linker" >> "$GITHUB_PATH"
export PATH="$RUNNER_TEMP/current-linker:$PATH"
clang_driver=$(bash scripts/ci_install_clang_oracle.sh --print-bin) || {
    printf "%s\n" "$clang_driver" >&2
    exit 1
}
export CC="$clang_driver"
export CXX="$clang_driver --driver-mode=g++"
if ! bash scripts/ensure_gas_247.sh x86_64-linux-gnu; then
    for log in "$HOME/.cache/binutils-2.47-build-x86_64_linux_gnu/"{configure.log,make.log,config.log}; do
        if [[ -f $log ]]; then
            printf '\n--- %s ---\n' "$log" >&2
            tail -n 60 "$log" >&2
        fi
    done
    exit 1
fi
unset CC CXX
echo "$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin" >> "$GITHUB_PATH"
export PATH="$HOME/.cache/gas-2.47-x86_64-linux-gnu/bin:$PATH"
# Benchmark the exact authenticated cache oracle, never rolling 16.0.1.
gcc_driver=$(bash scripts/ci_install_gcc_oracle.sh --version 16.2 --print-bin) || {
    printf "%s\n" "$gcc_driver" >&2
    exit 1
}
export PATH="$(dirname "$gcc_driver"):$PATH"
test "$(gcc-16 -dumpfullversion)" = 16.2.0
gcc-16 --version
valgrind_bin=$(CC=gcc-16 CXX=g++-16 bash scripts/ci_install_valgrind_current.sh --print-bin)
export PATH="$valgrind_bin:$PATH"
valgrind --version
rustc -Vv
cargo --version
rustfmt --version
cargo clippy --version
