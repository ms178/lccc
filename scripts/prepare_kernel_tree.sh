#!/usr/bin/env bash
# ============================================================================
# prepare_kernel_tree.sh — regenerate the patched linux-cachymod tree
# (linux-$KVER, KVER derived from the package PKGBUILD)
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
#   LCCC_KERNEL_SOURCE_MODE  auto (default: verified kernel.org tarball, git
#            mirror fallback), tarball, or git
#   LCCC_KERNEL_GIT_URL      stable-tree mirror (default: github.com/gregkh/linux;
#            carries the signed v$KVER release tags)
# ============================================================================
set -euo pipefail

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REPO_ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
LCCC=${LCCC:-$REPO_ROOT/target/fastbuild/lccc}
LCCC_LD=${LCCC_LD:-$REPO_ROOT/target/fastbuild/lccc-ld}
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
# Mount-table queries (decoded \040-style escapes, fail-closed when unreadable).
# shellcheck source=mountinfo.sh
source "$(dirname -- "${BASH_SOURCE[0]}")/mountinfo.sh"
# mounts_under DIR: 0 if a mount point is DIR or lies below it (or unknown).
mounts_under() {
  # Mountinfo uses physical absolute names. Normalize only the query: changing
  # PKG_ROOT itself would break prefix checks for explicit relative PKGDIRs.
  local physical
  physical=$(realpath -m -- "$1") || return 0
  has_mount_under "$physical"
}
ensure_pkgdir() {
  [[ -d $PKGDIR ]] && return 0
  command -v git >/dev/null 2>&1 || return 1
  echo "prepare_kernel_tree: fetching package sources from $PKG_REPO"
  local rel=${PKGDIR#"$PKG_ROOT"/} branch
  [[ $rel != /* ]] || { echo "prepare_kernel_tree: PKGDIR must live under PKG_ROOT" >&2; return 1; }
  # `..`, `.` and empty components would name a path outside PKG_ROOT, and
  # `git sparse-checkout set` takes them verbatim.
  case "/$rel/" in
    */../* | */./* | *//*) echo "prepare_kernel_tree: PKGDIR=$PKGDIR has a '.', '..' or empty component" >&2; return 1 ;;
  esac
  # Clear PKG_ROOT only when it is provably ours: absent, empty, or a git
  # checkout of $PKG_REPO.  The directory itself is kept (its parent may be
  # owned by someone else), and anything else is refused, so a mistyped
  # PKG_ROOT such as $HOME is never wiped.
  if [[ -e $PKG_ROOT ]]; then
    if [[ -d $PKG_ROOT && -z $(ls -A -- "$PKG_ROOT") ]]; then
      :
    elif [[ -d $PKG_ROOT/.git ]] \
         && [[ $(git -C "$PKG_ROOT" remote get-url origin 2>/dev/null) == "$PKG_REPO" ]]; then
      if mounts_under "$PKG_ROOT"; then
        echo "prepare_kernel_tree: refusing to clear PKG_ROOT=$PKG_ROOT (a mount is below it)" >&2
        return 1
      fi
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
# A trailing slash makes `-L` look through a symlink, and `rm -rf "$KDIR/"` then
# empties the link's target. Normalise first so every guard below sees the path.
while [[ $KDIR == */ && $KDIR != / ]]; do KDIR=${KDIR%/}; done
# The script removes $KDIR before extracting into it; refuse anything that is not
# a linux-* source directory (a stray KERNEL_DIR=/ or $HOME must never be wiped).
case $(basename -- "$KDIR") in
  linux*) ;;
  *) echo "prepare_kernel_tree: refusing KDIR=$KDIR (basename must start with linux)" >&2; exit 1 ;;
esac
# A basename check is not enough: refuse the filesystem root, the home
# directory, symlinks, and any existing directory that is not a kernel tree or
# an empty directory (a stray `linux-*` user directory must never be removed).
if [[ $KDIR == / || $KDIR -ef $HOME || -L $KDIR ]]; then
  echo "prepare_kernel_tree: refusing KDIR=$KDIR (root, home or symlink)" >&2; exit 1
