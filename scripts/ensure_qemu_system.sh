#!/usr/bin/env bash
# ============================================================================
# ensure_qemu_system.sh — provision the pinned qemu-system-x86_64 (11.1.3)
#
# Why this exists, and why it is not in the tag hash cache
# -------------------------------------------------------
# `qemu_boot_test.sh` boots an LCCC-built bzImage in a system emulator, and the
# version it runs on is part of the result: TCG's instruction decoding, its
# CPU model defaults and its firmware loading all differ between releases. The
# tag hash cache carries QEMU payloads only for USER-mode emulation
# (`harness-cache-3`: `qemu-{aarch64,arm,i386,riscv64}.xz`), so there is no
# cached system emulator to restore, and a `main`-branch patch cannot add one —
# the binary blobs live inside orphan tags that a branch patch must never
# rewrite or relabel (see
# engineering/FOLLOWUP-2026-10-27-tag-hash-cache-refresh.md §1).
#
# What this script does instead is provision the SAME release the cache pins
# for user mode — QEMU 11.1.3, which is the latest public QEMU release and the
# version `ensure_qemu_user.sh` holds at `QEMU_VERSION=11.1.3` — by the same
# mechanism that script already uses: a `.deb` from Debian sid pinned BY
# CONTENT, so a mirror that silently re-spins a package cannot change what the
# harness runs.
#
# Pins (Debian sid, dists/sid/main/binary-amd64/Packages.xz, 2026-11-17; every
# .deb below was downloaded in full and checked against its SHA-256 from that
# signed index, and the SHA-1 is the digest of those same verified bytes, used
# only as the snapshot.debian.org content-addressed fallback key):
#
#   qemu-system-x86     1:11.1.3+ds-1   the emulator itself
#   qemu-system-common  1:11.1.3+ds-1   qemu-pr-helper, shared data
#   qemu-system-data    1:11.1.3+ds-1   firmware/ROM directory (`-L` target)
#   seabios             1.17.0-1        bios-256k.bin, the PC BIOS QEMU loads
#   ipxe-qemu           2.0.0+dfsg-5    efi-*.rom option ROMs
#   libc6               2.43-7          see "glibc bootstrap" below
#   libgcc-s1           16.2.0-4        "
#   libslirp0           4.9.5-1         "
#
# glibc bootstrap — why three library packages are part of the pin
# ----------------------------------------------------------------
# sid's qemu-system-x86 11.1.3 requires `GLIBC_2.42`, `GLIBC_2.43` and
# `SLIRP_4.9`. A Debian 13 host ships glibc 2.41 (measured: 2.41-12+deb13u4),
# so the binary does not start there:
#
#   /lib/x86_64-linux-gnu/libm.so.6: version `GLIBC_2.43' not found
#   /lib/x86_64-linux-gnu/libslirp.so.0: version `SLIRP_4.9' not found
#
# Rather than fall back to the host's older emulator, the prefix carries the
# glibc the binary was built against and starts it through that glibc's own
# loader, with the prefix's libraries first and the host's as the fallback for
# everything the host already satisfies (`bin/qemu-system-x86_64` is a wrapper
# that does exactly this). This is the same shape as any prefix toolchain: no
# root, no system change, nothing outside the prefix directory.
#
# Layout produced (all under the prefix):
#   bin/qemu-system-x86_64        wrapper: prefix loader + library path
#   usr/bin/qemu-system-x86_64    the real binary (dpkg layout, untouched)
#   usr/lib{,64}/…                sid libc6, libgcc-s1, libslirp0, ld.so
#   share/qemu/           symlink UNION of the two firmware dirs below: all of
#                         usr/share/qemu plus every usr/share/seabios file whose
#                         name qemu does not already provide
#   share/seabios -> ../usr/share/seabios   the dpkg dir, kept for harnesses that
#                         take a separate BIOS directory
#   usr/share/qemu/       qemu-system-data + ipxe-qemu: linuxboot_dma.bin,
#                         kvmvapic.bin, efi-*.rom
#   usr/share/seabios/    seabios: bios-256k.bin
#
# `share/qemu` is a single self-contained firmware directory — it holds all five
# files the harnesses require (bios-256k.bin, linuxboot_dma.bin, kvmvapic.bin,
# efi-e1000.rom, vgabios-stdvga.bin) — because the three QEMU consumers in this
# tree disagree about
# layout: `qemu_boot_test.sh` accepts Debian's split layout and symlinks a union
# itself, but `kernel_isolate.sh` requires one directory holding all four and
# exits 1 otherwise. Shipping the union satisfies both with no environment
# variable, and lets `qemu_boot_test.sh` take its "prefer a single
# self-contained directory" fast path. See _firmware_ok.
#
# Usage:
#   ensure_qemu_system.sh [PREFIX] [--print-bin] [--print-env] [--self-test]
#
#   PREFIX          install location; defaults to
#                   ${LCCC_QEMU_SYSTEM_PREFIX:-$HOME/.cache/lccc/qemu-system-11.1.3}
#   --print-bin     print the wrapper path and nothing else (for `QEMU=$(…)`)
#   --print-env     print `export` lines for QEMU and the firmware directories,
#                   for `eval "$(… --print-env)"`
#   --self-test     run the internal checks against temporary fake prefixes
#
# Environment:
#   QEMU_DL_DIR     .deb download cache (default $HOME/.cache/dl), shared with
#                   ensure_qemu_user.sh: a pre-placed .deb there must hash to
#                   the pin, exactly as in that script.
#
# Point the boot test at the result with either of:
#   QEMU=$(scripts/ensure_qemu_system.sh --print-bin) scripts/qemu_boot_test.sh
#   eval "$(scripts/ensure_qemu_system.sh --print-env)"  # QEMU + firmware dirs
# ============================================================================
set -euo pipefail

