#!/usr/bin/env bash
# ============================================================================
# prepare_kernel_tree.sh — regenerate the patched linux-cachymod-6.18.55 tree
# used by build_kernel_boot.sh / build_kernel_vm.sh / realmode_corpus.sh.
#
# The persisted workspace snapshot is size-capped (~128 MiB / 10k files), so the
# ~55k-file kernel tree and the 155 MiB tarball do NOT fit in a size-capped
# workspace snapshot, so they are lost between sessions.  This script makes the
# tree cheap to regenerate deterministically (~3 min): download, verify,
# extract, apply the CachyMod patch series, configure with the package's real
# config, run `make prepare` with LCCC, and create the boot-code stubs
# (capflags.c, utsversion.h, zoffset.h, voffset.h) that a full Kbuild would
# otherwise generate.
#
# Single sources of truth (read from the archpkgbuilds PKGBUILD, never copied):
#   * version      _major / _minor          (-> KVER)
#   * release      pkgrel                   (-> localversion.10-pkgrel)
#   * tarball      sha256sums[0]            (verified before extraction)
#   * patch list   source=() *.patch order, with prepare()'s own filters
#                  (-prevent-avx2, -prjc, -prjc-s) — so a PKGBUILD bump is
#                  picked up without touching this script.
#
# Compiler policy: kernel translation units are compiled ONLY by LCCC.
# `make prepare` builds include/generated/asm-offsets.h from
# kernel/asm-offsets.c, which is kernel code, so it runs with CC=$LCCC.  Host
# tools (kconfig, fixdep, mkcpustr, ...) stay on HOSTCC (default gcc): they are build
# machinery, not generated code.  The stamp records the compiler, and a stamp
# written by an older GCC-prepared tree forces a re-run under LCCC.
#
# Idempotent: a completed tree is stamped with .lccc-prepared and skipped.
# The canary set below must all exist; the ~10k-file snapshot cap truncates
# large trees, so a damaged tree regenerates instead of half-working.
LCCC_PREPARED_CANARIES=(
  # Alphabetically and structurally distributed source/build sentinels.  The
  # snapshot file cap once retained every generated header below but dropped
  # the central Kbuild include, yielding a misleading late "No rule" failure.
  Makefile
  scripts/Kbuild.include
  init/main.c
  kernel/workqueue.c
  arch/x86/boot/setup.ld
  include/generated/autoconf.h
  include/generated/utsversion.h
  arch/x86/include/generated/asm/rwonce.h
  arch/x86/kernel/cpu/capflags.c
  arch/x86/boot/zoffset.h
  arch/x86/boot/voffset.h
  arch/x86/boot/cpustr.h
)
#
# Usage:
#   prepare_kernel_tree.sh [kernel-dir]          (default: $KERNEL_WORK/linux-$KVER)
# Environment:
#   KERNEL_WORK  scratch directory for the tarball and tree
#            (default: $XDG_CACHE_HOME/lccc/kernel-work, else ~/.cache/lccc/kernel-work).
#            Kept out of the repository so repository-wide scans never see the
#            kernel sources.
#   PKG_ROOT archpkgbuilds checkout root
#            (default: $XDG_CACHE_HOME/lccc/archpkgbuilds, else ~/.cache/lccc/archpkgbuilds).
#            Must be empty, absent, or a checkout of $PKG_REPO: the script
#            clears it before cloning, so anything else is refused.
#            Both defaults sit in the cache directory, which the workspace
#            snapshot excludes, so the ~55k-file tree is never persisted.
#   HOSTCC   host compiler for build tools (default: gcc).  Kernel TUs always
#            use LCCC.
#   PKGDIR   archpkgbuilds sparse checkout of packages/linux-cachymod-6.18
#            (default: $PKG_ROOT/packages/linux-cachymod-6.18)
#   KVER     kernel version (default: derived from PKGBUILD _major.minor)
#   LCCC     compiler for kernel TUs (default: <repo>/target/fastbuild/lccc)
#   LCCC_CPUSCHED / LCCC_PREVENT_AVX2   PKGBUILD option mirrors (eevdf / no)
#   LCCC_PKGREL  override pkgrel (default: PKGBUILD pkgrel)
# ============================================================================
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
LCCC=${LCCC:-$REPO_ROOT/target/fastbuild/lccc}
LCCC_CACHE=${XDG_CACHE_HOME:-$HOME/.cache}/lccc
KERNEL_WORK=${KERNEL_WORK:-$LCCC_CACHE/kernel-work}
HOSTCC=${HOSTCC:-gcc}

