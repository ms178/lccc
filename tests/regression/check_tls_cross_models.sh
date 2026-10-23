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
    # Register-discipline fixtures: dynamic TLS combined with a hidden
    # wide-type libcall (wide.c) and with the frame-pointer builtins (fp.c).
    # Both are the shapes the two frame optimizations must reason about.
    "$cc" -O2 -fPIC -shared -S "$FIX/wide.c" -o "$TD/$arch-wide.s"
    "$cc" -O2 -fPIC -shared -c "$FIX/wide.c" -o "$TD/$arch-wide.o"
    "$cc" -O2 -fPIC -shared -S "$FIX/fp.c" -o "$TD/$arch-fp.s"
    # Scratch-register discipline across the resolver call itself (gd_acc.c).
    "$cc" -O2 -fPIC -shared -S "$FIX/gd_acc.c" -o "$TD/$arch-gdacc.s"
    "$cc" -O2 -fPIC -shared -c "$FIX/gd_acc.c" -o "$TD/$arch-gdacc.o"
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
# Extract one function body. A NEGATIVE assertion over an empty extract passes
# vacuously — a renamed or dropped symbol would silently retire the check — so
# an empty extract is itself a failure.
extract_fn() {
    file=$1 symbol=$2 out=$3
    sed -n "/^$symbol:/,/^\.size $symbol/p" "$file" >"$out"
    [ -s "$out" ] || bad "missing function body for $symbol (the shape checks below would pass vacuously)"
}
rv_dyn_addr="$TD/riscv64-dyn-address.s"
extract_fn "$TD/riscv64-dyn.s" external_address "$rv_dyn_addr"
if grep -Eq '\b(sd|ld) s0,|addi s0,' "$rv_dyn_addr"; then
    bad 'RISC-V one-resolver address path regressed to an unnecessary frame pointer'
fi
rv_dyn_load="$TD/riscv64-dyn-load.s"
extract_fn "$TD/riscv64-dyn.s" external_load "$rv_dyn_load"
if grep -Eq '\b(sd|ld) s[0-9]+,|addi s0,' "$rv_dyn_load"; then
    bad 'RISC-V one-resolver load path regressed to an unnecessary frame pointer or callee-saved spill'
fi
arm_dyn_load="$TD/aarch64-dyn-load.s"
extract_fn "$TD/aarch64-dyn.s" external_load "$arm_dyn_load"
if grep -Eq '\b(str|ldr|stp|ldp) x19' "$arm_dyn_load"; then
    bad 'AArch64 one-resolver load path saved an unused callee-saved register'
fi
arm_dyn_store="$TD/aarch64-dyn-store.s"
extract_fn "$TD/aarch64-dyn.s" external_store "$arm_dyn_store"
if grep -Eq '\bx20\b' "$arm_dyn_store"; then
    bad 'AArch64 one-resolver store path saved an unused second callee-saved register'
fi

# --- Static register discipline: linker-free, so it always runs --------------
# Both frame optimizations are selected from IR alone, so a regression surfaces
# as a register that the emitted frame never establishes. These sweeps read the
# assembly the compiler actually produced; they need no linker and no emulator,
# which is what keeps them live in CI wherever the runtime half has to skip.
# Labels beginning with '.' are assembler locals (.Ltls_gd_0, .Lfunc_end0), not
# function boundaries, so the function pattern starts at a letter or underscore.

# RISC-V: any function that reads s0 must establish it. __builtin_frame_address
# and __builtin_return_address lower to `mv t0, s0` / `ld t0, -8(s0)`, so the
# RA-only frame a lone resolver call permits would leave them reading the
# CALLER's frame pointer -- and the load can fault if the caller has none.
riscv_s0_offenders() {
    awk '
        /^[A-Za-z_][A-Za-z_0-9$.]*:/ {
            if (name != "" && reads && !estab) print name
            name = $0; sub(/:$/, "", name); reads = 0; estab = 0; next
        }
        name == "" { next }
        /^[ \t]*#/ || /^[ \t]*\.cfi/ { next }
        /^[ \t]*addi[ \t]+s0,[ \t]*sp,/ { estab = 1; next }
        /^[ \t]*(sd|ld)[ \t]+s0,[ \t]*-?[0-9]+\(sp\)/ { next }
        /s0/ { reads = 1 }
        END { if (name != "" && reads && !estab) print name }
    ' "$1"
}

# AArch64: the same invariant for x29, which emit_prologue_arm establishes
# unconditionally. Pinned here so a future frameless-leaf optimization cannot
# silently reintroduce the hazard on this side of the port.
aarch64_x29_offenders() {
    awk '
        /^[A-Za-z_][A-Za-z_0-9$.]*:/ {
            if (name != "" && reads && !estab) print name
            name = $0; sub(/:$/, "", name); reads = 0; estab = 0; next
        }
        name == "" { next }
        /^[ \t]*#/ || /^[ \t]*\.cfi/ { next }
        /^[ \t]*mov[ \t]+x29,[ \t]*sp/ { estab = 1; next }
        /^[ \t]*(stp|ldp)[ \t]+x29,/ { next }
        /x29/ { reads = 1 }
        END { if (name != "" && reads && !estab) print name }
    ' "$1"
}

for f in "$TD"/riscv64-*.s; do
    offenders=$(riscv_s0_offenders "$f")
    [ -z "$offenders" ] || bad "RISC-V $(basename "$f") reads s0 without establishing it: $offenders"
done
for f in "$TD"/aarch64-*.s; do
    offenders=$(aarch64_x29_offenders "$f")
    [ -z "$offenders" ] || bad "AArch64 $(basename "$f") reads x29 without establishing it: $offenders"
done