QEMU_VERSION=11.1.3
DEB_VERSION=11.1.3+ds-1

# package<TAB>version<TAB>pool path<TAB>bytes<TAB>sha256<TAB>sha1
PINS=$(cat <<'TABLE'
qemu-system-x86	1:11.1.3+ds-1	pool/main/q/qemu/qemu-system-x86_11.1.3+ds-1_amd64.deb	10782144	576518c3b59477bc4f94982b34e771a6948ab6143a66f98cb7e032839e9db578	e89800ac332f3a7b7d4593ae4451ab138837952a
qemu-system-common	1:11.1.3+ds-1	pool/main/q/qemu/qemu-system-common_11.1.3+ds-1_amd64.deb	1014272	99b406a732b5573af2ec828b4118ed6634fa5920e5131a1fb8c0aded5e995327	5d5436d3c8ead45482026f41936ee1f667c9c58f
qemu-system-data	1:11.1.3+ds-1	pool/main/q/qemu/qemu-system-data_11.1.3+ds-1_all.deb	2223616	17575e9e4d9388a235920a82c7065c38a5bc0d98d2e773134fd915c90bc0c847	d9538a004c350811a0397a7658469f2114f0bb6e
seabios	1.17.0-1	pool/main/s/seabios/seabios_1.17.0-1_all.deb	158720	3b957cbc305adaeafec78db2575bd122107647c653ba8eaa4f4348f9153295d0	5da61823e803e352028fe817328431c4a853e913
ipxe-qemu	2.0.0+dfsg-5	pool/main/i/ipxe/ipxe-qemu_2.0.0+dfsg-5_all.deb	1056244	4d566dc0b7e3e5b2afb0061f3a2f83784ce36368775d5196c93e5bb6e68b05e8	b558d0b6320b6fc6d0908445ea6eb5be59597bd1
libc6	2.43-7	pool/main/g/glibc/libc6_2.43-7_amd64.deb	1866392	5c71715c51103beb4fbba9fe9f8dff604af6610dfa71c70bff342f54a7860803	d3b47ff25250d1e0065ca0b39417fc55b4e8e341
libgcc-s1	16.2.0-4	pool/main/g/gcc-16/libgcc-s1_16.2.0-4_amd64.deb	74120	d251ea642c3795e8723ac2b9eeebe511467ca04590ba70f3d555a7616787d7e3	b24b594d16496c6cc51b608a973a39c75e594635
libslirp0	4.9.5-1	pool/main/libs/libslirp/libslirp0_4.9.5-1_amd64.deb	71872	3fad48bcc3830a8d236678042e246121cfac26931311874d0a69f862dd82e5d3	d2bc104806f4d9a428507292f809dcf0d54288b6
TABLE
)

