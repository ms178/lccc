#!/usr/bin/env bash
# ensure_gas_247.sh — (re)provision the pinned binutils 2.47 oracle pair
# (GNU as + objdump) for a cross target after a harness wipe.
#
# The differential execution suites hard-gate on GNU as 2.47
# (latest-toolchains-only policy), and the encdiff corpus gate additionally
# pins the DISASSEMBLER: objdump decides BEATS/ok verdicts through
# decodes_same, so the runner image's objdump is as much an oracle as its
# `as' — whatever binutils the host ships is not a verdict authority. Both
# binaries come from the same 2.47 build, land under the same prefix, and
# are required TOGETHER: an as-only cache (a provision interrupted after
# the gas cp) must not short-circuit the gate into running with an
# unpinned objdump. Distro binutils is older, and the locally built tools
# live under the snapshot-excluded .cache/ tree, so they never survive a
# workspace restore. This script rebuilds them idempotently from the
# upstream tarball; only the binutils tools needed are configured and
# built (no ld/gold/gdb/sim), which keeps the build at a few minutes on
# the 2-vCPU sandbox.
#
# ftp.gnu.org is NOT universally reachable from the sandbox (connection
# blackholed), so the tarball fetch walks a mirror chain and takes the
# first one that answers. Whatever the source — a fresh download OR a
# tarball pre-placed through the GAS_DL_DIR cache — the bytes must hash to
# the pinned SHA-256 below (cross-verified between ftp.gnu.org's mirror
# chain: kernel.org and uwaterloo serve identical bytes; GNU release
# tarballs are immutable). A mirror compromise, a truncated fetch and a
# doctored pre-placed tarball are the same defect: a binutils that is not
# the one the pins promise. The download cache and build tree are
# parametrised (GAS_DL_DIR / GAS_CACHE) so a persistent workspace can keep
# the tarball across wipes; the install prefix is arg 2.
#
# Usage: scripts/ensure_gas_247.sh [target-triple] [install-prefix]
#   target-triple defaults to riscv64-linux-gnu. The tools are installed
#   as <prefix>/bin/as and <prefix>/bin/objdump, and their versions are
#   printed on success — after the freshly installed pair has passed the
#   same validation the cache fast path applies.
#
# scripts/ensure_gas_247.sh --self-test
#   runs the validation matrix against fake tool pairs (correct 2.47,
#   wrong 2.46, substring lookalikes, mismatched tokens, unreadable
#   --version, version-correct-but-functionally-broken) and exits 0 only
#   if every case lands on the verdict it must.
set -euo pipefail

# ─── Pair validation ───────────────────────────────────────────────────────
# The version contract is an ANCHORED token, not a substring: grepping
# for "2.47" accepts 2.470, 12.47 and wrapper-2.47-malicious alike. The
# token is extracted from --version's first line (always the last
# whitespace-delimited field a GNU tool prints), matched anchored against
# the pin, and required to be the SAME token from both halves of the
# pair: two different builds both calling themselves 2.47 is not "one
# 2.47 build", which is what the pair contract promises. The grammar
# accepts dated snapshot tokens (2.47.20260726) BY DESIGN: the pinned
# RELEASE tarball's own build stamps a dated snapshot token into
# --version, so requiring the bare "2.47" would reject the genuine
# product of the pinned bytes. The grammar's job is lookalike and
# pair-mismatch rejection, not snapshot discrimination — the byte-level
# binding is the provenance marker's job (layer 3 below). A functional
# canary then proves the pair actually assembles and disassembles in both
# modes the gates run — a version string is not a tool. Non-x86 targets
# have no shared canary source and validate version-only.

_version_token() {  # <tool>: the version token of --version's first line
    local line
    line=$("$1" --version 2>/dev/null | sed -n '1,1p') || return 1
    [[ -n "$line" ]] || return 1
    printf '%s\n' "$line" | awk '{print $NF}'
}