fi
# Source-acquisition mode.  Sandboxed environments (e.g. the Arena harness)
# often cannot reach cdn.kernel.org at all; the stable tree is mirrored on
# GitHub (github.com/gregkh/linux carries the signed v$KVER tags), so a shallow
# checkout of the exact release tag is the fallback source.  The PKGBUILD
# sha256 cannot verify a git checkout; instead the checkout is pinned by
# `git describe --exact-match --tags HEAD == v$KVER` (the release tag itself)
# and the resolved commit is recorded in the .lccc-prepared stamp.
#   auto    (default) tarball with sha256 verification when kernel.org is
#           reachable; shallow git checkout of tag v$KVER otherwise.
#   tarball force the verified-tarball path (offline git trees rejected).
#   git     force the git path (no kernel.org access attempted).
SOURCE_MODE=${LCCC_KERNEL_SOURCE_MODE:-auto}
# Tag-signature trust policy for the git fallback (LCCC_KERNEL_GIT_TAG_VERIFY):
#   strict  the v$KVER tag must carry a GOOD GPG signature (git verify-tag);
#           an unsigned or unverifiable tag aborts preparation.
#   auto    (default) verify-tag is attempted; a good signature is recorded,
#           a missing/unverifiable one is a LOUD warning plus a
#           gpg-verified=no line in the stamp, so downstream consumers can
#           decide.  Tag PRESENCE alone (exact-match) pins the version, not
#           the content.
#   off     skip verification (recorded as off).
LCCC_KERNEL_GIT_TAG_VERIFY=${LCCC_KERNEL_GIT_TAG_VERIFY:-auto}
GIT_TAG_VERIFY_RESULT=unknown
git_tree_at_tag() { # KDIR is a git checkout sitting exactly on tag v$KVER
  [[ -d $KDIR/.git ]] || return 1
  [[ $(git -C "$KDIR" describe --exact-match --tags HEAD 2>/dev/null) == "v$KVER" ]] || return 1
  case $LCCC_KERNEL_GIT_TAG_VERIFY in
    off)
      GIT_TAG_VERIFY_RESULT=off
      ;;
    strict)
      if git -C "$KDIR" verify-tag "v$KVER" >/dev/null 2>&1; then
        GIT_TAG_VERIFY_RESULT=yes
      else
        echo "prepare_kernel_tree: FATAL strict mode: tag v$KVER has no verifiable GPG signature" >&2
        return 1
      fi
      ;;
    *)
      if git -C "$KDIR" verify-tag "v$KVER" >/dev/null 2>&1; then
        GIT_TAG_VERIFY_RESULT=yes
      else
        GIT_TAG_VERIFY_RESULT=no
        echo "prepare_kernel_tree: WARNING tag v$KVER signature not verified (no trusted kernel keyring?); stamp records gpg-verified=no" >&2
      fi
      ;;
  esac
  return 0
}
if [[ -e $KDIR ]] && git_tree_at_tag; then
  SOURCE_MODE=git
  echo "prepare_kernel_tree: using existing git checkout at tag v$KVER"
else
  if [[ -e $KDIR ]]; then
    # Replacing KDIR deletes it. Only a directory this script stamped with
    # .lccc-prepared, or an empty one, may be replaced. A Makefile is not
    # ownership: a real kernel checkout has one, and it must survive.
    if [[ ! -d $KDIR ]] || { [[ -n $(ls -A -- "$KDIR") ]] && [[ ! -f $KDIR/.lccc-prepared ]]; }; then
      echo "prepare_kernel_tree: refusing KDIR=$KDIR (not empty and not prepared by this script;" >&2
      echo "  remove it yourself or choose another KDIR)" >&2
      exit 1
    fi
  fi
  # Extraction deletes KDIR. A mount point below it would make that deletion
  # reach the mounted filesystem, so refuse before anything is removed.
  if mounts_under "$KDIR"; then
    echo "prepare_kernel_tree: refusing KDIR=$KDIR (a mount point lies below it)" >&2; exit 1
  fi
fi
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