# The four files a QEMU firmware directory must hold for these harnesses.
# vgabios-stdvga.bin belongs to the contract because `-machine pc` defaults to
# `-vga std` and resolves that ROM from the -L directory the harnesses pass,
# which replaces the compiled-in data dir. See scripts/qemu_firmware.sh.
FIRMWARE=(bios-256k.bin linuxboot_dma.bin kvmvapic.bin efi-e1000.rom vgabios-stdvga.bin)

print_bin=0
print_env=0
self_test=0
prefix=
for arg in "$@"; do
    case $arg in
        --print-bin) print_bin=1 ;;
        --print-env) print_env=1 ;;
        --self-test) self_test=1 ;;
        # Print the header block only: from after the opening rule up to (but
        # not including) the closing rule. Deriving the range from the rule
        # itself keeps --help correct when the header grows; a hardcoded line
        # number silently starts printing code and truncated pin rows.
        -h|--help) sed -n '3,/^# ====/p' "$0" | sed '$d'; exit 0 ;;
        --*) echo "ensure_qemu_system: unknown argument: $arg" >&2; exit 2 ;;
        *) [ -z "$prefix" ] || { echo "ensure_qemu_system: only one prefix may be given" >&2; exit 2; }
           prefix=$arg ;;
    esac
done
prefix=${prefix:-${LCCC_QEMU_SYSTEM_PREFIX:-$HOME/.cache/lccc/qemu-system-$QEMU_VERSION}}
dl_dir=${QEMU_DL_DIR:-$HOME/.cache/dl}
bindir=$prefix/bin
wrapper=$bindir/qemu-system-x86_64

_sha256_is() {  # _sha256_is <file> <expected-hex>: whole-digest equality
    local got
    got=$(sha256sum "$1" 2>/dev/null | awk '{print $1}') || return 1
    [[ $got == "$2" ]]
}

# _loader <prefix>: the prefix's own ld.so, or empty when there is none.
_loader() {
    local p=$1 c
    for c in "$p/usr/lib64/ld-linux-x86-64.so.2" "$p/usr/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2"; do
        [ -e "$c" ] && { printf '%s\n' "$c"; return 0; }
    done
    return 1
}

# _firmware_ok <prefix>: every required firmware file is present in ONE
# self-contained directory, `share/qemu`.
#
# The single-directory rule is deliberately stricter than any one consumer needs,
# because the three QEMU harnesses in this tree disagree:
#   * qemu_boot_test.sh prefers one self-contained dir but will resolve the four
#     files individually across share/qemu + share/seabios and symlink a union
#     itself (its Debian-split-layout fallback);
#   * kernel_isolate.sh requires one dir holding all four, with no fallback
#     ("no firmware union dir" and exit 1 otherwise);
#   * qemu_qmp_probe.py auto-detects a share/qemu next to the binary holding the
#     full union.
# Presenting the union in share/qemu satisfies all three with no environment
# variable, and lets qemu_boot_test.sh take its fast path instead of building a
# throwaway union under $WORK on every run.
_firmware_ok() {
    local p=$1 f
    for f in "${FIRMWARE[@]}"; do
        [[ -f $p/share/qemu/$f ]] || return 1
    done
    return 0
}

# _version_ok <prefix>: the acceptance rule for an installed prefix. All three
# parts must hold, because each catches a different failure:
#   1. the wrapper runs and reports the pinned version as a WHOLE token on its
#      first --version line — rejects the host's older emulator on PATH and a
#      lookalike such as "11.1.30";
#   2. -machine help lists a pc machine and -cpu help offers qemu64 — proves the
#      binary really initializes (its glibc bootstrap resolves, its machine
#      registry loads) and that it offers the CPU model qemu_boot_test.sh
#      passes; --version alone would not notice a broken loader path, since it
#      exits before machine init;
#   3. the four firmware files qemu_boot_test.sh requires are present.
_version_ok() {
    local p=$1 line
    [ -x "$p/bin/qemu-system-x86_64" ] || return 1
    line=$("$p/bin/qemu-system-x86_64" --version 2>/dev/null | head -n 1) || return 1
    case $line in
        *" version $QEMU_VERSION "*|*" version $QEMU_VERSION") ;;
        *) return 1 ;;
    esac
    timeout 120 "$p/bin/qemu-system-x86_64" -machine help 2>/dev/null | grep -qE '^[[:space:]]*pc' || return 1
    timeout 120 "$p/bin/qemu-system-x86_64" -cpu help 2>/dev/null | grep -qw 'qemu64' || return 1
    _firmware_ok "$p"
}