_canary() {  # functional target verification; x86-family targets only
    case "$target" in
        x86_64-*|i686-*|i386-*) ;;
        *) return 0 ;;
    esac
    local tmp tag listing ok=0
    tmp=$(mktemp -d)
    printf '.text\nmov %%eax,%%ebx\n' >"$tmp/canary.s"
    for tag in 64 32; do
        if ! "$as" "--$tag" -o "$tmp/canary$tag.o" "$tmp/canary.s" \
                2>/dev/null; then
            echo "validate: canary: $as failed to assemble 'mov %eax,%ebx' in --$tag mode" >&2
            ok=1
            break
        fi
        listing=$("$od" -d "$tmp/canary$tag.o" 2>/dev/null) || {
            echo "validate: canary: $od failed to disassemble the --$tag object" >&2
            ok=1
            break
        }
        if ! grep -Eq 'mov[[:space:]]+%eax,%ebx' <<<"$listing"; then
            echo "validate: canary: $od did not decode the --$tag canary back to 'mov %eax,%ebx'" >&2
            ok=1
            break
        fi
    done
    rm -rf "$tmp"
    return "$ok"
}

validate_pair() {
    local pin_re='^2\.47(\.[0-9]+)*$'
    local tok_as tok_od
    tok_as=$(_version_token "$as") \
        || { echo "validate: $as --version is unreadable" >&2; return 1; }
    tok_od=$(_version_token "$od") \
        || { echo "validate: $od --version is unreadable" >&2; return 1; }
    [[ "$tok_as" =~ $pin_re ]] || {
        echo "validate: $as reports version '$tok_as' — not the pinned 2.47 (the token is matched against an anchored pattern: 2.470, 12.47, 2.46 and lookalikes are rejected)" >&2
        return 1
    }
    [[ "$tok_od" =~ $pin_re ]] || {
        echo "validate: $od reports version '$tok_od' — not the pinned 2.47 (anchored token)" >&2
        return 1
    }
    [[ "$tok_as" == "$tok_od" ]] || {
        echo "validate: pair mismatch — as says '$tok_as', objdump says '$tok_od'; both version tokens must be identical" >&2
        return 1
    }
    _canary || return 1
}

# ─── Supply-chain pin: the tarball's bytes ────────────────────────────────
# Checked on EVERY trust path — freshly fetched from any mirror AND
# already present in the download cache (GAS_DL_DIR is exactly the channel
# through which a pre-placed tarball would otherwise skip verification).
# sha256sum output is one <hash>  <file> line; the comparison is on the
# whole 64-hex-digit token, never a substring of it.
GAS_TARBALL_SHA256=154ab23b60070e8f27013c22977f1129425d67d1e8acd6e13010e617811e4cff

_sha256_is() {  # _sha256_is <file> <expected-hex>: whole-digest equality
    [[ -f "$1" ]] || return 1
    local got
    got=$(sha256sum "$1" 2>/dev/null) || return 1
    got=${got%% *}
    [[ "$got" == "$2" ]]
}

_tarball_matches_pin() {  # <tarball>: the pinned binutils-2.47 bytes?
    _sha256_is "$1" "$GAS_TARBALL_SHA256"
}

# ─── Provenance: tie the installed pair to the pinned tarball's bytes ────
# Layer 3 of the pair validation. Layers 1+2 alone (anchored version
# token + functional canary) accepted ANY pair under the prefix that
# self-reported a 2.47.* token — a stale pair from an older pin, a pair
# installed by a different tool, a partially-overwritten prefix: all of
# them bypassed the tarball pin entirely, which is exactly the gap
# between "the tarball BYTES are pinned" and "the installed PAIR came
# from those bytes". The marker written after every verified-tarball
# build closes it:
#
#   target=<the configured triple>        must equal this invocation's
#   tarball_version=2.47                  the pin, for the record
#   tarball_sha256=<GAS_TARBALL_SHA256>   must equal the CURRENT pin — a
#                                          pin rotation retires every
#                                          existing cache automatically
#   as_sha256/objdump_sha256=<digests>    must equal the CURRENT bytes of
#                                          <prefix>/bin/{as,objdump} — a
#                                          swapped, tampered, truncated or
#                                          partially-overwritten pair is
#                                          rejected even when it still
#                                          self-reports 2.47.* and passes
#                                          the canary
#
# The key set is exact (no unknown keys, no duplicates): a marker from a
# different tool — or a hand-edited one — fails the parse and the pair
# rebuilds from the verified tarball. Honest boundary, same as the
# tarball pin: a local attacker who can write the binaries can rewrite
# the marker too; the runner filesystem is the trusted side, the network
# and the caches are not. What the marker DOES close is every accidental
# and environmental drift case that previously rode the fast path on a
# self-reported version string alone.
PROVENANCE_KEYS=(target tarball_version tarball_sha256 as_sha256 objdump_sha256)

