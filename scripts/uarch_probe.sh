#!/usr/bin/env bash
# Build and run scripts/uarch_probe.c pinned to one CPU (default: last online).
# Output is cycles relative to a dependent ADD chain, so it is valid inside a
# VM without PMU access.  Usage: scripts/uarch_probe.sh [cpu-number]
set -euo pipefail
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cpu=${1:-$(( $(nproc) - 1 ))}
bin=$(mktemp /tmp/uarch_probe.XXXXXX)
trap 'rm -f "$bin"' EXIT
gcc -O2 -o "$bin" "$here/uarch_probe.c"
echo "# host: $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2- | xargs) (cpu $cpu)"
taskset -c "$cpu" "$bin"