# RISC-V: `call __tls_get_addr` clobbers t0, the codegen accumulator. Any read
# of t0 after the call that no write precedes is a stale register-cache hit
# (gd_acc.c `w2` stored the resolver's leftover t0 instead of `v`). Operand 1
# is the destination except for stores and branches, which only read.
riscv_stale_t0() {
    awk '
        /^[A-Za-z_][A-Za-z_0-9$.]*:/ { fn = $1; armed = 0; next }
        /call[ \t]+__tls_get_addr/ { armed = 1; next }
        armed && /^[ \t]*[a-z]/ {
            line = $0; sub(/^[ \t]+/, "", line); n = split(line, p, /[ \t,()]+/)
            first = (p[1] ~ /^(s[bhwd]|beq|bne|blt|bge|bltu|bgeu|beqz|bnez)$/) ? 2 : 3
            for (i = first; i <= n; i++) if (p[i] == "t0") { print fn " " line; armed = 0; next }
            if (p[2] == "t0" || p[1] ~ /^(ret|call|tail|j|jr)$/) armed = 0
        }
    ' "$1"
}
# AArch64: the TLSDESC `blr` is a full call to the text peephole, so the
# allocator must keep every caller-saved pool register (x4-x8, x13, x14) out
# of homes that span it. A pool register read after the `blr` that no write
# precedes is such a home.
aarch64_pool_across_tlsdesc() {
    awk '
        /^[A-Za-z_][A-Za-z_0-9$.]*:/ { fn = $1; armed = 0; td = 0; next }
        /\.tlsdesccall/ { td = 1; next }
        td && /blr[ \t]+x1/ { armed = 1; td = 0; split("", w); next }
        armed && /^[ \t]*[a-z]/ {
            line = $0; sub(/^[ \t]+/, "", line); n = split(line, p, /[ \t,\[\]!]+/)
            ro = (p[1] ~ /^(str|strb|strh|stp|cmp|cmn|tst|cbz|cbnz)$/)
            for (i = ro ? 2 : 3; i <= n; i++) {
                r = p[i]; sub(/^[xw]/, "", r)
                if (p[i] ~ /^[xw]([4-8]|13|14)$/ && !(r in w)) { print fn " " line; armed = 0; next }
            }
            if (!ro && p[2] ~ /^[xw][0-9]+$/) { r = p[2]; sub(/^[xw]/, "", r); w[r] = 1 }
            if (p[1] ~ /^(b|bl|blr|br|ret)$/) armed = 0
        }
    ' "$1"
}
for f in "$TD"/riscv64-*.s; do
    stale=$(riscv_stale_t0 "$f")
    [ -z "$stale" ] || bad "RISC-V $(basename "$f") reads a resolver-clobbered t0: $stale"
done
for f in "$TD"/aarch64-*.s; do
    held=$(aarch64_pool_across_tlsdesc "$f")
    [ -z "$held" ] || bad "AArch64 $(basename "$f") keeps a caller-saved home across a TLSDESC call: $held"
done

# wide.c pins the discipline the hidden i128 libcall requires: __divti3 clobbers
# a0-a7, so the resolver's result must be parked in a callee-saved register
# rather than left in a0, and the function must get a real s0 frame instead of
# the RA-only one. This is the shape that used to return the quotient twice.
rv_wide="$TD/riscv64-wide-combine.s"
sed -n '/^combine:/,/^\.size combine/p' "$TD/riscv64-wide.s" >"$rv_wide"
grep -q 'call __tls_get_addr' "$rv_wide" || bad 'RISC-V wide fixture lost the hidden resolver call'
grep -q 'call __divti3' "$rv_wide" || bad 'RISC-V wide fixture lost the hidden i128 libcall'
if ! awk '
    /call __tls_get_addr/ { seen = 1; next }
    seen && /^[ \t]*mv s[0-9]+, a0/ { parked = 1 }
    END { exit !parked }
' "$rv_wide"; then
    bad 'RISC-V dynamic-TLS address was not parked in a callee-saved register across __divti3'
fi
grep -Eq '^[ \t]*addi[ \t]+s0,[ \t]*sp,' "$rv_wide" \
    || bad 'RISC-V function with a hidden libcall took the RA-only frame'

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

    # The wide-type fixture against the same shared TLS definitions: combine()
    # must return address plus quotient, not the quotient twice. This is the
    # executable form of the register-discipline assertion above.
    "$gcc" -no-pie "$TD/$arch-wide.o" -L"$TD" -Wl,-rpath,"$TD" -l"$arch-tls" -o "$TD/$arch-wide"
    LD_LIBRARY_PATH="$TD" "$qemu" -L "$sysroot" "$TD/$arch-wide" || bad "$arch wide-type TLS runtime"

    # gd_acc.c against a GCC host that defines ext_tls. LD_BIND_NOT=1 keeps
    # every resolver call on the lazy PLT path, whose stub clobbers t0: the
    # bound fast path happens to preserve it and hid the stale-cache read.
    "$gcc" -shared "$TD/$arch-gdacc.o" -o "$TD/lib$arch-gdacc.so"
    "$gcc" -O2 "$FIX/gd_acc_main.c" -L"$TD" -Wl,-rpath,"$TD" -l"$arch-gdacc" -o "$TD/$arch-gdacc"
    "$qemu" -L "$sysroot" -E LD_BIND_NOT=1 -E LD_LIBRARY_PATH="$TD" "$TD/$arch-gdacc" \
        || bad "$arch scratch register across the TLS resolver (LD_BIND_NOT=1)"
    note "$arch IE/dynamic/wide/resolver-scratch runtime interoperability"
done

[ "$fail" -eq 0 ] || exit 1
echo 'cross TLS models: PASS'