_pair_provenance_ok() {  # requires $as/$od/$provenance/$target set
    [[ -f "$provenance" ]] || return 1
    local line key value
    local -A seen=()
    # `read` returns 1 at EOF even when it DID deliver a final line
    # without a trailing newline; the `|| [[ -n $line ]]` guard processes
    # that last partial line instead of silently dropping it (a marker
    # whose last key was dropped would parse as a missing-key marker).
    while IFS= read -r line || [[ -n "$line" ]]; do
        [[ -n "$line" ]] || continue
        [[ "$line" == *"="* ]] || return 1
        key=${line%%=*}
        value=${line#*=}
        [[ -z "${seen[$key]+x}" ]] || return 1   # duplicate key
        seen[$key]=$value
    done < "$provenance"
    [[ ${#seen[@]} -eq ${#PROVENANCE_KEYS[@]} ]] || return 1
    local k
    for k in "${PROVENANCE_KEYS[@]}"; do
        [[ -n "${seen[$k]+x}" ]] || return 1
    done
    [[ "${seen[target]}" == "$target" ]] || return 1
    [[ "${seen[tarball_sha256]}" == "$GAS_TARBALL_SHA256" ]] || return 1
    _sha256_is "$as" "${seen[as_sha256]}" || return 1
    _sha256_is "$od" "${seen[objdump_sha256]}" || return 1
}

_write_provenance() {  # after a verified-tarball build passed validation
    local as_sum od_sum
    as_sum=$(sha256sum "$as") && as_sum=${as_sum%% *}
    od_sum=$(sha256sum "$od") && od_sum=${od_sum%% *}
    printf 'target=%s\ntarball_version=%s\ntarball_sha256=%s\nas_sha256=%s\nobjdump_sha256=%s\n' \
        "$target" "$ver" "$GAS_TARBALL_SHA256" "$as_sum" "$od_sum" \
        > "$provenance.tmp"
    mv -f "$provenance.tmp" "$provenance"
}

# ─── Self-test: the validation matrix, against fake tool pairs ────────────
# Every case the validators can be fed, each pinned to the verdict it must
# land on. The fakes are functional for the canary layer wherever the case
# is meant to reject at the VERSION layer (and vice versa), so each layer
# is proven to reject on its own, independently.
self_test() {
    local tmp ok=0
    tmp=$(mktemp -d)
    trap 'rm -rf "$tmp"' RETURN

    local V47="GNU assembler (GNU Binutils) 2.47"
    local O47="GNU objdump (GNU Binutils) 2.47"

    mkver() {  # mkver <path> <version-line>: prints the version, succeeds otherwise
        cat >"$1" <<EOF
#!/bin/sh
case "\$1" in --version) printf '%s\n' "$2"; exit 0;; esac
exit 0
EOF
        chmod +x "$1"
    }
    mkok() {  # mkok <path> <version-line>: as above, and -d decodes the canary
        cat >"$1" <<EOF
#!/bin/sh
case "\$1" in --version) printf '%s\n' "$2"; exit 0;; esac
printf '%s\n' '   0: 89 d8                  mov    %eax,%ebx'
exit 0
EOF
        chmod +x "$1"
    }
    mkbad() {  # mkbad <path> <version-line>: as above, but -d decodes something else
        cat >"$1" <<EOF
#!/bin/sh
case "\$1" in --version) printf '%s\n' "$2"; exit 0;; esac
printf '%s\n' '   0: 0f 1f 40 00             nopl   0x0(%rax,%rax,1)'
exit 0
EOF
        chmod +x "$1"
    }
    mkfail() {  # mkfail <path>: --version exits 1
        cat >"$1" <<'EOF'
#!/bin/sh
case "$1" in --version) exit 1;; esac
exit 0
EOF
        chmod +x "$1"
    }
    mkempty() {  # mkempty <path>: --version prints nothing
        cat >"$1" <<'EOF'
#!/bin/sh
case "$1" in --version) :;; esac
exit 0
EOF
        chmod +x "$1"
    }
    mkref() {  # mkref <path>: --version fine, refuses everything else
        cat >"$1" <<'EOF'
#!/bin/sh
case "$1" in --version) printf '%s\n' "GNU assembler (GNU Binutils) 2.47"; exit 0;; esac
exit 1
EOF
        chmod +x "$1"
    }

    check() {  # check <label> <want: 0=accept, 1=reject>
        local label=$1 want=$2 got
        if validate_pair >/dev/null 2>"$tmp/err"; then got=0; else got=1; fi
        if [[ $got == "$want" ]]; then
            printf '  ok   %-44s -> %s\n' "$label" \
                "$([[ $want == 0 ]] && echo ACCEPT || echo REJECT)"
        else
            printf '  FAIL %-44s -> wanted %s, got %s: %s\n' "$label" \
                "$([[ $want == 0 ]] && echo ACCEPT || echo REJECT)" \
                "$([[ $got == 0 ]] && echo ACCEPT || echo REJECT)" \
                "$(sed -n '1,1p' "$tmp/err")"
            ok=1
        fi
    }

    target=x86_64-linux-gnu

    as="$tmp/gas-247";  od="$tmp/od-247"
    mkok "$as" "$V47"; mkok "$od" "$O47"
    check "correct 2.47 pair (functional canary)" 0

    as="$tmp/gas-246"; od="$tmp/od-246"
    mkok "$as" "GNU assembler (GNU Binutils) 2.46"
    mkok "$od" "GNU objdump (GNU Binutils) 2.46"
    check "2.46 pair under the 2.47-named prefix" 1

    as="$tmp/gas-2470"; od="$tmp/od-2470"
    mkok "$as" "GNU assembler (GNU Binutils) 2.470"
    mkok "$od" "GNU objdump (GNU Binutils) 2.470"
    check "substring lookalike 2.470" 1

    as="$tmp/gas-1247"; od="$tmp/od-1247"
    mkok "$as" "GNU assembler (GNU Binutils) 12.47"
    mkok "$od" "GNU objdump (GNU Binutils) 12.47"
    check "substring lookalike 12.47" 1

    as="$tmp/gas-wrap"; od="$tmp/od-wrap"
    mkok "$as" "GNU assembler (GNU Binutils) wrapper-2.47-malicious"
    mkok "$od" "GNU objdump (GNU Binutils) wrapper-2.47-malicious"
    check "substring lookalike wrapper-2.47-malicious" 1

    as="$tmp/gas-247"; od="$tmp/od-247d"
    mkok "$as" "$V47"
    mkok "$od" "GNU objdump (GNU Binutils) 2.47.20260726"
    check "mismatched tokens (2.47 vs 2.47.20260726)" 1

    as="$tmp/gas-snap"; od="$tmp/od-snap"
    mkok "$as" "GNU assembler (GNU Binutils) 2.47.20260726"
    mkok "$od" "GNU objdump (GNU Binutils) 2.47.20260726"
    check "dated snapshot 2.47.20260726, both halves" 0

    as="$tmp/gas-fail"; od="$tmp/od-fail"
    mkfail "$as"; mkfail "$od"
    check "failing --version" 1

    as="$tmp/gas-empty"; od="$tmp/od-empty"
    mkempty "$as"; mkempty "$od"
    check "empty --version line" 1

    as="$tmp/gas-247"; od="$tmp/od-broken"
    mkok "$as" "$V47"
    mkbad "$od" "$O47"
    check "version-correct, canary-broken pair" 1

    as="$tmp/gas-ref"; od="$tmp/od-247"
    mkref "$as"; mkok "$od" "$O47"
    check "version-correct, as refuses to assemble" 1

    target=riscv64-linux-gnu
    as="$tmp/gas-rv"; od="$tmp/od-rv"
    mkver "$as" "$V47"; mkver "$od" "$O47"
    check "non-x86 target: version-only validation" 0

    # The supply-chain comparator, on real digests of a real file: the
    # whole token decides (prefix/substring lookalikes of the hash itself
    # are the same defect class as the version-token lookalikes above).
    printf 'binutils tarball bytes\n' >"$tmp/fake.tarball"
    local sum lookalike
    sum=$(sha256sum "$tmp/fake.tarball")
    sum=${sum%% *}
    # A 62-of-64-hex lookalike, constructed so it is GUARANTEED different
    # from the real digest whatever the content hashes to: flipping the
    # first two hex digits to a value they provably do not have (a
    # `${sum%??}ff` splice can reproduce the original verbatim when the
    # digest happens to end in "ff" — a lookalike case that is secretly
    # the exact digest is a vacuous test, the defect class this matrix
    # exists to hunt).
    case ${sum:0:2} in
        00) lookalike="11${sum:2}" ;;
        *)  lookalike="00${sum:2}" ;;
    esac
    if _sha256_is "$tmp/fake.tarball" "$sum"; then
        printf '  ok   %-44s -> %s\n' "sha256 pin: matching digest" ACCEPT
    else
        printf '  FAIL %-44s -> wanted ACCEPT: digest comparator\n' "sha256 pin: matching digest"
        ok=1
    fi
    if _sha256_is "$tmp/fake.tarball" "$lookalike"; then
        printf '  FAIL %-44s -> wanted REJECT: lookalike digest\n' "sha256 pin: 62-of-64-hex lookalike"
        ok=1
    else
        printf '  ok   %-44s -> %s\n' "sha256 pin: 62-of-64-hex lookalike" REJECT
    fi
    if _sha256_is "$tmp/absent.tarball" "$sum"; then
        printf '  FAIL %-44s -> wanted REJECT: missing file\n' "sha256 pin: missing tarball"
        ok=1
    else
        printf '  ok   %-44s -> %s\n' "sha256 pin: missing tarball" REJECT
    fi

    # The provenance layer, against a fake installed pair and fabricated
    # markers — every drift case the fast path must refuse (a pair that
    # still passes layers 1+2: it self-reports 2.47 and decodes the
    # canary, but is NOT the pair the pinned tarball produced).
    local marker sum_as sum_od
    marker="$tmp/provenance"
    provenance="$marker"
    ver=2.47
    target=x86_64-linux-gnu
    as="$tmp/gas-prov"; od="$tmp/od-prov"
    mkok "$as" "$V47"; mkok "$od" "$O47"
    sum_as=$(sha256sum "$as"); sum_as=${sum_as%% *}
    sum_od=$(sha256sum "$od"); sum_od=${sum_od%% *}
    prov_case() {  # prov_case <label> <want> <marker-text>
        local label=$1 want=$2 got
        printf '%s' "$3" >"$marker"
        if _pair_provenance_ok >/dev/null 2>&1; then got=0; else got=1; fi
        if [[ $got == "$want" ]]; then
            printf '  ok   %-44s -> %s\n' "$label" \
                "$([[ $want == 0 ]] && echo ACCEPT || echo REJECT)"
        else
            printf '  FAIL %-44s -> wanted %s, got %s\n' "$label" \
                "$([[ $want == 0 ]] && echo ACCEPT || echo REJECT)" \
                "$([[ $got == 0 ]] && echo ACCEPT || echo REJECT)"
            ok=1
        fi
    }
    local good_marker no_objdump_marker rot_pin
    good_marker=$(printf 'target=%s\ntarball_version=%s\ntarball_sha256=%s\nas_sha256=%s\nobjdump_sha256=%s\n' \
        "$target" "$ver" "$GAS_TARBALL_SHA256" "$sum_as" "$sum_od")
    no_objdump_marker=$(printf 'target=%s\ntarball_version=%s\ntarball_sha256=%s\nas_sha256=%s\n' \
        "$target" "$ver" "$GAS_TARBALL_SHA256" "$sum_as")
    # A rotated pin, guaranteed different from the current one (same
    # first-two-hex-digit flip as the lookalike digest above — a splice
    # off the tail can reproduce the original when the pin ends in the
    # spliced characters, which the current pin's "...e4cff" tail does).
    case ${GAS_TARBALL_SHA256:0:2} in
        00) rot_pin="11${GAS_TARBALL_SHA256:2}" ;;
        *)  rot_pin="00${GAS_TARBALL_SHA256:2}" ;;
    esac
    prov_case "provenance: marker of these exact bytes" 0 "$good_marker"
    # A pair installed before the marker existed (the current real-world
    # cache): no marker at all — the fast path must rebuild, not trust the
    # bare version+canary layers.
    rm -f "$marker"
    if _pair_provenance_ok >/dev/null 2>&1; then
        printf '  FAIL %-44s -> wanted REJECT: no marker file\n' "provenance: marker missing (pre-marker pair)"
        ok=1
    else
        printf '  ok   %-44s -> %s\n' "provenance: marker missing (pre-marker pair)" REJECT
    fi
    prov_case "provenance: empty marker file" 1 ""
    prov_case "provenance: wrong target (prefix reuse)" 1 \
        "${good_marker/x86_64-linux-gnu/riscv64-linux-gnu}"
    prov_case "provenance: wrong tarball pin (rotated pin)" 1 \
        "${good_marker/$GAS_TARBALL_SHA256/$rot_pin}"
    prov_case "provenance: as bytes drifted (swapped pair)" 1 \
        "${good_marker/$sum_as/$sum_od}"
    prov_case "provenance: objdump bytes drifted (tamper)" 1 \
        "${good_marker/$sum_od/$sum_as}"
    prov_case "provenance: missing key" 1 "$no_objdump_marker"
    prov_case "provenance: unknown extra key" 1 \
        "$good_marker
