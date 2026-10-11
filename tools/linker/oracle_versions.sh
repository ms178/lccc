#!/usr/bin/env bash
# Source-only helpers: a directory name is not proof of the binary's version.
# Match a complete whitespace-delimited release token (2.47 != 2.470/2.47.1).
# The pinned binutils-2.47 release archive itself emits 2.47.20260726, just
# like the GAS pair built from those bytes. Permit exactly that archive's
# dated banner too, NOT arbitrary patch levels or unrelated dated snapshots.
oracle_version_matches() {
    local banner=$1 version=$2 escaped
    [[ $version =~ ^[0-9]+(\.[0-9]+)+$ ]] || return 2
    escaped=${version//./\\.}
    if [[ $version == 2.47 ]]; then
        escaped='2\.47(\.20260726)?'
    fi
    [[ $banner =~ (^|[[:space:]])${escaped}($|[[:space:]]) ]]
}

oracle_require_version() {
    local exe=$1 expected=$2 banner
    if ! banner=$("$exe" --version 2>&1); then
        printf 'oracle executable failed: %s\n%s\n' "$exe" "$banner" >&2
        return 1
    fi
    # Only the first line is the tool identity; license text is not a banner.
    banner=${banner%%$'\n'*}
    if ! oracle_version_matches "$banner" "$expected"; then
        printf 'oracle version mismatch: %s; expected %s; got %s\n' \
            "$exe" "$expected" "$banner" >&2
        return 1
    fi
}