# Content identity of every input that determines patching/preparation. The
# selected configuration is an INPUT here; later caller changes to .config are
# managed by olddefconfig/syncconfig in the build harness, not by silently
# restoring the package config. Source-acquisition transport (auto/tarball/git)
# is deliberately NOT part of the recipe: the tarball sha256 pins content, and
# the resolved git commit is recorded separately in the stamp.
recipe_id() {
  {
    printf 'format=2\nversion=%s\npkgrel=%s\nsuffix=%s\ncpusched=%s\nprevent-avx2=%s\ntarball-sha256=%s\n' \
      "$KVER" "$PKGREL" "${LCCC_PKGBASE_SUFFIX:--cachymod}" "$CPUSCHED" "$PREVENT_AVX2" "$PKG_TARBALL_SHA256"
    printf 'PKGBUILD '; sha256_of "$PKGBUILD"
    printf 'config '; sha256_of "$PKGDIR/config"
    while IFS= read -r patch_name; do
      [[ -n $patch_name ]] || continue
      case $patch_name in
        *-prevent-avx2*) [[ $PREVENT_AVX2 =~ ^(yes|y|1)$ ]] || continue ;;
        *-prjc.patch|*-prjc-s.patch) [[ $CPUSCHED =~ ^(bmq|pds)$ ]] || continue ;;
      esac
      printf '%s ' "$patch_name"; sha256_of "$PKGDIR/$patch_name"
    done < <(pkgbuild_patch_sources)
  } | sha256_of /dev/stdin
}

