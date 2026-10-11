#!/usr/bin/env bash
# Complete, SHA-pinned Expat 2.8.5 C library/tools/tests, LCCC vs native GCC.
# Correctness only: no timings, PMU claims, package recipe PGO or C++ compiler claim.
set -euo pipefail
if [[ $# != 0 ]]; then
    if [[ $# == 1 && ( $1 == --help || $1 == -h ) ]]; then
        echo 'usage: LCCC=/path/to/lccc ARTIFACT_DIR=/persisted/evidence WORK_ROOT=/large/drive bash run.sh'
        echo 'Optional: GCC=gcc CACHE=/archive/cache KEEP_WORK=1; requires native Linux and active swap.'
        exit 0
    fi
    echo 'unexpected arguments; use --help (configuration is through environment variables)' >&2
    exit 2
fi
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
LCCC=${LCCC:-$ROOT/target/fastbuild/lccc}
GCC=${GCC:-gcc}
ARTIFACT_DIR=${ARTIFACT_DIR:-$ROOT/target/expat-validation}
CACHE=${CACHE:-$HOME/.cache/lccc-expat}
WORK_ROOT=${WORK_ROOT:-${TMPDIR:-/tmp}}
KEEP_WORK=${KEEP_WORK:-0}
VERSION=2.8.5
SHA256=952c03c33a6b337f12dae7a9b0f9dee86f867550d35c994d6bdaaddd37dc8454
URL=https://github.com/libexpat/libexpat/releases/download/R_2_8_5/expat-$VERSION.tar.bz2
[[ -x $LCCC ]] || { echo "missing compiler: $LCCC" >&2; exit 2; }
[[ $(wc -l </proc/swaps) -gt 1 ]] || { echo 'active swap is required' >&2; exit 2; }
for tool in "$GCC" cmake ninja curl sha256sum python3 timeout tar flock; do
    command -v "$tool" >/dev/null || { echo "missing tool: $tool" >&2; exit 2; }
done
LCCC=$(realpath "$LCCC")
GCC=$(command -v "$GCC")
mkdir -p "$ARTIFACT_DIR" "$CACHE" "$WORK_ROOT"
ARTIFACT_DIR=$(realpath "$ARTIFACT_DIR")
exec 9>"$ARTIFACT_DIR/.writer.lock"
flock -n 9 || { echo 'artifact directory is in use' >&2; exit 2; }
WORK=$(mktemp -d "$WORK_ROOT/lccc-expat.XXXXXX")
# Never delete a caller-provided directory: only our private mktemp child.
cleanup() { [[ $KEEP_WORK == 1 ]] || rm -rf -- "$WORK"; }
trap cleanup EXIT
printf '%s\n' "$WORK" > "$ARTIFACT_DIR/work-path.txt"
printf '{"complete":false,"status":"in-progress"}\n' > "$ARTIFACT_DIR/summary.json.tmp"
mv "$ARTIFACT_DIR/summary.json.tmp" "$ARTIFACT_DIR/summary.json"
ARCHIVE=$CACHE/expat-$VERSION.tar.bz2
if [[ ! -f $ARCHIVE ]]; then
    curl -fL --retry 2 --max-time 120 "$URL" -o "$WORK/download.tar.bz2"
    printf '%s  %s\n' "$SHA256" "$WORK/download.tar.bz2" | sha256sum -c -
    cp "$WORK/download.tar.bz2" "$CACHE/.expat-$VERSION.$$.tmp"
    mv "$CACHE/.expat-$VERSION.$$.tmp" "$ARCHIVE"
fi
printf '%s  %s\n' "$SHA256" "$ARCHIVE" | sha256sum -c - \
    > "$ARTIFACT_DIR/archive-check.log"
# The exact signed-release bytes are pinned above. In addition, reject all
# links, special entries and escaping paths BEFORE giving the archive to tar.
python3 - "$ARCHIVE" "$VERSION" <<'PY'
import sys, tarfile
from pathlib import PurePosixPath
with tarfile.open(sys.argv[1], 'r:bz2') as archive:
    members = archive.getmembers()
    if not members or len(members) > 10000 or sum(m.size for m in members) > 128 * 1024 * 1024:
        raise SystemExit('unexpected archive size/member count')
    names = set()
    for member in members:
        path = PurePosixPath(member.name)
        if (path.is_absolute() or '..' in path.parts or not path.parts
                or path.parts[0] != 'expat-' + sys.argv[2]
                or not (member.isfile() or member.isdir()) or member.name in names):
            raise SystemExit('unsafe/duplicate archive member: ' + member.name)
        names.add(member.name)
PY
tar --extract --bzip2 --no-same-owner --no-same-permissions -f "$ARCHIVE" -C "$WORK"
SRC=$WORK/expat-$VERSION
INC=$("$GCC" -print-file-name=include)
"$LCCC" --version > "$ARTIFACT_DIR/lccc-version.txt"
"$GCC" --version > "$ARTIFACT_DIR/gcc-version.txt"
sha256sum "$LCCC" "$GCC" > "$ARTIFACT_DIR/compilers.sha256"
/sbin/swapon --show > "$ARTIFACT_DIR/swap.txt"
for key in gcc lccc; do
    cc=$GCC; [[ $key != lccc ]] || cc=$LCCC
    flags='-O2 -DNDEBUG'
    [[ $key != lccc ]] || flags="$flags -I$INC"
    timeout --kill-after=5s 300 cmake -S "$SRC" -B "$WORK/$key" -G Ninja \
        -DCMAKE_C_COMPILER="$cc" \
        -DCMAKE_BUILD_TYPE=Release -DCMAKE_C_FLAGS= \
        -DCMAKE_EXPORT_COMPILE_COMMANDS=ON -DCMAKE_C_FLAGS_RELEASE="$flags" \
        -DEXPAT_SHARED_LIBS=OFF -DEXPAT_BUILD_TESTS=ON -DEXPAT_BUILD_TOOLS=ON \
        -DEXPAT_BUILD_EXAMPLES=ON -DEXPAT_BUILD_DOCS=OFF \
        > "$ARTIFACT_DIR/configure-$key.log" 2>&1
    cp "$WORK/$key/compile_commands.json" "$ARTIFACT_DIR/compile-commands-$key.json"
    cp "$WORK/$key/CMakeCache.txt" "$ARTIFACT_DIR/cmake-cache-$key.txt"
    timeout --kill-after=5s 900 cmake --build "$WORK/$key" --parallel 2 \
        > "$ARTIFACT_DIR/build-$key.log" 2>&1
    timeout --kill-after=5s 600 ctest --test-dir "$WORK/$key" --output-on-failure \
        --no-tests=error --timeout 120 -j2 --output-junit "$ARTIFACT_DIR/ctest-$key.xml" \
        > "$ARTIFACT_DIR/ctest-$key.log" 2>&1
    cp "$WORK/$key/xmlwf/xmlwf" "$ARTIFACT_DIR/xmlwf-$key"
done
python3 - "$ARTIFACT_DIR" "$WORK" "$SHA256" <<'PY'
import hashlib, json, subprocess, sys, xml.etree.ElementTree as ET
from pathlib import Path
art, work = map(Path, sys.argv[1:3])
reports = {}
for key in ('gcc', 'lccc'):
    root = ET.parse(art / ('ctest-' + key + '.xml')).getroot()
    cases = list(root.iter('testcase'))
    if not cases or any(c.find('failure') is not None or c.find('error') is not None
                        or c.find('skipped') is not None for c in cases):
        raise SystemExit('empty, failed or skipped CTest result: ' + key)
    reports[key] = len(cases)
fixtures = {
    'attributes.xml': '<root z="7" a="x&amp;y"><empty/>text&#65;</root>',
    'namespaces.xml': '<r xmlns="urn:root" xmlns:p="urn:p"><p:x p:a="v">ok</p:x></r>',
    'entities.xml': '<!DOCTYPE r [<!ENTITY x "green&amp;blue">]><r>&x;</r>',
    'unicode.xml': '<café><![CDATA[raw < & >]]> π 😀</café>',
    'large.xml': '<root>' + '<entry id="178" x="y">café &amp; text</entry>' * 32768 + '</root>',
}
invalid = {'bad-nesting.xml': b'<a><b></a>', 'bad-utf8.xml': b'<r>\xf0\x80\x80\x80</r>',
           'bad-entity.xml': b'<r>&undefined;</r>'}
checks = []
for name, content in {**fixtures, **invalid}.items():
    source = work / name
    source.write_bytes(content.encode() if isinstance(content, str) else content)
    outputs = []
    for key in ('gcc', 'lccc'):
        directory = work / (key + '-xml')
        directory.mkdir(exist_ok=True)
        cmd = [str(art / ('xmlwf-' + key)), '-n', '-r', '-d', str(directory), str(source)]
        p = subprocess.run(cmd, capture_output=True, timeout=30, check=False)
        (art / (key + '-' + name + '.log')).write_bytes(p.stdout + p.stderr)
        # Signal/timeout/infrastructure failure is not a valid XML rejection.
        expected = 2 if name in invalid else 0
        if p.returncode != expected:
            raise SystemExit(f'{key} {name}: status {p.returncode}, expected {expected}')
        if name not in invalid:
            result = directory / name
            if not result.is_file() or not result.stat().st_size:
                raise SystemExit('missing/empty canonical XML output: ' + str(result))
            outputs.append(result.read_bytes())
    if outputs and outputs[0] != outputs[1]:
        raise SystemExit('canonical XML differs: ' + name)
    checks.append({'name': name, 'status': 'pass', 'kind': 'reject' if name in invalid else 'canonical',
                   'input_sha256': hashlib.sha256(source.read_bytes()).hexdigest(),
                   'output_sha256': hashlib.sha256(outputs[0]).hexdigest() if outputs else None})
summary = {'complete': True, 'status': 'pass', 'version': '2.8.5', 'archive_sha256': sys.argv[3],
           'measurement': 'correctness only, not a timing or PMU result',
           'flags': '-O2 -DNDEBUG', 'jobs': 2, 'ctest_pass': reports, 'xml_cases': checks,
           'scope': 'static C library, examples, xmlwf and upstream C tests; no C++ code in this configuration'}
tmp = art / 'summary.json.tmp'
tmp.write_text(json.dumps(summary, indent=2) + '\n')
tmp.replace(art / 'summary.json')
print(json.dumps(summary, indent=2))
PY