extra=1
"
    prov_case "provenance: duplicate key" 1 \
        "$good_marker
target=other-target
"
    prov_case "provenance: not key=value at all" 1 "garbage"
    rm -f "$marker"

    if [[ $ok == 0 ]]; then
        echo "ensure_gas_247 self-test: every case lands on its verdict"
    else
        echo "ensure_gas_247 self-test: FAILURES above" >&2
    fi
    return "$ok"
}

if [[ ${1:-} == --self-test ]]; then
    self_test
    exit $?
fi

target=${1:-riscv64-linux-gnu}
prefix=${2:-${HOME}/.cache/gas-2.47-${target}}
as="$prefix/bin/as"
od="$prefix/bin/objdump"
provenance="$prefix/.lccc-binutils-provenance"

# BOTH binaries, validated as a PAIR (pinned token, one build, functional
# canary) AND provably the product of the PINNED TARBALL's bytes (the
# provenance marker: recorded tarball digest equal to the current pin and
# recorded binary digests equal to the installed bytes), or a full
# rebuild. Three guards, three failure modes: a cache carrying only one
# of the pair is an interrupted provision; a pair under the 2.47-NAMED
# prefix that does not validate is a lie — the prefix name is not the
# version; and a pair whose provenance marker is missing, stale (a pin
# rotation) or describes different bytes (a swap, a tamper, a partial
# overwrite) never rode the pinned tarball at all — it rebuilds. Anything
# else falls through to the rebuild, which reinstalls from one verified
# source tree, must itself pass the same validation, and writes a fresh
# marker.
if [[ -x "$as" && -x "$od" ]] && validate_pair && _pair_provenance_ok; then
    "$as" --version | sed -n '1,1p'
    "$od" --version | sed -n '1,1p'
    exit 0