# ---- self-test: the acceptance rules, exercised without a download -----------
if [[ $self_test == 1 ]]; then
    for tool in sha256sum timeout; do
        command -v "$tool" >/dev/null 2>&1 || { echo "self-test: $tool missing" >&2; exit 1; }
    done
    td=$(mktemp -d)
    trap 'rm -rf "$td"' EXIT
    mkdir -p "$td/good/bin" "$td/good/share/qemu" "$td/good/share/seabios" \
             "$td/oldver/bin" "$td/oldver/share/qemu" "$td/oldver/share/seabios" \
             "$td/lookalike/bin" "$td/lookalike/share/qemu" "$td/lookalike/share/seabios" \
             "$td/nofw/bin" "$td/nofw/share/qemu" "$td/nofw/share/seabios" \
             "$td/splitfw/bin" "$td/splitfw/share/qemu" "$td/splitfw/share/seabios"
    # A fake wrapper stands in for the real one: --version, -machine help and
    # -cpu help are the whole contract _version_ok checks. The firmware files
    # are the four required names.
    _mk() {  # _mk <dir> <banner-version>
        cat > "$1/bin/qemu-system-x86_64" <<FAKE
#!/bin/sh
case "\$1" in
    --version) printf 'QEMU emulator version $2 (Debian 1:$2+ds-1)\n' ;;
    -machine)  printf 'Supported machines are:\npc                   Standard PC (i440FX + PIIX, 1996)\n' ;;
    -cpu)      printf 'Available CPUs:\nqemu64               (alias configured by machine type)\n' ;;
esac
FAKE
        chmod +x "$1/bin/qemu-system-x86_64"
    }
    _mk "$td/good"     "$QEMU_VERSION"
    _mk "$td/oldver"   "10.0.13"
    _mk "$td/lookalike" "11.1.30"
    _mk "$td/nofw"     "$QEMU_VERSION"
    _mk "$td/splitfw"  "$QEMU_VERSION"
    # `noboot` is the regression this rule exists for: a wrapper that answers
    # --version correctly but cannot actually start the emulator (observed when
    # the wrapper baked in the absolute path of the staging directory it was
    # assembled in, which is deleted once the prefix moves into place). --version
    # exits before machine init, so only -machine help notices.
    mkdir -p "$td/noboot/bin" "$td/noboot/share/qemu" "$td/noboot/share/seabios"
    cat > "$td/noboot/bin/qemu-system-x86_64" <<FAKE
#!/bin/sh
case "\$1" in
    --version) printf 'QEMU emulator version $QEMU_VERSION (Debian 1:$QEMU_VERSION+ds-1)\n' ;;
    *) echo "exec: loader: not found" >&2; exit 127 ;;