# The CachyMod patches and the package config live in a sibling repository,
# which the persisted workspace does not keep: a fresh machine has the tarball
# URL but nothing to patch with, so this script used to die before downloading
# anything.  Fetch it bloblessly (one directory of one repo) instead, which makes
# the script the only prerequisite of every kernel gate.
PKG_REPO=${PKG_REPO:-https://github.com/ms178/archpkgbuilds.git}
PKG_ROOT=${PKG_ROOT:-$LCCC_CACHE/archpkgbuilds}
# PKGDIR is *derived* from PKG_ROOT unless the caller pinned it explicitly.
# It used to be an absolute default (a per-user checkout path), so setting
# PKG_ROOT alone left the two out of step: `rel` below stayed an absolute path
# and `git sparse-checkout set` rejected it with "specify directories rather
# than patterns (no leading slash)" — the kernel gate died before downloading
# anything.  Keeping the kernel tree off the (size-capped) workspace snapshot
# requires exactly that override, so the default must follow it.
PKGDIR=${PKGDIR:-$PKG_ROOT/packages/linux-cachymod-6.18}
ensure_pkgdir() {
  [[ -d $PKGDIR ]] && return 0
  command -v git >/dev/null 2>&1 || return 1
  echo "prepare_kernel_tree: fetching package sources from $PKG_REPO"
  local rel=${PKGDIR#"$PKG_ROOT"/} branch
  [[ $rel != /* ]] || { echo "prepare_kernel_tree: PKGDIR must live under PKG_ROOT" >&2; return 1; }
  # Clear PKG_ROOT only when it is provably ours: absent, empty, or a git
  # checkout of $PKG_REPO.  The directory itself is kept (its parent may be
  # owned by someone else), and anything else is refused, so a mistyped
  # PKG_ROOT such as $HOME is never wiped.
  if [[ -e $PKG_ROOT ]]; then
    if [[ -d $PKG_ROOT && -z $(ls -A -- "$PKG_ROOT") ]]; then
      :
    elif [[ -d $PKG_ROOT/.git ]] \
         && [[ $(git -C "$PKG_ROOT" remote get-url origin 2>/dev/null) == "$PKG_REPO" ]]; then
      find "$PKG_ROOT" -mindepth 1 -maxdepth 1 -exec rm -rf -- {} +
    else
      echo "prepare_kernel_tree: refusing to clear PKG_ROOT=$PKG_ROOT" >&2
      echo "  (not empty and not a checkout of $PKG_REPO); point PKG_ROOT elsewhere" >&2
      return 1
    fi
  fi
  mkdir -p -- "$PKG_ROOT" || return 1
  git clone --quiet --filter=blob:none --no-checkout --depth 1 "$PKG_REPO" "$PKG_ROOT" || return 1
  branch=$(git -C "$PKG_ROOT" symbolic-ref --short HEAD 2>/dev/null || echo main)
  git -C "$PKG_ROOT" sparse-checkout init --cone || return 1
  git -C "$PKG_ROOT" sparse-checkout set "$rel" || return 1
  git -C "$PKG_ROOT" checkout --quiet --force "$branch" || return 1
  [[ -d $PKGDIR ]]
}
ensure_pkgdir || {
  echo "prepare_kernel_tree: PKGDIR not found and could not be fetched: $PKGDIR" >&2
  echo "  clone $PKG_REPO yourself, or point PKGDIR at an existing checkout" >&2
  exit 1
}
PKGBUILD="$PKGDIR/PKGBUILD"
[[ -r $PKGBUILD ]] || { echo "prepare_kernel_tree: no PKGBUILD in $PKGDIR" >&2; exit 1; }

# ---- PKGBUILD readers --------------------------------------------------------
pkgbuild_scalar() { # pkgbuild_scalar <name> -> value of `name=value` (first match)
  sed -n "s/^$1=\\([^ #]*\\).*/\\1/p" "$PKGBUILD" | head -n 1
}
# Patch names in source=() order, exactly the entries prepare() would consider.
# Comments are stripped first (the array carries commented-out URLs); URLs and
# unexpanded variables are not local patch files and are skipped.
pkgbuild_patch_sources() {
  awk '/^source=\(/{f=1; next} f && /^\)/{f=0} f' "$PKGBUILD" \
    | sed 's/#.*$//' | tr -s ' \t' '\n' | tr -d '"' \
    | grep -E '\.patch$' | grep -vE '[$:/]' || true
}

PKG_MAJOR=$(pkgbuild_scalar _major)
PKG_MINOR=$(pkgbuild_scalar _minor)
PKG_RELEASE=$(pkgbuild_scalar pkgrel)
[[ -n $PKG_MAJOR && -n $PKG_MINOR && -n $PKG_RELEASE ]] || {
  echo "prepare_kernel_tree: cannot read _major/_minor/pkgrel from $PKGBUILD" >&2
  exit 1
}
PKG_KVER="$PKG_MAJOR.$PKG_MINOR"
KVER=${KVER:-$PKG_KVER}
if [[ $KVER != "$PKG_KVER" ]]; then
  # A tree for a different package version than the PKGBUILD describes would
  # apply the wrong patch series.  Refuse rather than silently mix versions.
  echo "prepare_kernel_tree: KVER=$KVER but $PKGBUILD pins $PKG_KVER" >&2
  exit 1
fi
# First 64-hex entry of the sha256sums=() array (source[0], the tarball).  The
# array may start on its own line and carry comments, so read the block.
PKG_TARBALL_SHA256=$(sed -n '/^sha256sums=(/,/)/p' "$PKGBUILD" | sed 's/#.*$//' \
  | grep -oE '[0-9a-f]{64}' | head -n 1 || true)
[[ -n $PKG_TARBALL_SHA256 ]] || { echo "prepare_kernel_tree: no sha256sums[0] in $PKGBUILD" >&2; exit 1; }

KDIR=${1:-${KERNEL_DIR:-$KERNEL_WORK/linux-$KVER}}
# The script removes $KDIR before extracting into it; refuse anything that is not
# a linux-* source directory (a stray KERNEL_DIR=/ or $HOME must never be wiped).
case $(basename -- "$KDIR") in
  linux*) ;;
  *) echo "prepare_kernel_tree: refusing KDIR=$KDIR (basename must start with linux)" >&2; exit 1 ;;
esac
WORK=$(dirname "$KDIR")
TARBALL="$WORK/linux-$KVER.tar.xz"
PKGREL=${LCCC_PKGREL:-$PKG_RELEASE}
CPUSCHED=${LCCC_CPUSCHED:-eevdf}
PREVENT_AVX2=${LCCC_PREVENT_AVX2:-no}

# Portable helpers: GNU coreutils names first, BSD/macOS fallbacks second.
sha256_of() { # sha256_of <file> -> hex digest only
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -- "$1" | cut -d' ' -f1
  else
    shasum -a 256 -- "$1" | cut -d' ' -f1
  fi
}
jobs_count() {
  getconf _NPROCESSORS_ONLN 2>/dev/null || nproc 2>/dev/null || echo 2
}

# `make olddefconfig`/`make prepare` need host tools that are not part of the
# LCCC build and therefore easy to have absent on a fresh machine; failing
# there wastes the whole download+patch phase that precedes it.  Check first.
preflight_host_tools() {
  local missing=() tool comp cfgname
  # kconfig lexers/parsers, plus the unconditional build-time host tools.
  for tool in make "$HOSTCC" ld ar nm objcopy perl awk sed bc flex bison cpio \
              xz patch tar curl; do
    command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
  done
  # Compression tool implied by the package config (KERNEL_ZSTD=y upstream).
  # Preflight the config that step 4 installs ($PKGDIR/config) instead of
  # demanding every compressor some *other* config might select: lz4/lzop
  # absent must not block a zstd kernel (the old unconditional list did).
  local cfg="$PKGDIR/config"
  for comp in zstd lz4 lzop gzip bzip2 xz; do
    case $comp in
      zstd)  cfgname=ZSTD ;;
      lz4)   cfgname=LZ4 ;;
      lzop)  cfgname=LZO ;;
      gzip)  cfgname=GZIP ;;
      bzip2) cfgname=BZIP2 ;;
      xz)    cfgname=XZ ;;
    esac
    if [[ -r $cfg ]] && grep -q "^CONFIG_KERNEL_${cfgname}=y" "$cfg"; then
      command -v "$comp" >/dev/null 2>&1 \
        || missing+=("$comp (CONFIG_KERNEL_${cfgname}=y)")
    fi
  done
  command -v sha256sum >/dev/null 2>&1 || command -v shasum >/dev/null 2>&1 \
    || missing+=("sha256sum or shasum")
  if ((${#missing[@]})); then
    echo "prepare_kernel_tree: missing host tools: ${missing[*]}" >&2
    echo "  Build machines only; never built with LCCC." >&2
    echo "  Debian/Ubuntu: apt-get install build-essential flex bison libelf-dev \\" >&2
    echo "    libssl-dev bc cpio kmod dwarves zstd lz4 lzop bzip2" >&2
    echo "  (HOSTCC=$HOSTCC; set HOSTCC to another host compiler if needed)" >&2
    return 1
  fi
  echo "prepare_kernel_tree: host tools OK"
}
preflight_host_tools || exit 1

# Already prepared?  Verify the stamp AND a canary file the snapshot truncation
# removed last time (setup.ld); a damaged tree must regenerate, not half-work.
# A tree stamped by another compiler gets only the LCCC `make prepare` rerun.
compiler_id() { # identity of the kernel compiler, recorded in the stamp
  printf 'compiler=%s version=%s\n' "$LCCC" "$("$LCCC" --version 2>/dev/null | head -n 1)"
}
require_lccc() {
  [[ -x $LCCC ]] || {
    echo "prepare_kernel_tree: LCCC compiler missing: $LCCC" >&2
    echo "  Kernel translation units (asm-offsets.c) must be built by LCCC, never GCC." >&2
    echo "  Build it first: scripts/build_lccc_fast.sh" >&2
    exit 1
  }
}
# Configure + generate headers with LCCC for the kernel TUs; host tools via HOSTCC.
lccc_make_prepare() {
  # olddefconfig FIRST, under the same CC: it answers NEW symbols with defaults
  # without reading stdin, so the syncconfig inside `prepare` cannot prompt
  # (the kernel's cc-option visibility differs between gcc and lccc).
  make ARCH=x86_64 CC="$LCCC" HOSTCC="$HOSTCC" olddefconfig >/dev/null
  make ARCH=x86_64 CC="$LCCC" HOSTCC="$HOSTCC" prepare -j"$(jobs_count)"
}
canaries_ok() {
  local c
  for c in "${LCCC_PREPARED_CANARIES[@]}"; do
    [[ -f "$KDIR/$c" ]] || { echo "prepare_kernel_tree: canary missing: $c" >&2; return 1; }
  done
}
if [[ -f "$KDIR/.lccc-prepared" ]] && [[ -f "$KDIR/${LCCC_PREPARED_CANARIES[0]}" ]] && canaries_ok; then
  require_lccc
  if [[ $(cat "$KDIR/.lccc-prepared") == "$(compiler_id)" ]]; then
    echo "prepare_kernel_tree: $KDIR already prepared (stamp + canaries OK)"
    exit 0
  fi
  echo "prepare_kernel_tree: $KDIR was prepared by a different compiler; re-running LCCC make prepare"
  (cd "$KDIR" && lccc_make_prepare)
  (cd "$KDIR" && compiler_id > .lccc-prepared)
  echo "prepare_kernel_tree: $KDIR re-prepared with LCCC"
  exit 0
fi

mkdir -p "$WORK"
cd "$WORK"

# ---- 1. source tarball ------------------------------------------------------
# The workspace snapshot truncates large files (~128 MB cap): a stale tarball
# can be present but truncated. `xz -t` validates integrity cheaply (~2 s);
# a corrupt archive is re-downloaded instead of failing mid-extract.  The
# PKGBUILD's sha256 is the authority: an archive that passes `xz -t` but is
# not the kernel.org release is rejected.
tarball_ok() {
  [[ -f $TARBALL ]] && xz -t "$TARBALL" 2>/dev/null \
    && [[ $(sha256_of "$TARBALL") == "$PKG_TARBALL_SHA256" ]]
}
if ! tarball_ok; then
  echo "prepare_kernel_tree: downloading linux-$KVER.tar.xz"
  curl -fsSL --retry 3 -o "$TARBALL" "https://cdn.kernel.org/pub/linux/kernel/v${KVER%%.*}.x/linux-$KVER.tar.xz"
  tarball_ok || {
    echo "prepare_kernel_tree: download failed sha256 check against PKGBUILD ($PKG_TARBALL_SHA256)" >&2
    exit 1
  }
fi

# ---- 2. extract -------------------------------------------------------------
echo "prepare_kernel_tree: extracting"
rm -rf "$KDIR"
# tar may exit non-zero on benign "Directory renamed before its status could
# be extracted" warnings (a GNU tar quirk when a directory's metadata changes
# between tar's open and its later utime/chmod pass, seen under load on
# FUSE/overlayfs). Real corruption is caught by the sha256 above and by the
# sentinel check below; tolerate the warning stream but verify the tree.
tar_err=$(mktemp)
tar -xf "linux-$KVER.tar.xz" 2>"$tar_err" || true
grep -v "Directory renamed before its status could be extracted" "$tar_err" >&2 || true
rm -f "$tar_err"
# Post-extract integrity: check sentinel files spread across the tree (not just
# the top-level Makefile) before trusting it.  Early sentinels all live in the
# alphabetical head of the archive, so a mid-archive truncation passes them
# while later files are silently absent.  Compare the file count against the
# archive listing; a short tree is re-extracted once before giving up.
sentinels_ok() {
  local s
  for s in Makefile init/main.c arch/x86/Makefile kernel/sched/core.c include/linux/sched.h; do
    [[ -f "$KDIR/$s" ]] || return 1
  done
}
sentinels_ok || {
  echo "prepare_kernel_tree: extraction incomplete (sentinel missing); retrying" >&2
  rm -rf "$KDIR"
  tar -xf "linux-$KVER.tar.xz" || { echo "prepare_kernel_tree: tar failed on retry" >&2; exit 1; }
}
sentinels_ok || { echo "prepare_kernel_tree: tar extraction incomplete (sentinel missing)" >&2; exit 1; }
n_tar=$(tar -tf "linux-$KVER.tar.xz" | grep -v '/$' | wc -l)
n_tree=$(cd "$KDIR" && find . \( -type f -o -type l \) -not -path './.git/*' | wc -l)
if (( n_tree < n_tar )); then
  echo "prepare_kernel_tree: short tree ($n_tree of $n_tar files); re-extracting" >&2
  rm -rf "$KDIR"
  tar -xf "linux-$KVER.tar.xz" || { echo "prepare_kernel_tree: tar failed on re-extract" >&2; exit 1; }
  n_tree=$(cd "$KDIR" && find . \( -type f -o -type l \) -not -path './.git/*' | wc -l)
  (( n_tree >= n_tar )) || { echo "prepare_kernel_tree: still short after re-extract ($n_tree of $n_tar)" >&2; exit 1; }
fi
cd "$KDIR"

# ---- 2b. localversion files, exactly like prepare() --------------------------
# PKGBUILD: echo "-$pkgrel" > localversion.10-pkgrel;
#           echo "${pkgbase#linux}" > localversion.20-pkgname
printf -- '-%s\n' "$PKGREL" > localversion.10-pkgrel
printf -- '%s\n' "${LCCC_PKGBASE_SUFFIX:--cachymod}" > localversion.20-pkgname

# ---- 3. apply the CachyMod patch series (PKGBUILD source order) -------------
# Mirrors prepare() of the PKGBUILD, including its option filters.  Deriving the
# list from source=() (instead of a copy in this script) means the series can
# never drift from the package: a 6.18.5x bump that adds or reorders a patch is
# picked up automatically.
apply_patch_series() {
  local n=0 total=0 src patch_log
  patch_log=$(mktemp) || return 1
  while IFS= read -r src; do
    [[ -n $src ]] || continue
    total=$((total+1))
    case $src in
      *-prevent-avx2*)
        [[ $PREVENT_AVX2 =~ ^(yes|y|1)$ ]] || continue ;;
    esac
    case $src in
      *-prjc.patch)
        [[ $CPUSCHED =~ ^(bmq|pds)$ ]] || continue ;;
      *-prjc-s.patch)
        [[ $CPUSCHED =~ ^(bmq|pds)$ ]] || continue ;;
    esac
    [[ -f "$PKGDIR/$src" ]] || {
      echo "prepare_kernel_tree: PKGBUILD lists $src but $PKGDIR lacks it" >&2
      return 1
    }
    if patch -Np1 --silent --forward < "$PKGDIR/$src" >"$patch_log" 2>&1; then
      n=$((n+1))
    elif grep -q "previously applied" "$patch_log"; then
      n=$((n+1))
    else
      echo "prepare_kernel_tree: patch FAILED: $src" >&2
      tail -5 "$patch_log" >&2
      return 1
    fi
  done < <(pkgbuild_patch_sources)
  rm -f -- "$patch_log"
  echo "prepare_kernel_tree: applied $n patches (PKGBUILD source=() lists $total .patch entries)"
}
apply_patch_series || exit 1