fi

ver=2.47
dl_dir=${GAS_DL_DIR:-${HOME}/dl}
cache=${GAS_CACHE:-${HOME}/.cache}
tarball="$dl_dir/binutils-$ver.tar.xz"
mkdir -p "$dl_dir" "$cache"

# Provisioning is a READ-MODIFY-WRITE over two shared paths: the download
# cache (the tarball and its `.part` staging name) and the extracted source
# tree (`$cache/binutils-2.47` and its `.extracting` scratch directory).
# The per-target build directory is private, but those two are not, so two
# invocations under one workspace contend: one is told `Directory not empty`
# while it removes the other's `.extracting` tree, and the loser's tar finds
# the winner's files already in place ("binutils-2.47/COPYING: Cannot open:
# File exists").  Measured 2026-10-05 while provisioning the x86_64 pair and
# the aarch64 triple side by side; both then failed and had to be re-run.
#
# Serialize the whole read-modify-write with an advisory lock on the shared
# cache, and RE-CHECK the fast path after acquiring it: a second caller that
# waited for the first to finish must find a valid pair and return, not
# rebuild it.  `flock` is util-linux and always present; if it is missing the
# provision still works, just unserialized (the guard below is a warning, not
# a failure, because a single-threaded CI step is the common case).
lock="$dl_dir/.ensure_gas_247.lock"
if command -v flock >/dev/null 2>&1; then
    exec 9>"$lock"
    flock 9