esac
FAKE
    chmod +x "$td/noboot/bin/qemu-system-x86_64"
    # Every case except `nofw` carries a self-contained union — all four
    # required files in share/qemu — which is the shape the prefix must present.
    for d in good oldver lookalike noboot splitfw; do
        for f in "${FIRMWARE[@]}"; do : > "$td/$d/share/qemu/$f"; done
    done
    # `splitfw` is the raw Debian split layout instead: complete ACROSS the two
    # directories, but no single one holds all four. qemu_boot_test.sh would
    # paper over that by symlinking a throwaway union under $WORK;
    # kernel_isolate.sh would exit 1 with "no firmware union dir". The prefix
    # must not ship it, so the acceptance rule rejects it.
    rm -f "$td/splitfw/share/qemu/bios-256k.bin"
    : > "$td/splitfw/share/seabios/bios-256k.bin"
    _version_ok "$td/good"      || { echo "self-test FAIL: pinned prefix rejected" >&2; exit 1; }
    _version_ok "$td/oldver"  && { echo "self-test FAIL: 10.0.13 accepted" >&2; exit 1; }
    _version_ok "$td/lookalike" && { echo "self-test FAIL: 11.1.30 accepted" >&2; exit 1; }
    _version_ok "$td/nofw"    && { echo "self-test FAIL: incomplete firmware accepted" >&2; exit 1; }
    _version_ok "$td/splitfw" && { echo "self-test FAIL: split firmware layout accepted" >&2; exit 1; }
    _version_ok "$td/noboot"  && { echo "self-test FAIL: wrapper that cannot boot accepted" >&2; exit 1; }
    _version_ok "$td/missing" && { echo "self-test FAIL: absent prefix accepted" >&2; exit 1; }
    echo "ensure_qemu_system: self-test PASS (version + bootability + firmware rules)"
    exit 0
fi

for tool in curl sha256sum timeout; do
    command -v "$tool" >/dev/null 2>&1 || { echo "ensure_qemu_system: $tool is required" >&2; exit 2; }
done
command -v dpkg-deb >/dev/null 2>&1 || {
    echo "ensure_qemu_system: dpkg-deb is required to unpack the .deb files" >&2; exit 2; }

if _version_ok "$prefix"; then
    if [[ $print_bin == 1 ]]; then printf '%s\n' "$wrapper"; exit 0; fi
    if [[ $print_env == 1 ]]; then
        printf 'export QEMU=%s\n' "$wrapper"
        printf 'export QEMU_DATA_DIR=%s\n' "$prefix/share/qemu"
        printf 'export QEMU_ROM_DIR=%s\n' "$prefix/share/qemu"
        printf 'export QEMU_BIOS_DIR=%s\n' "$prefix/share/seabios"
        exit 0
    fi
    echo "ensure_qemu_system: $prefix already holds qemu-system-x86_64 $QEMU_VERSION"
    echo "  QEMU=$wrapper"
    exit 0
fi

mkdir -p "$dl_dir"
staging=$(mktemp -d "${TMPDIR:-/tmp}/lccc-qemu-system.XXXXXX")
trap 'rm -rf "$staging"' EXIT
mkdir -p "$staging/root"

# ---- fetch and verify every pinned .deb ------------------------------------
while IFS=$'\t' read -r pkg ver pool size sha256 sha1; do
    [[ -z $pkg ]] && continue
    deb=$dl_dir/$(basename "$pool")
    if [[ -f $deb ]] && _sha256_is "$deb" "$sha256"; then
        echo "ensure_qemu_system: cached  $pkg $ver"
    else
        echo "ensure_qemu_system: fetching $pkg $ver ($(numfmt --to=iec "${size:-0}" 2>/dev/null || echo "$size") bytes)"
        ok=0
        for url in "https://deb.debian.org/debian/$pool" "https://snapshot.debian.org/file/$sha1"; do
            if curl -fsSL --retry 3 --retry-delay 2 -o "$deb.part" "$url" 2>/dev/null; then
                if _sha256_is "$deb.part" "$sha256"; then
                    mv "$deb.part" "$deb"; ok=1; break
                fi
                echo "  digest MISMATCH from $url — trying the next source" >&2
                rm -f "$deb.part"
            fi
        done
        [[ $ok == 1 ]] || { echo "ensure_qemu_system: cannot fetch a verified $pkg $ver" >&2; exit 1; }
    fi
    dpkg-deb -x "$deb" "$staging/root"
done <<< "$PINS"

# ---- assemble the prefix ----------------------------------------------------
mkdir -p "$staging/prefix/bin" "$staging/prefix/share"
cp -a "$staging/root/." "$staging/prefix/"
ln -sfn ../usr/share/seabios "$staging/prefix/share/seabios"

