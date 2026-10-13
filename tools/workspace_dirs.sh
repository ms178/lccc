#!/usr/bin/env bash
# Shared resolution of the PERSISTED (user-facing) workspace directories.
#
# AUD-DOC-1: 30 scripts hardcoded `/home/user`, which is the path in ONE of the
# harnesses this project runs in.  A second harness mounts the user-facing
# directory at /mnt/user-data/outputs and has no /home/user at all, so every
# hardcoded path silently wrote into a void (or failed late, after the work was
# done).  Resolve once, from the environment, with a loud failure:
#
#   LCCC_WORKSPACE   explicit override (wins)
#   /home/user       this project's primary harness
#   /mnt/user-data/outputs   the alternate harness
#   $PWD             last resort: the caller's directory (never silent)
#
# Usage:
#   . "$(dirname "$0")/..//tools/workspace_dirs.sh"
#   ws=$(lccc_workspace_dir)          # persisted workspace root
#   art=$(lccc_artifacts_dir)         # $ws/artifacts
#   patch=$(lccc_deliverable_path)    # $ws/ms178-1.patch
lccc_workspace_dir() {
    if [[ -n ${LCCC_WORKSPACE:-} ]]; then
        [[ -d $LCCC_WORKSPACE ]] || { echo "LCCC_WORKSPACE=$LCCC_WORKSPACE is not a directory" >&2; return 1; }
        printf '%s\n' "$LCCC_WORKSPACE"; return 0
    fi
    local candidate
    for candidate in /home/user /mnt/user-data/outputs; do
        if [[ -d $candidate && -w $candidate ]]; then
            printf '%s\n' "$candidate"; return 0
        fi
    done
    if [[ -w $PWD ]]; then
        printf '%s\n' "$PWD"; return 0
    fi
    echo "no writable persisted workspace found; set LCCC_WORKSPACE" >&2
    return 1
}

lccc_artifacts_dir() { printf '%s/artifacts\n' "$(lccc_workspace_dir)" || return 1; }
lccc_deliverable_path() { printf '%s/ms178-1.patch\n' "$(lccc_workspace_dir)" || return 1; }
