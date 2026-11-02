#!/usr/bin/env bash
set -euo pipefail
ROOT=$(CDPATH='' cd -- "$(dirname -- "$0")/../../.." && pwd)
# shellcheck source=tests/workloads/glibc-2.44/build_guards.sh
source "$ROOT/tests/workloads/glibc-2.44/build_guards.sh"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

make_compiler() {
    cat >"$1" <<'EOF'
#!/usr/bin/env bash
case ${1-} in
  -dumpmachine) echo x86_64-test-linux-gnu ;;
  --version) printf '%s\n' "${FAKE_VERSION-output}" ;;
  *) exit 0 ;;
esac
EOF
    chmod +x "$1"
}
mkdir -p "$tmp/toolchain"
make_compiler "$tmp/toolchain/gcc-99"
make_compiler "$tmp/toolchain/g++-99"

glibc_resolve_host_pair "$tmp/toolchain/gcc-99" ""
[[ $HOST_CC == "$tmp/toolchain/gcc-99" && $HOST_CXX == "$tmp/toolchain/g++-99" ]]
glibc_resolve_host_pair "" "$tmp/toolchain/g++-99"
[[ $HOST_CC == "$tmp/toolchain/gcc-99" && $HOST_CXX == "$tmp/toolchain/g++-99" ]]
if glibc_resolve_host_pair 'ccache gcc' "$tmp/toolchain/g++-99" 2>/dev/null; then
    echo 'FAIL: command-string HOST_CC was accepted' >&2; exit 1
fi

mkdir -p "$tmp/work/build-lccc" "$tmp/work/stage-lccc" "$tmp/work/glibc-deadbeef"
glibc_safe_remove_tree "$tmp/work" "$tmp/work/build-lccc"
[[ ! -e $tmp/work/build-lccc ]]
rm -rf "$tmp/work/stage-lccc"
mkdir -p "$tmp/outside" && ln -s "$tmp/outside" "$tmp/work/stage-lccc"
if glibc_safe_remove_tree "$tmp/work" "$tmp/work/stage-lccc" 2>/dev/null; then
    echo 'FAIL: symlink escape was accepted' >&2; exit 1
fi
[[ -d $tmp/outside ]]
if glibc_safe_remove_tree "$tmp/work" "$tmp/work/arbitrary" 2>/dev/null; then
    echo 'FAIL: unknown child was accepted' >&2; exit 1
fi

mkdir -p "$tmp/repo/sub"
git -C "$tmp/repo" init -q
git -C "$tmp/repo" -c user.name=test -c user.email=test@example.invalid \
    commit --allow-empty -qm first
printf patch >"$tmp/repo/sub/input.patch"
first=$(glibc_patch_git_provenance "$tmp/repo/sub/input.patch")
git -C "$tmp/repo" -c user.name=test -c user.email=test@example.invalid \
    commit --allow-empty -qm unrelated
second=$(glibc_patch_git_provenance "$tmp/repo/sub/input.patch")
[[ $first != "$second" ]] # provenance changes, input acceptance does not.

cat >"$tmp/empty-version" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
chmod +x "$tmp/empty-version"
[[ $(glibc_version_line "$tmp/empty-version") == '<empty version output:'* ]]
mkdir -p "$tmp/logs"
printf configure-failed >"$tmp/logs/configure.log"
if glibc_report_build_failure 7 "$tmp/logs" 2>"$tmp/report"; then
    echo 'FAIL: failure reporter lost status' >&2; exit 1
else
    [[ $? == 7 ]]
fi
grep -q 'status 7' "$tmp/report"
grep -q 'configure-failed' "$tmp/report"

echo 'glibc build guards: PASS'
