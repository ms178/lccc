#!/usr/bin/env bash
# Sourceable safety helpers for build_lccc.sh. No work is performed at import.

_glibc_find_sibling() { # compiler-path sibling-basename
    local original=$1 sibling=$2 directory candidate
    directory=$(dirname -- "$original")
    candidate=$directory/$sibling
    if [[ -x $candidate ]]; then
        printf '%s\n' "$candidate"
    else
        command -v -- "$sibling" || return 1
    fi
}

glibc_resolve_host_pair() { # [cc] [cxx]; sets HOST_CC/HOST_CXX
    local cc=${1-} cxx=${2-} base sibling
    if [[ -z $cc && -z $cxx ]]; then
        cc=$(command -v gcc || true)
        cxx=$(command -v g++ || true)
    elif [[ -n $cc && -z $cxx ]]; then
        base=$(basename -- "$cc")
        case $base in
            gcc) sibling=g++ ;;
            gcc-[0-9]*) sibling=g++-${base#gcc-} ;;
            clang) sibling=clang++ ;;
            clang-[0-9]*) sibling=clang++-${base#clang-} ;;
            cc) sibling=c++ ;;
            *)
                echo "error: cannot derive HOST_CXX from HOST_CC=$cc; set both bare executable paths" >&2
                return 2 ;;
        esac
        cxx=$(_glibc_find_sibling "$cc" "$sibling") || {
            echo "error: companion C++ compiler '$sibling' not found for HOST_CC=$cc" >&2
            return 2
        }
    elif [[ -z $cc ]]; then
        base=$(basename -- "$cxx")
        case $base in
            g++) sibling=gcc ;;
            g++-[0-9]*) sibling=gcc-${base#g++-} ;;
            clang++) sibling=clang ;;
            clang++-[0-9]*) sibling=clang-${base#clang++-} ;;
            c++) sibling=cc ;;
            *)
                echo "error: cannot derive HOST_CC from HOST_CXX=$cxx; set both bare executable paths" >&2
                return 2 ;;
        esac
        cc=$(_glibc_find_sibling "$cxx" "$sibling") || {
            echo "error: companion C compiler '$sibling' not found for HOST_CXX=$cxx" >&2
            return 2
        }
    fi

    for spec in "HOST_CC:$cc" "HOST_CXX:$cxx"; do
        local label=${spec%%:*} path=${spec#*:}
        [[ $path != *[[:space:]]* ]] || {
            echo "error: $label must be one bare executable path, not a command string: $path" >&2
            return 2
        }
        [[ -x $path ]] || { echo "error: $label is not executable: $path" >&2; return 2; }
    done
    local cc_target cxx_target
    cc_target=$($cc -dumpmachine) || return 2
    cxx_target=$($cxx -dumpmachine) || return 2
    [[ $cc_target == "$cxx_target" ]] || {
        echo "error: incoherent host compiler targets: CC=$cc_target CXX=$cxx_target" >&2
        return 2
    }
    export HOST_CC=$cc HOST_CXX=$cxx HOST_TARGET=$cc_target
}

glibc_safe_remove_tree() { # canonical-work candidate
    local work_real candidate_real base
    work_real=$(realpath -m -- "$1")
    candidate_real=$(realpath -m -- "$2")
    [[ $work_real != / && $work_real != "$HOME" ]] || {
        echo "error: refusing destructive GLIBC_WORK_DIR: $work_real" >&2
        return 2
    }
    base=${candidate_real##*/}
    case $base in
        glibc-*|build-lccc|stage-lccc) ;;
        *) echo "error: refusing unknown build child: $candidate_real" >&2; return 2 ;;
    esac
    [[ $candidate_real == "$work_real"/* ]] || {
        echo "error: refusing rm outside guarded work tree: $candidate_real" >&2
        return 2
    }
    rm -rf --one-file-system -- "$candidate_real"
}

glibc_patch_git_provenance() { # patch; prints revision or sentinel
    local root
    if root=$(git -C "$(dirname -- "$1")" rev-parse --show-toplevel 2>/dev/null); then
        git -C "$root" rev-parse HEAD
    else
        printf '%s\n' not-a-git-checkout
    fi
}

glibc_version_line() { # executable; always emits one line
    local output
    if ! output=$($1 --version 2>&1); then
        printf '<version probe failed: %s>\n' "$1"
    elif [[ -z $output ]]; then
        printf '<empty version output: %s>\n' "$1"
    else
        printf '%s\n' "${output%%$'\n'*}"
    fi
}

glibc_report_build_failure() { # status build-dir
    local status=$1 build=$2
    echo "error: glibc build failed (status $status); log tails:" >&2
    local log
    for log in configure build install; do
        if [[ -f $build/$log.log ]]; then
            printf '%s\n' "==> $log.log <==" >&2
            tail -n 40 "$build/$log.log" >&2 || true
        fi
    done
    return "$status"
}
