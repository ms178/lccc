#!/usr/bin/env bash
# AArch64/RISC-V TLS model, relocation and runtime contract.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
CCC=${CCC:-$ROOT/target/fastbuild/lccc}
CCC=$(readlink -f "$CCC")
FIX=$ROOT/tests/regression/tls_cross_models
TD=$(mktemp -d)
trap 'rm -rf "$TD"' EXIT HUP INT TERM
fail=0
bad() { echo "FAIL: $*" >&2; fail=1; }
note() { echo "  $*"; }

ln -sf "$CCC" "$TD/aarch64-linux-gnu-lccc"
ln -sf "$CCC" "$TD/riscv64-linux-gnu-lccc"

check_reloc_count() {
    file=$1 pattern=$2 expected=$3 what=$4
    count=$(readelf -rW "$file" | grep -Ec "$pattern" || true)
    [ "$count" -eq "$expected" ] || bad "$what: expected $expected relocations, got $count"
}

for arch in aarch64 riscv64; do
    cc="$TD/$arch-linux-gnu-lccc"
    "$cc" -O2 -fno-pic -S "$FIX/consumer.c" -o "$TD/$arch-ie.s"
    "$cc" -O2 -fno-pic -c "$FIX/consumer.c" -o "$TD/$arch-ie.o"
    "$cc" -O2 -fPIC -shared -S "$FIX/consumer.c" -o "$TD/$arch-dyn.s"
    "$cc" -O2 -fPIC -shared -c "$FIX/consumer.c" -o "$TD/$arch-dyn.o"

done

# Static executable defaults: preemptible external is IE, definition is LE.
check_reloc_count "$TD/aarch64-ie.o" 'R_AARCH64_TLSIE_ADR_GOTTPREL_PAGE21.*external_tls' 3 'AArch64 IE page'
check_reloc_count "$TD/aarch64-ie.o" 'R_AARCH64_TLSIE_LD64_GOTTPREL_LO12_NC.*external_tls' 3 'AArch64 IE load'
check_reloc_count "$TD/aarch64-ie.o" 'R_AARCH64_TLSLE_ADD_TPREL_HI12.*local_tls' 1 'AArch64 LE high'
check_reloc_count "$TD/aarch64-ie.o" 'R_AARCH64_TLSLE_ADD_TPREL_LO12_NC.*local_tls' 1 'AArch64 LE low'
check_reloc_count "$TD/riscv64-ie.o" 'R_RISCV_TLS_GOT_HI20.*external_tls' 3 'RISC-V IE GOT'
check_reloc_count "$TD/riscv64-ie.o" 'R_RISCV_TPREL_HI20.*local_tls' 1 'RISC-V LE high'
check_reloc_count "$TD/riscv64-ie.o" 'R_RISCV_TPREL_ADD.*local_tls' 1 'RISC-V LE add marker'
check_reloc_count "$TD/riscv64-ie.o" 'R_RISCV_TPREL_LO12_I.*local_tls' 1 'RISC-V LE low'

# Shared objects must use loader-safe dynamic models, never static TPREL.
for kind in ADR_PAGE21 LD64_LO12 ADD_LO12 CALL; do
    check_reloc_count "$TD/aarch64-dyn.o" "R_AARCH64_TLSDESC_${kind}.*external_tls" 3 "AArch64 TLSDESC $kind external"
    check_reloc_count "$TD/aarch64-dyn.o" "R_AARCH64_TLSDESC_${kind}.*local_tls" 1 "AArch64 TLSDESC $kind local"
done
if readelf -rW "$TD/aarch64-dyn.o" | grep -q 'TLSLE_'; then
    bad 'AArch64 shared object retained a static TLSLE relocation'
fi
check_reloc_count "$TD/riscv64-dyn.o" 'R_RISCV_TLS_GD_HI20.*external_tls' 3 'RISC-V GD external'
check_reloc_count "$TD/riscv64-dyn.o" 'R_RISCV_TLS_GD_HI20.*local_tls' 1 'RISC-V GD local'
check_reloc_count "$TD/riscv64-dyn.o" 'R_RISCV_CALL_PLT.*__tls_get_addr' 4 'RISC-V GD resolver calls'
if readelf -rW "$TD/riscv64-dyn.o" | grep -q 'R_RISCV_TPREL_'; then
    bad 'RISC-V shared object retained a static TPREL relocation'
fi

# Shape controls catch mutations back to hard-coded LE or missing hidden-call
# frame handling. RISC-V IE address return is the three-instruction psABI form.
grep -q 'adrp x0, :gottprel:external_tls' "$TD/aarch64-ie.s" || bad 'AArch64 IE assembly shape missing'
grep -q '\.tlsdesccall external_tls' "$TD/aarch64-dyn.s" || bad 'AArch64 TLSDESC marker missing'
grep -q 'auipc a0, %tls_ie_pcrel_hi(external_tls)' "$TD/riscv64-ie.s" || bad 'RISC-V IE direct return shape missing'
if sed -n '/^external_address:/,/^\.size external_address/p' "$TD/riscv64-ie.s" | grep -q 'mv a0, t0'; then
    bad 'RISC-V IE address return regressed to a t0 relay'
fi
grep -q 'call __tls_get_addr' "$TD/riscv64-dyn.s" || bad 'RISC-V GD resolver call missing'
grep -q 'sd ra,' "$TD/riscv64-dyn.s" || bad 'RISC-V hidden TLS call failed to save ra'
rv_dyn_addr="$TD/riscv64-dyn-address.s"
sed -n '/^external_address:/,/^\.size external_address/p' "$TD/riscv64-dyn.s" >"$rv_dyn_addr"
if grep -Eq '\b(sd|ld) s0,|addi s0,' "$rv_dyn_addr"; then
    bad 'RISC-V one-resolver address path regressed to an unnecessary frame pointer'
fi

# Cross-runtime interoperability against GCC-built TLS definitions and hosts.
for arch in aarch64 riscv64; do
    gcc="$arch-linux-gnu-gcc"
    qemu="qemu-$arch"
    if ! command -v "$gcc" >/dev/null 2>&1 || ! command -v "$qemu" >/dev/null 2>&1; then
        note "SKIP $arch runtime (cross GCC/qemu unavailable)"
        continue
    fi
    sysroot="/usr/$arch-linux-gnu"
    "$gcc" -O2 -fno-pic -c "$FIX/provider.c" -o "$TD/$arch-provider.o"
    "$gcc" -O2 -fno-pic -c "$FIX/main.c" -o "$TD/$arch-main.o"
    "$gcc" -no-pie "$TD/$arch-main.o" "$TD/$arch-ie.o" "$TD/$arch-provider.o" -o "$TD/$arch-ie"
    "$qemu" -L "$sysroot" "$TD/$arch-ie" || bad "$arch IE runtime"

    "$gcc" -O2 -fPIC -c "$FIX/provider.c" -o "$TD/$arch-provider-pic.o"
    "$gcc" -shared "$TD/$arch-dyn.o" "$TD/$arch-provider-pic.o" -o "$TD/lib$arch-tls.so"
    "$gcc" -O2 "$FIX/main.c" -L"$TD" -Wl,-rpath,"$TD" -l"$arch-tls" -o "$TD/$arch-dyn"
    LD_LIBRARY_PATH="$TD" "$qemu" -L "$sysroot" "$TD/$arch-dyn" || bad "$arch dynamic runtime"
    note "$arch IE/dynamic runtime interoperability"
done

[ "$fail" -eq 0 ] || exit 1
echo 'cross TLS models: PASS'