# ---- 4. config + generated headers (LCCC for kernel TUs) --------------------
require_lccc
echo "prepare_kernel_tree: configuring (package config + olddefconfig + prepare with LCCC)"
cp "$PKGDIR/config" .config
# NOTE: stdout of make must stay visible — a silently-short `make prepare` once
# left arch/x86/include/generated/asm/rwonce.h missing and every boot compile
# broke.
lccc_make_prepare

# ---- 5. boot-code stubs a full Kbuild would generate ------------------------
# capflags.c (mkcapflags.sh over cpufeatures.h)
( cd arch/x86/kernel/cpu && sh mkcapflags.sh capflags.c \
    ../../include/asm/cpufeatures.h ../../include/asm/vmxfeatures.h )

# cpustr.h (arch/x86/boot/Makefile builds it with the mkcpustr host tool;
# realmode_corpus.sh needs it for cpu.c but does not drive Kbuild).  Host tool.
if [[ ! -f arch/x86/boot/cpustr.h ]]; then
  "$HOSTCC" -O2 -Iarch/x86/include -Iarch/x86/include/generated -Iinclude \
      arch/x86/boot/mkcpustr.c -o "$WORK/mkcpustr"
  "$WORK/mkcpustr" > arch/x86/boot/cpustr.h
fi