fi

# Double-checked under the lock: the pair may have been installed while this
# invocation waited for it.
if [[ -x "$as" && -x "$od" ]] && validate_pair && _pair_provenance_ok; then
    "$as" --version | sed -n '1,1p'
    "$od" --version | sed -n '1,1p'
    exit 0
fi

# Mirror chain: ftp.gnu.org first (canonical), then well-known mirrors that
# answer from the sandbox network. --connect-timeout keeps a blackholed
# host from stalling the provision.
if [[ ! -f "$tarball" ]]; then
    fetched=""
    for base in \
        "https://ftp.gnu.org/gnu/binutils" \
        "https://mirrors.kernel.org/gnu/binutils" \
        "https://mirror.csclub.uwaterloo.ca/gnu/binutils"; do
        echo "fetch: trying $base/binutils-$ver.tar.xz" >&2
        if curl --connect-timeout 10 --max-time 300 -sSLo "$tarball.part" \
            "$base/binutils-$ver.tar.xz"; then
            mv -f "$tarball.part" "$tarball"
            fetched=1
            break
        fi
        rm -f "$tarball.part"
    done
    [[ -n "$fetched" ]] || { echo "FATAL: no mirror reachable for binutils-$ver" >&2; exit 1; }
fi

# The supply-chain pin, on every trust path: a freshly fetched tarball, a
# cached one, or one pre-placed through GAS_DL_DIR must ALL hash to the
# pinned digest before a single byte of it is extracted. A mismatch is
# fatal, and the bad copy is deleted so the next run re-fetches instead of
# wedging on the same poisoned cache.
if ! _tarball_matches_pin "$tarball"; then
    rm -f "$tarball" "$tarball.part"
    echo "FATAL: $tarball is not the pinned binutils-$ver tarball (sha256 " \
         "$GAS_TARBALL_SHA256) — removed; a mirror or the GAS_DL_DIR cache " \
         "served bytes we did not order" >&2
    exit 1
