#!/usr/bin/env bash
# Source-only helpers: a directory name is not proof of the binary's version.
# Match a complete whitespace-delimited release token (2.47 != 2.470/2.47.1).
oracle_version_matches() {
    local banner=$1 version=$2 escaped
    [[ $version =~ ^[0-9]+(\.[0-9]+)+$ ]] || return 2
    escaped=${version//./\\.}
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