# utsversion.h (init/Makefile normally builds it from the build banner)
mkdir -p include/generated
printf '#define UTS_VERSION "#1 SMP PREEMPT_DYNAMIC"\n' > include/generated/utsversion.h

# zoffset.h / voffset.h (normally nm of compressed/vmlinux and vmlinux).
# header.o only consumes them as immediates, so code size is unaffected;
# efi32/efi64 entries must satisfy the CONFIG_EFI_MIXED .if in header.S.
cat > arch/x86/boot/zoffset.h <<'EOF'
/* STUB zoffset.h — normally generated from compressed/vmlinux (nm).
 * Values are immediates only; they do not affect setup.elf size. */
#define ZO_startup_32 0x1000
#define ZO_efi32_stub_entry 0x1000
#define ZO_efi64_stub_entry 0x1200
#define ZO_efi_pe_entry 0x1300
#define ZO_efi32_pe_entry 0x1400
#define ZO_input_data 0x200000
#define ZO_kernel_info 0x1500
#define ZO__end 0x800000
#define ZO__ehead 0x200
#define ZO__text 0x210
#define ZO__data 0x300
#define ZO__edata 0x800000
#define ZO__sbat 0x400
#define ZO__esbat 0x410
#define ZO_z_input_len 0x100000
#define ZO_z_output_len 0x400000
#define ZO_z_extract_offset 0x0
#define ZO_z_min_extract_offset 0x1000
#define ZO_z_extra_bytes 0x10000
#define ZO_INIT_SIZE 0x500000
EOF
cat > arch/x86/boot/voffset.h <<'EOF'
/* STUB voffset.h — normally generated from vmlinux (nm). Immediates only. */
#define VO__text 0x100000
#define VO__end  0x800000
EOF

# Verify every canary before stamping: a half-prepared tree must never pass.
for c in "${LCCC_PREPARED_CANARIES[@]}"; do
  [[ -f "$c" ]] || { echo "prepare_kernel_tree: post-check failed, missing: $c" >&2; exit 1; }
done
compiler_id > .lccc-prepared
echo "prepare_kernel_tree: $KDIR ready (stamp written: $(cat .lccc-prepared))"