fi

src="$cache/binutils-$ver"
build="$src-build-${target//-/_}"
# A bare -d test is not enough: an interrupted extraction (harness wipe mid
# tar, full disk, ...) leaves a tree that passes -d but has no configure,
# which would wedge the gate until manual cleanup. Reuse the cached tree only
# when its configure script is intact; otherwise re-extract atomically
# (extract to a scratch dir, then rename into place) so no partially
# extracted tree is ever visible to the build.
if [[ ! -f "$src/configure" ]]; then
    rm -rf "$src" "$src.extracting"
    mkdir -p "$src.extracting"
    tar -xJf "$tarball" -C "$src.extracting"
    mv "$src.extracting/binutils-$ver" "$src"
    rm -rf "$src.extracting"
fi
rm -rf "$build"
mkdir -p "$build"
cd "$build"
"$src/configure" --target="$target" --prefix="$prefix" \
    --disable-gdb --disable-sim --disable-gprofng --disable-nls \
    --disable-werror --disable-ld --disable-gold >configure.log 2>&1
make -j2 >make.log 2>&1
mkdir -p "$prefix/bin"
cp gas/as-new "$as"
cp binutils/objdump "$od"
# objcopy is the third half of the AArch64 matrix (read .text). Same
# 2.47 build, same prefix.
if [[ -x binutils/objcopy ]]; then
    cp binutils/objcopy "$prefix/bin/objcopy"
fi
# The freshly installed pair must pass the SAME validation the cache
# fast path applies — an install that cannot justify itself is a failure,
# not a print-and-hope.
validate_pair || { echo "FATAL: the freshly installed pair fails validation" >&2; exit 1; }
# The marker is part of the install, not an afterthought: without it the
# NEXT run cannot prove this pair came from the pinned tarball and will
# rebuild it. Written atomically (temp + rename) so an interrupted write
# can never leave a half-marker that parses.
_write_provenance
"$as" --version | sed -n '1,1p'
"$od" --version | sed -n '1,1p'
