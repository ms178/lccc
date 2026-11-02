#!/usr/bin/env bash
# Regression coverage for scripts/rust_toolchain.sh.
#
# Keep manifest parsing and override precedence stable: a harness restore or
# bisection must never silently pick whichever rustup default happened to be
# installed.  This test does not install any toolchain; its synthetic channel
# names prove only selection behavior.
set -euo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
# shellcheck source=../../scripts/rust_toolchain.sh
source "$repo_root/scripts/rust_toolchain.sh"

expect_eq() {
    local actual=$1 expected=$2 label=$3
    if [[ "$actual" != "$expected" ]]; then
        printf 'FAIL %s: got %q, expected %q\n' "$label" "$actual" "$expected" >&2
        exit 1
    fi
}

manifest_channel=$(lccc_rust_toolchain_from_manifest "$repo_root")
(
    unset LCCC_RUST_TOOLCHAIN RUSTUP_TOOLCHAIN LCCC_SELECTED_RUST_TOOLCHAIN
    lccc_select_rust_toolchain "$repo_root"
    expect_eq "$RUSTUP_TOOLCHAIN" "$manifest_channel" "repository manifest channel"
    expect_eq "$LCCC_SELECTED_RUST_TOOLCHAIN" "$manifest_channel" "reported manifest channel"
)

(
    export RUSTUP_TOOLCHAIN=caller-override
    unset LCCC_RUST_TOOLCHAIN LCCC_SELECTED_RUST_TOOLCHAIN
    lccc_select_rust_toolchain "$repo_root"
    expect_eq "$RUSTUP_TOOLCHAIN" caller-override "caller Rustup override"
)

(
    export RUSTUP_TOOLCHAIN=caller-override
    export LCCC_RUST_TOOLCHAIN=lccc-override
    unset LCCC_SELECTED_RUST_TOOLCHAIN
    lccc_select_rust_toolchain "$repo_root"
    expect_eq "$RUSTUP_TOOLCHAIN" lccc-override "LCCC override precedence"
)

tmp=$(mktemp -d "${TMPDIR:-/tmp}/lccc-toolchain-selector.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
printf '[toolchain]\nchannel = "synthetic-channel"\n' > "$tmp/rust-toolchain.toml"
expect_eq "$(lccc_rust_toolchain_from_manifest "$tmp")" synthetic-channel "synthetic manifest channel"
rm "$tmp/rust-toolchain.toml"
expect_eq "$(lccc_rust_toolchain_from_manifest "$tmp")" stable "missing-manifest fallback"

printf 'PASS rust_toolchain selector: manifest=%s, overrides honored\n' "$manifest_channel"

# Execute the real bootstrap function with a tiny rustup stub, never the
# bootstrap's swap/apt/repository side effects.
python3 - "$repo_root" <<'PYTEST'
from pathlib import Path
import os, subprocess, sys, tempfile
root = Path(sys.argv[1])
text = (root / 'scripts/lccc-bootstrap.sh').read_text()
a = text.index('setup_rust() {'); b = text.index('\n}\n', a) + 3
for version, should_install, component_fail in [('stable', True, False), ('beta', True, False),
        ('nightly', True, False), ('1.99.0', False, False), ('1.98.0', True, False), ('stable', True, True)]:
    with tempfile.TemporaryDirectory() as tmp:
        home = Path(tmp); binary = home / '.cargo/bin'; binary.mkdir(parents=True)
        stub = binary / 'rustup'
        stub.write_text('#!/bin/bash\necho "$*" >> "$LOG"\n'
            'if [[ "$*" == "toolchain list" ]]; then printf "stable-x86_64-unknown-linux-gnu\\n1.99.0-x86_64-unknown-linux-gnu\\n"; fi\n'
            'if [[ "$1" == component && "$COMPONENT_FAIL" == 1 ]]; then exit 7; fi\nexit 0\n')
        stub.chmod(0o755)
        script = 'set -euo pipefail\nlog() { :; }\nRUST_PROXIES=(cargo rustc)\n' + text[a:b] + '\nsetup_rust\n'
        result = subprocess.run(['bash', '-c', script], text=True, capture_output=True,
            env=dict(os.environ, HOME=tmp, RUST_VERSION=version, LOG=str(home / 'log'), COMPONENT_FAIL=str(int(component_fail))))
        commands = (home / 'log').read_text().splitlines()
        installed = any(c.startswith('toolchain install ' + version + ' ') for c in commands)
        assert installed == should_install, (version, commands)
        assert (result.returncode != 0) == component_fail, (version, result.stderr)
print('PASS bootstrap Rust freshness/component contracts: 6 scenarios')
PYTEST