# share/qemu is a REAL directory holding a symlink union of both dpkg firmware
# directories, so one `-L share/qemu` satisfies every harness (see
# _firmware_ok for why a split layout is not enough). Relative targets keep the
# union valid when the prefix moves. qemu-system-data's own files win over
# seabios's same-named ones: those are the copies QEMU 11.1.3 was built and
# tested against, and a SeaBIOS vgabios from another release must not shadow
# them. seabios only fills the gaps — which is precisely why the union is needed
# at all, since bios-256k.bin ships in seabios and nowhere else.
mkdir -p "$staging/prefix/share/qemu"
for f in "$staging/prefix/usr/share/qemu"/*; do
    [[ -e $f ]] || continue
    ln -sfn "../../usr/share/qemu/$(basename "$f")" \
        "$staging/prefix/share/qemu/$(basename "$f")"
done
for f in "$staging/prefix/usr/share/seabios"/*; do
    [[ -e $f ]] || continue
    b=$(basename "$f")
    [[ -e $staging/prefix/share/qemu/$b ]] && continue
    ln -sfn "../../usr/share/seabios/$b" "$staging/prefix/share/qemu/$b"
done

loader=$(_loader "$staging/prefix") || {
    echo "ensure_qemu_system: the glibc package did not provide an ld.so" >&2; exit 1; }
libdir=${loader%/*}
[[ $libdir == */lib64 ]] && libdir=$libdir/../lib/x86_64-linux-gnu

# The wrapper locates everything from its own position, so the prefix keeps
# working after it is moved into place (and after any later relocation). Baking
# in an absolute path here would be wrong twice over: the staging directory is
# deleted once the prefix moves, and a relocated prefix would break silently.
cat > "$staging/prefix/bin/qemu-system-x86_64" <<'WRAPPER'
#!/bin/sh
# Generated by scripts/ensure_qemu_system.sh — do not edit.
#
# qemu-system-x86_64 11.1.3 (Debian sid 1:11.1.3+ds-1), started through the
# glibc it was built against: sid's binary requires GLIBC_2.42/GLIBC_2.43 and
# SLIRP_4.9, which a Debian 13 host (glibc 2.41) does not provide. The prefix's
# libraries come first; the host's are the fallback for everything it already
# satisfies. Nothing outside this prefix is modified.
set -e
prefix=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
libdir="$prefix/usr/lib/x86_64-linux-gnu"
loader="$prefix/usr/lib64/ld-linux-x86-64.so.2"
[ -e "$loader" ] || loader="$libdir/ld-linux-x86-64.so.2"
[ -e "$loader" ] || { echo "$0: no ld.so in $prefix (prefix incomplete)" >&2; exit 127; }
exec "$loader" --library-path "$libdir:/usr/lib/x86_64-linux-gnu:/lib/x86_64-linux-gnu" \
    "$prefix/usr/bin/qemu-system-x86_64" "$@"
WRAPPER
chmod +x "$staging/prefix/bin/qemu-system-x86_64"

_version_ok "$staging/prefix" || {
    echo "ensure_qemu_system: the assembled prefix failed its own acceptance check" >&2
    echo "  (loader: $loader, libdir: $libdir)" >&2
    exit 1
}

mkdir -p "$(dirname "$prefix")"
rm -rf "$prefix"
mv "$staging/prefix" "$prefix"
trap - EXIT
rm -rf "$staging"

# Re-check in the FINAL location: the acceptance rules must hold for the
# artifact as installed, not only for the staging copy it was assembled from.
_version_ok "$prefix" || {
    echo "ensure_qemu_system: the installed prefix at $prefix is not usable" >&2
    rm -rf "$prefix"
    exit 1
}

if [[ $print_bin == 1 ]]; then printf '%s\n' "$wrapper"; exit 0; fi
if [[ $print_env == 1 ]]; then
    printf 'export QEMU=%s\n' "$wrapper"
    printf 'export QEMU_DATA_DIR=%s\n' "$prefix/share/qemu"
    printf 'export QEMU_ROM_DIR=%s\n' "$prefix/share/qemu"
    printf 'export QEMU_BIOS_DIR=%s\n' "$prefix/share/seabios"
    exit 0
fi
echo "ensure_qemu_system: installed qemu-system-x86_64 $QEMU_VERSION at $prefix"
echo "  QEMU=$wrapper"
echo "  firmware: $prefix/share/qemu (+ $prefix/share/seabios)"