# `make olddefconfig`/`make prepare` need host tools that are not part of the
# LCCC build and therefore easy to have absent on a fresh machine; failing
# there wastes the whole download+patch phase that precedes it.  Check first.
preflight_host_tools() {
  local missing=() tool comp cfgname
  # kconfig lexers/parsers, plus the unconditional build-time host tools.
  # `cpio` is deliberately absent: the kernel build never invokes it (an
  # embedded initramfs is produced by usr/gen_init_cpio from
  # CONFIG_INITRAMFS_SOURCE, and external initramfs images are built by the
  # caller — scripts/gen_initramfs.py covers that without a cpio binary).
  for tool in make "$HOSTCC" ld ar nm objcopy perl awk sed bc flex bison \
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
PREPARE_RECIPE=$(recipe_id)

# Already prepared?  Verify the stamp AND a canary file the snapshot truncation
# removed last time (setup.ld); a damaged tree must regenerate, not half-work.
# A tree stamped by another compiler gets only the LCCC `make prepare` rerun.
compiler_id() { # identity of the kernel compiler, recorded in the stamp
  printf 'compiler=%s version=%s sha256=%s\n' \
    "$LCCC" "$("$LCCC" --version 2>/dev/null | head -n 1)" "$(sha256_of "$LCCC")"
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
  # Use the same target assembler/linker as the full build, including Kconfig probes.
  make ARCH=x86_64 CC="$LCCC" LD="$LCCC_LD" AS="$LCCC" HOSTCC="$HOSTCC" olddefconfig >/dev/null
  make ARCH=x86_64 CC="$LCCC" LD="$LCCC_LD" AS="$LCCC" HOSTCC="$HOSTCC" prepare -j"$(jobs_count)"
}
# Restore boot-only generated headers after preparation or compiler invalidation.
# Never replace real Kbuild output that is already present.
prepare_boot_headers() {
  # capflags.c (mkcapflags.sh over cpufeatures.h)
  if [[ ! -f arch/x86/kernel/cpu/capflags.c ]]; then
    ( cd arch/x86/kernel/cpu && sh mkcapflags.sh capflags.c \
        ../../include/asm/cpufeatures.h ../../include/asm/vmxfeatures.h )
  fi

  # cpustr.h (arch/x86/boot/Makefile builds it with the mkcpustr host tool;
  # realmode_corpus.sh needs it for cpu.c but does not drive Kbuild).  Host tool.
  if [[ ! -f arch/x86/boot/cpustr.h ]]; then
    "$HOSTCC" -O2 -Iarch/x86/include -Iarch/x86/include/generated -Iinclude \
        arch/x86/boot/mkcpustr.c -o "$WORK/mkcpustr"
    "$WORK/mkcpustr" > arch/x86/boot/cpustr.h
  fi

  # utsversion.h (init/Makefile normally builds it from the build banner)
  mkdir -p include/generated
  [[ -f include/generated/utsversion.h ]] || printf '#define UTS_VERSION "#1 SMP PREEMPT_DYNAMIC"\n' > include/generated/utsversion.h

  # zoffset.h / voffset.h (normally nm of compressed/vmlinux and vmlinux).
  # header.o only consumes them as immediates, so code size is unaffected;
  # efi32/efi64 entries must satisfy the CONFIG_EFI_MIXED .if in header.S.
  if [[ ! -f arch/x86/boot/zoffset.h ]]; then
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
  fi
  if [[ ! -f arch/x86/boot/voffset.h ]]; then
    cat > arch/x86/boot/voffset.h <<'EOF'
/* STUB voffset.h — normally generated from vmlinux (nm). Immediates only. */
#define VO__text 0x100000
#define VO__end  0x800000
EOF
  fi
}

canaries_ok() {
  local c
  for c in "${LCCC_PREPARED_CANARIES[@]}"; do
    [[ -f "$KDIR/$c" ]] || { echo "prepare_kernel_tree: canary missing: $c" >&2; return 1; }
  done
}
if [[ -f "$KDIR/.lccc-prepared" ]] && [[ -f "$KDIR/${LCCC_PREPARED_CANARIES[0]}" ]] && canaries_ok; then
  require_lccc
  stamp_recipe=$(sed -n 's/^recipe-sha256=//p' "$KDIR/.lccc-prepared" | head -n1)
  if [[ $stamp_recipe != "$PREPARE_RECIPE" ]]; then
    if [[ ${LCCC_REPREPARE:-0} != 1 ]]; then
      echo "prepare_kernel_tree: recipe mismatch/legacy stamp in $KDIR; refusing to overwrite it" >&2
      echo "  expected recipe=$PREPARE_RECIPE; found=${stamp_recipe:-legacy}" >&2
      echo "  use a fresh KERNEL_DIR, or explicitly set LCCC_REPREPARE=1 to replace this owned tree" >&2
      exit 3
    fi
    echo "prepare_kernel_tree: recipe changed; LCCC_REPREPARE=1 replaces owned tree" >&2
  else
    if [[ $(head -n1 "$KDIR/.lccc-prepared") == "$(compiler_id)" ]]; then
      echo "prepare_kernel_tree: $KDIR already prepared (compiler + recipe + canaries OK)"
      exit 0
    fi
    echo "prepare_kernel_tree: compiler content changed; cleaning and re-running LCCC make prepare"
    # Kbuild does not fingerprint the compiler executable. A content change must
    # invalidate generated headers/asm-offsets, not just relink them.
    (cd "$KDIR" && make ARCH=x86_64 CC="$LCCC" LD="$LCCC_LD" AS="$LCCC" HOSTCC="$HOSTCC" clean >/dev/null)
    (cd "$KDIR"; lccc_make_prepare; prepare_boot_headers)
    canaries_ok
    # Preserve source/config/patch provenance while updating only compiler identity.
    stamp_tmp=$(mktemp "$KDIR/.lccc-prepared.tmp.XXXXXX")
    trap 'rm -f -- "$stamp_tmp"' EXIT
    { compiler_id; grep -v '^compiler=' "$KDIR/.lccc-prepared"; } > "$stamp_tmp"
    mv -f "$stamp_tmp" "$KDIR/.lccc-prepared"
    echo "prepare_kernel_tree: $KDIR re-prepared with LCCC"
    exit 0
  fi
fi

# A valid stamp is a strict no-op above. Reaching here with a stamp means
# missing canaries or an explicit reprepare: check mounts BEFORE the ownership
# decision so a mount guard, not a generic recipe error, protects an unsafe path.
if [[ -e $KDIR && -f $KDIR/.lccc-prepared ]]; then
  if mounts_under "$KDIR"; then
    echo "prepare_kernel_tree: refusing KDIR=$KDIR (a mount point lies below it)" >&2; exit 1
  fi
  if [[ ${LCCC_REPREPARE:-0} != 1 ]]; then
    echo "prepare_kernel_tree: refusing to replace prepared $KDIR without LCCC_REPREPARE=1" >&2
    exit 3
  fi
fi

mkdir -p "$WORK"
cd "$WORK"

# Shallow checkout of the exact release tag from the stable-tree git mirror.
# Only reachable when KDIR is empty, absent, or a replaceable stamped tree
# (guaranteed by the guard block above).  A failed clone leaves no tree behind.
git_checkout_tree() {
  local url=${LCCC_KERNEL_GIT_URL:-https://github.com/gregkh/linux}
  echo "prepare_kernel_tree: shallow git checkout of v$KVER from $url"
  rm -rf -- "$KDIR"
  git clone --quiet --depth 1 --branch "v$KVER" "$url" "$KDIR" || return 1
  git_tree_at_tag || {
    echo "prepare_kernel_tree: clone did not land on tag v$KVER" >&2
    return 1
  }
}

if [[ $SOURCE_MODE == git ]]; then
  [[ -d $KDIR ]] || git_checkout_tree || exit 1
  git_tree_at_tag || {
    echo "prepare_kernel_tree: KDIR vanished or is not at tag v$KVER" >&2
    exit 1
  }
  cd "$KDIR"
else

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
  curl_ok=1
  if ! curl -fsSL --retry 3 -o "$TARBALL" \
        "https://cdn.kernel.org/pub/linux/kernel/v${KVER%%.*}.x/linux-$KVER.tar.xz"; then
    curl_ok=0
  fi
  # Trust-boundary discipline: a DOWNLOADED archive whose sha256 does not
  # match the PKGBUILD is a corrupted or substituted artifact — auto mode
  # must NOT quietly degrade to the git fallback (an earlier revision did,
  # downgrading the integrity guarantee from PKGBUILD-sha256 to tag-presence).
  # Only an unreachable/curl-failed download falls back.
  if [[ $curl_ok == 1 ]] && [[ -f $TARBALL ]] && xz -t "$TARBALL" 2>/dev/null \
     && [[ $(sha256_of "$TARBALL") != "$PKG_TARBALL_SHA256" ]]; then
    echo "prepare_kernel_tree: FATAL tarball sha256 mismatch against PKGBUILD ($PKG_TARBALL_SHA256)" >&2
    exit 1
  fi
  if [[ $curl_ok == 0 ]] || ! tarball_ok; then
    if [[ $SOURCE_MODE == tarball ]]; then
      echo "prepare_kernel_tree: download failed sha256 check against PKGBUILD ($PKG_TARBALL_SHA256)" >&2
      exit 1
    fi
    # auto mode: kernel.org unreachable (sandboxed network) — fall back to the
    # signed-tag git mirror rather than dying after a wasted download attempt.
    echo "prepare_kernel_tree: kernel.org tarball unavailable; falling back to git mirror" >&2
    git_checkout_tree || exit 1
    SOURCE_MODE=git
    cd "$KDIR"
  fi
fi

# `auto` must COMMIT to the archive once it is verified.  The extraction block
# below is guarded by `SOURCE_MODE == tarball` (git mode has already produced
# its tree), so leaving the mode at `auto` after a successful verify/download
# skipped extraction entirely: KDIR was never created, the `cd "$KDIR"` inside
# that guard never ran, and the patch series executed in the *work* directory
# instead.  The observable failure was `patch` asking
# "File to patch: kernel/ksysfs.c" and the run dying 4 s after start — on a
# fresh machine, which is exactly the situation this script exists for (a
# prepared tree exits through the stamp fast-path long before this code).
# Verification already happened above (xz -t + the PKGBUILD sha256), so this
# is a mode decision, not a second integrity gate.
if [[ $SOURCE_MODE != git ]] && tarball_ok; then
  SOURCE_MODE=tarball
fi

# ---- 2. extract -------------------------------------------------------------
# tar always creates `linux-$KVER/`, whatever KDIR is named. Extract into a
# private staging directory and move that top directory to KDIR, so a custom
# KDIR works and no stray directory is ever entered or deleted by name.
# (git mode skips all of this: the checkout already IS the tree, git verified
# every object against the tag, and the canary checks before stamping cover
# worktree completeness.)
extract_tree() {
  local stage tar_err
  echo "prepare_kernel_tree: extracting"
  stage=$(mktemp -d "$WORK/.extract.XXXXXX") || return 1
  # tar may exit non-zero on benign "Directory renamed before its status could
  # be extracted" warnings (a GNU tar quirk when a directory's metadata changes
  # between tar's open and its later utime/chmod pass, seen under load on
  # FUSE/overlayfs). Real corruption is caught by the sentinel and count checks
  # below; tolerate the warning stream but verify the tree.
  tar_err=$(mktemp)
  tar -xf "$TARBALL" -C "$stage" 2>"$tar_err" || true
  grep -v "Directory renamed before its status could be extracted" "$tar_err" >&2 || true
  rm -f "$tar_err"
  # Only a complete extraction may replace KDIR. The old tree is removed after
  # the new one is staged, so a failed tar leaves the previous tree in place.
  if [[ ! -d $stage/linux-$KVER ]]; then
    rm -rf -- "$stage"
    echo "prepare_kernel_tree: extraction produced no linux-$KVER; KDIR left untouched" >&2
    return 1
  fi
  rm -rf -- "$KDIR"
  mv -- "$stage/linux-$KVER" "$KDIR"
  rm -rf -- "$stage"
}
if [[ $SOURCE_MODE == tarball ]]; then
extract_tree

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
  extract_tree
}
sentinels_ok || { echo "prepare_kernel_tree: tar extraction incomplete (sentinel missing)" >&2; exit 1; }
n_tar=$(tar -tf "$TARBALL" | grep -v '/$' | wc -l)
n_tree=$(cd "$KDIR" && find . \( -type f -o -type l \) -not -path './.git/*' | wc -l)
if (( n_tree < n_tar )); then
  echo "prepare_kernel_tree: short tree ($n_tree of $n_tar files); re-extracting" >&2
  extract_tree
  n_tree=$(cd "$KDIR" && find . \( -type f -o -type l \) -not -path './.git/*' | wc -l)
  (( n_tree >= n_tar )) || { echo "prepare_kernel_tree: still short after re-extract ($n_tree of $n_tar)" >&2; exit 1; }
fi
cd "$KDIR"
fi  # SOURCE_MODE == tarball (extract block)
fi  # SOURCE_MODE == git else-branch (tarball download + extract)

# ---- 2c. pristine base for patching -----------------------------------------
# A previously failed run can leave a half-patched tree. In git mode we can
# (and must) restore the pristine tag state first: re-patching a dirty tree
# silently double-applies hunks that patch's reverse-detection misses under
# fuzz, producing inconsistent sources that later fail in even more confusing
# ways. (tarball mode is inherently pristine: extraction always replaces KDIR.)
if [[ $SOURCE_MODE == git ]]; then
  git checkout -q -- .
  git clean -qfd
fi

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
    # The base is pristine (step 2c), so a plain forward apply is exact: no
    # --forward leniency, no "previously applied" ambiguity — any mismatch
    # fails loudly and the next run starts from the pristine tag again.
    if patch -p1 --silent < "$PKGDIR/$src" >"$patch_log" 2>&1; then
      n=$((n+1))
    else
      echo "prepare_kernel_tree: patch FAILED: $src" >&2
      tail -5 "$patch_log" >&2
      rm -f -- "$patch_log"
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

# ---- 5. boot-code headers -----------------------------------------------
prepare_boot_headers

# Verify every canary before stamping: a half-prepared tree must never pass.
for c in "${LCCC_PREPARED_CANARIES[@]}"; do
  [[ -f "$c" ]] || { echo "prepare_kernel_tree: post-check failed, missing: $c" >&2; exit 1; }
done
stamp_tmp=$(mktemp .lccc-prepared.tmp.XXXXXX)
trap 'rm -f -- "$stamp_tmp"' EXIT
{
  compiler_id
  printf 'format=2\nrecipe-sha256=%s\nconfig-input-sha256=%s\npkgrel=%s\ncpusched=%s\nprevent-avx2=%s\n' \
    "$PREPARE_RECIPE" "$(sha256_of "$PKGDIR/config")" "$PKGREL" "$CPUSCHED" "$PREVENT_AVX2"
  if [[ -d $PKGDIR/.git ]] || git -C "$PKGDIR" rev-parse --git-dir >/dev/null 2>&1; then
    printf 'package-commit=%s\n' "$(git -C "$PKGDIR" rev-parse HEAD 2>/dev/null || echo unknown)"
  fi
  printf 'source=%s\n' "$SOURCE_MODE"
  if [[ $SOURCE_MODE == git ]]; then
    printf 'commit=%s\n' "$(git rev-parse HEAD 2>/dev/null || echo unknown)"
    printf 'tag-verified=%s\n' "$(git describe --exact-match --tags HEAD 2>/dev/null || echo no)"
    printf 'gpg-verified=%s\n' "$GIT_TAG_VERIFY_RESULT"
  else
    printf 'tarball-sha256=%s\n' "$PKG_TARBALL_SHA256"
  fi
} > "$stamp_tmp"
mv -f "$stamp_tmp" .lccc-prepared
echo "prepare_kernel_tree: $KDIR ready (stamp written: $(cat .lccc-prepared))"
