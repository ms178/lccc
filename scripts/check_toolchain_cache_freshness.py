#!/usr/bin/env python3
"""Is every package in the tag hash cache still the latest public release?

The repository's oracle toolchains arrive through the transport tags
(`harness-cache-1/2/3/4`, `rust-cache-1.97.0`), the reviewed
content-addressed release assets (`engineering/toolchains/cache-lock.json`
on the `harness-cache-4` release) and a set of digest-pinned upstream
installers that replaced individual stale tag payloads.  A pin only
stays honest while nobody ships a newer release upstream — and
upstream never notifies this repository.  Before this check existed,
the cache quietly held mold 2.42.1 for three weeks after mold 3.0.0,
QEMU 8.2.2 for *two years* after its release, and a v6.12 kernel
relocs tool, while every gate believed it compared against the
latest public oracle (see
engineering/FOLLOWUP-2026-10-27-tag-hash-cache-refresh.md).

What this tool does
-------------------
Offline (`--offline`, no network, what the unit tests run):
    Reads the pinned version out of every file that states one —
    the tag restorer, each installer, the cache lock, the kernel
    tools, the QEMU/ARM/ICX/Valgrind provisioners, the MSRV floor —
    and fails if any two files disagree about the same package.
    This is the check that makes a version bump a *complete* bump.

Online (default):
    Additionally asks each upstream for its latest public release
    (GitHub release APIs, ftp.gnu.org / sourceware listings,
    kernel.org's releases.json, Arm's permalink catalogue, Intel's
    oneAPI APT index, download.qemu.org) and fails with STALE for
    every pin that is no longer the latest.  For mold and LLVM it
    also compares the GitHub-published asset digest against the
    digest pinned in the installer, so a re-issued asset is caught
    even when the version string does not move.

Intentional exceptions are reported as INFO, never silently skipped:
    * `rust-cache-1.97.0` is the deliberate MSRV floor
      (Cargo.toml `rust-version`); it must stay behind by design.
    * the `harness-cache-4` git tree's Clang 23.1.3 is a historical
      transport payload, superseded by the 23.1.4 release asset;
      nothing may restore from it (guarded in the offline checks).
    * `debs-bookworm.tar.xz` is a compatibility snapshot of Debian
      bookworm for the bookworm-built GCC prefix; "latest" for it
      means the latest bookworm point versions (checked against
      Debian's madison service where a source name maps 1:1).

Usage:
    scripts/check_toolchain_cache_freshness.py [--offline] [--json]

Exit: 0 = current (or offline-consistent), 1 = STALE / inconsistent,
      2 = usage or upstream-unreachable in online mode.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
AGENT_LOCK = ROOT / 'engineering/toolchains/agent-cache-lock.json'
UA = {"User-Agent": "lccc-toolchain-cache-freshness/1.0",
      "Accept": "application/vnd.github+json, application/json, text/plain, */*"}


# --------------------------------------------------------------------------
# Pin extraction (offline).  Every version is read from the file that
# enforces it; this table never carries a second hard-coded copy beyond
# the historical payloads whose bytes live in tags, which are facts
# about the tag trees (verified against their MANIFEST.sha256 files).
# --------------------------------------------------------------------------

def _read(rel):
    return (ROOT / rel).read_text()


def _one(rel, pattern, what):
    match = re.search(pattern, _read(rel), re.M)
    if not match:
        raise ValueError(f"{rel}: cannot find {what} (pattern {pattern!r})")
    return match.group(1)


def read_pins():
    """All pinned versions, keyed by package, from their enforcing files."""
    oracle = "scripts/oracle_toolchains_from_tags.sh"
    pins = {
        "gcc": _one(oracle, r"^GCC_VERSION=([0-9.]+)$", "GCC_VERSION"),
        "binutils": _one(oracle, r"^BINUTILS_VERSION=([0-9.]+)$", "BINUTILS_VERSION"),
        "clang": _one(oracle, r"^CLANG_VERSION=([0-9.]+)$", "CLANG_VERSION"),
        "rust": _one(oracle, r"^RUST_VERSION=([0-9.]+)$", "RUST_VERSION"),
        "mold": _one(oracle, r"^MOLD_VERSION=([0-9.]+)$", "MOLD_VERSION"),
        "qemu": _one("scripts/ensure_qemu_user.sh", r"^QEMU_VERSION=([0-9.]+)$", "QEMU_VERSION"),
        "kernel-relocs": _one("tests/linker/setup_kernel_tools.sh",
                              r"^PINNED_KVER=v([0-9.]+)$", "PINNED_KVER"),
        "arm-a64-xml": _one("scripts/ensure_arm_a64_xml.sh",
                            r'^ARM_XML_RELEASE="([0-9-]+)"$', "ARM_XML_RELEASE"),
        "icx": _one("scripts/icx_provision.py", r'^RELEASE = "([0-9.]+)"$', "ICX RELEASE"),
        "valgrind": _one("scripts/ci_install_valgrind_current.sh",
                         r"^version=([0-9.]+)$", "valgrind version"),
        "rust-msrv": _one("Cargo.toml", r'^rust-version = "([0-9.]+)"$', "rust-version"),
        # Tag-tree facts (payloads live in the transport tags, digests in
        # each tag's MANIFEST.sha256; last verified 2026-10-27):
        "clang-tag-tree": "23.1.3",     # harness-cache-4 tree (historical)
        "debs-bookworm-libc": "2.36-9+deb12u14",
        "debs-bookworm-gcc12": "12.2.0-14+deb12u1",
    }
    return pins


def offline_problems(pins):
    """Cross-file consistency: one package, one version, everywhere."""
    problems = []

    def expect(package, actual, where):
        if actual != pins[package]:
            problems.append(f"{package}: {where} pins {actual}, canonical pin is {pins[package]}")

    expect("clang", _one("scripts/ci_install_clang_oracle.sh", r"^VERSION=([0-9.]+)$", "VERSION"),
           "ci_install_clang_oracle.sh")
    expect("mold", _one("scripts/ci_install_mold_oracle.sh", r"^VERSION=([0-9.]+)$", "VERSION"),
           "ci_install_mold_oracle.sh")
    expect("mold", _one("tools/linker/setup_oracles.sh", r"^MOLD_VER=([0-9.]+)$", "MOLD_VER"),
           "tools/linker/setup_oracles.sh")
    expect("binutils", _one("tools/linker/setup_oracles.sh", r"^BINUTILS_VER=([0-9.]+)$", "BINUTILS_VER"),
           "tools/linker/setup_oracles.sh")
    expect("binutils", _one("tests/linker/setup_oracles.sh", r"^BINUTILS_VERSION=([0-9.]+)$", "BINUTILS_VERSION"),
           "tests/linker/setup_oracles.sh")

    mold_ref = _one("tests/linker/setup_oracles.sh", r"MOLD_REF=\$\{MOLD_REF:-v([0-9.]+)\}", "MOLD_REF")
    expect("mold", mold_ref, "tests/linker/setup_oracles.sh MOLD_REF")

    restore = _read("scripts/restore_toolchain_cache.py")
    versions = re.search(r"VERSIONS = \{([^}]*)\}", restore)
    if not versions:
        problems.append("restore_toolchain_cache.py: VERSIONS table not found")
    else:
        for tool, version in re.findall(r"'(\w+)': '([0-9.]+)'", versions.group(1)):
            expect(tool, version, "restore_toolchain_cache.py VERSIONS")

    lock = json.loads(_read("engineering/toolchains/cache-lock.json"))
    for tool, entry in lock["tools"].items():
        expect(tool, entry["version"], "cache-lock.json")
        asset = lock["tools"][tool]["url"].rsplit("/", 1)[-1]
        want = f"{tool}-{entry['version']}-{entry['sha256']}.tar.xz"
        if asset != want:
            problems.append(f"{tool}: cache-lock asset name {asset!r} is not content-addressed {want!r}")

    publisher = _read("scripts/publish_toolchain_cache.py")
    for tool, version in re.findall(r'\("(\w+)", "([0-9.]+)"', publisher):
        expect(tool, version, "publish_toolchain_cache.py")

    # The binutils archive digest is one fact stated in three places.
    gas_sha = _one("scripts/ensure_gas_247.sh", r"^GAS_TARBALL_SHA256=([0-9a-f]{64})$", "GAS_TARBALL_SHA256")
    for rel, pattern in (("scripts/oracle_toolchains_from_tags.sh", r"^BINUTILS_SHA=([0-9a-f]{64})$"),
                         ("tools/linker/setup_oracles.sh", r"^BINUTILS_SHA256=([0-9a-f]{64})$")):
        if _one(rel, pattern, "binutils sha256") != gas_sha:
            problems.append(f"binutils: {rel} digest disagrees with ensure_gas_247.sh")

    # GCC: the bookworm tag archive digest in the tag restorer and the
    # hosted installer must be the same bytes.
    gcc_sha = _one("scripts/oracle_toolchains_from_tags.sh", r"^GCC_SHA=([0-9a-f]{64})$", "GCC_SHA")
    if _one("scripts/ci_install_gcc_oracle.sh", r"^SHA256=([0-9a-f]{64})$", "SHA256") != gcc_sha:
        problems.append("gcc: ci_install_gcc_oracle.sh digest disagrees with oracle_toolchains_from_tags.sh")

    # The GitHub-only agent cache is pinned by a reviewed lock; every tool in
    # it must agree with the same enforcing files that drive hosted CI.
    if not AGENT_LOCK.is_file():
        problems.append('agent-cache-lock.json is missing')
    else:
        agent = json.loads(AGENT_LOCK.read_text())
        if agent.get('tag') != 'harness-cache-7':
            problems.append(f"agent-cache-lock.json names tag {agent.get('tag')!r}, expected 'harness-cache-7'")
        tools = agent.get('tools', {})
        for package in ('gcc', 'binutils', 'clang', 'rust', 'mold', 'qemu',
                        'kernel-relocs', 'arm-a64-xml', 'rust-msrv'):
            entry = tools.get(package)
            if not entry:
                problems.append(f'agent-cache-lock.json has no {package} entry')
                continue
            expect(package, entry.get('version', ''), 'agent-cache-lock.json')
        for field in ('tag_sha', 'commit_sha', 'tree_sha', 'cache_json_sha256'):
            if not agent.get(field):
                problems.append(f'agent-cache-lock.json has no {field}')

    # Nothing may restore Clang from the historical 23.1.3 tag tree or
    # mold from the superseded 2.42.1 tag binary any more.
    for rel in ("scripts/oracle_toolchains_from_tags.sh", "scripts/ci_prepare_research_tools.sh",
                "scripts/restore_toolchain_cache.py"):
        text = _read(rel)
        for stale in ("clang-23.1.3", "mold-2.42.1-x86_64.part00", "86b918f8af9444"):
            if stale in text:
                problems.append(f"{rel}: still references superseded payload {stale}")
    return problems


# --------------------------------------------------------------------------
# Upstream latest (online).  Parsers are pure functions over response
# bytes so the unit tests can feed them canned upstream answers.
# --------------------------------------------------------------------------

def vtuple(version):
    return tuple(int(part) for part in re.findall(r"[0-9]+", str(version)))


def parse_gcc_listing(text):
    versions = {m for m in re.findall(r"gcc-([0-9]+\.[0-9]+\.[0-9]+)/", text)}
    return max(versions, key=vtuple)


def parse_binutils_listing(text):
    versions = {m for m in re.findall(r"binutils-([0-9]+\.[0-9]+(?:\.[0-9]+)?)\.tar", text)}
    return max(versions, key=vtuple)


def parse_qemu_listing(text):
    versions = {m for m in re.findall(r"qemu-([0-9]+\.[0-9]+\.[0-9]+)\.tar\.xz", text)}
    return max(versions, key=vtuple)


def parse_valgrind_listing(text):
    versions = {m for m in re.findall(r"valgrind-([0-9]+\.[0-9]+\.[0-9]+)\.tar", text)}
    return max(versions, key=vtuple)


def parse_llvm_releases(data):
    for release in data:
        if release.get("prerelease") or release.get("draft"):
            continue
        tag = release.get("tag_name", "")
        if tag.startswith("llvmorg-"):
            return tag.removeprefix("llvmorg-"), release
    raise ValueError("no non-prerelease llvmorg-* release in response")


def parse_icx_packages(text):
    lines = {m for m in re.findall(r"^Package: intel-oneapi-compiler-dpcpp-cpp-runtime-([0-9]+\.[0-9]+)$",
                                    text, re.M)}
    return max(lines, key=vtuple)


def parse_madison(text, suite):
    """Latest version a Debian source package has in `suite` (madison text)."""
    found = None
    for line in text.splitlines():
        parts = [p.strip() for p in line.split("|")]
        if len(parts) >= 3 and parts[2] == suite:
            found = parts[1]  # rows are ordered old -> new by suite appearance
    if found is None:
        raise ValueError(f"no madison row for suite {suite}")
    return found


def _fetch(url, timeout=45):
    request = urllib.request.Request(url, headers=UA)
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return response.read()


def _fetch_json(url):
    return json.loads(_fetch(url))


def _head_ok(url):
    # developer.arm.com's CDN 403s Python's TLS client but serves curl
    # (the transport ensure_arm_a64_xml.sh itself uses), so curl is the
    # primary probe and urllib only the fallback for curl-less hosts.
    import shutil
    import subprocess
    if shutil.which("curl"):
        result = subprocess.run(
            ["curl", "-sIL", "--max-time", "30", "-o", "/dev/null",
             "-w", "%{http_code}", url],
            capture_output=True, text=True, timeout=40)
        if result.returncode == 0 and result.stdout.strip() in ("200", "404"):
            return result.stdout.strip() == "200"
        raise OSError(f"curl HEAD probe failed for {url}: {result.stderr.strip()[:200]}")
    request = urllib.request.Request(url, headers=UA, method="HEAD")
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return response.status == 200
    except urllib.error.HTTPError as error:
        if error.code == 404:
            return False
        raise


def _next_months(release, count=4):
    year, month = (int(part) for part in release.split("-"))
    out = []
    for _ in range(count):
        month += 1
        if month > 12:
            year, month = year + 1, 1
        out.append(f"{year:04d}-{month:02d}")
    return out


def upstream_latest(pins):
    """(package, latest, evidence) rows; a failed source yields a
    `__error__` row instead of aborting the remaining verdicts."""
    rows = []

    def attempt(fn):
        try:
            fn()
        except Exception as error:  # noqa: BLE001 - reported, not swallowed
            rows.append(("__error__", fn.__name__, f"{type(error).__name__}: {error}"))

    def clang():
        version, release = parse_llvm_releases(
            _fetch_json("https://api.github.com/repos/llvm/llvm-project/releases?per_page=10"))
        rows.append(("clang", version, "github.com/llvm/llvm-project/releases"))
        asset = f"LLVM-{pins['clang']}-Linux-X64.tar.zst"
        pinned = _one("scripts/ci_install_clang_oracle.sh", r"^SHA256=([0-9a-f]{64})$", "SHA256")
        digests = {a["name"]: (a.get("digest") or "") for a in release.get("assets", [])}
        if version == pins["clang"] and digests.get(asset) != f"sha256:{pinned}":
            rows.append(("clang-asset-digest", digests.get(asset, "?"),
                         f"installer pins sha256:{pinned}; asset re-issued or wrong digest"))

    def rust():
        data = _fetch_json("https://api.github.com/repos/rust-lang/rust/releases/latest")
        rows.append(("rust", data["tag_name"], "github.com/rust-lang/rust/releases/latest"))

    def mold():
        data = _fetch_json("https://api.github.com/repos/rui314/mold/releases/latest")
        rows.append(("mold", data["tag_name"].removeprefix("v"),
                     "github.com/rui314/mold/releases/latest"))
        asset = f"mold-{pins['mold']}-x86_64-linux.tar.gz"
        pinned = _one("scripts/ci_install_mold_oracle.sh", r"^SHA256=([0-9a-f]{64})$", "SHA256")
        digests = {a["name"]: (a.get("digest") or "") for a in data.get("assets", [])}
        if data["tag_name"].removeprefix("v") == pins["mold"] and digests.get(asset) != f"sha256:{pinned}":
            rows.append(("mold-asset-digest", digests.get(asset, "?"),
                         f"installer pins sha256:{pinned}; asset re-issued or wrong digest"))

    def gcc():
        rows.append(("gcc", parse_gcc_listing(
            _fetch("https://ftp.gnu.org/gnu/gcc/").decode("utf-8", "ignore")), "ftp.gnu.org/gnu/gcc/"))

    def binutils():
        rows.append(("binutils", parse_binutils_listing(
            _fetch("https://ftp.gnu.org/gnu/binutils/").decode("utf-8", "ignore")),
            "ftp.gnu.org/gnu/binutils/"))

    def qemu():
        rows.append(("qemu", parse_qemu_listing(
            _fetch("https://download.qemu.org/").decode("utf-8", "ignore")), "download.qemu.org/"))

    def kernel():
        data = _fetch_json("https://www.kernel.org/releases.json")
        rows.append(("kernel-relocs", data["latest_stable"]["version"], "kernel.org/releases.json"))

    def valgrind():
        rows.append(("valgrind", parse_valgrind_listing(
            _fetch("https://sourceware.org/pub/valgrind/").decode("utf-8", "ignore")),
            "sourceware.org/pub/valgrind/"))

    def icx():
        rows.append(("icx", parse_icx_packages(_fetch(
            "https://apt.repos.intel.com/oneapi/dists/all/main/binary-amd64/Packages"
        ).decode("utf-8", "ignore")), "apt.repos.intel.com/oneapi Packages index"))

    def arm():
        # Arm publishes no catalogue API; the pinned permalink must exist
        # and no later monthly permalink may.  (Arm ISA XML releases are
        # named by their release month.)
        base = ("https://developer.arm.com/-/cdn-downloads/permalink/"
                "Exploration-Tools-A64-ISA/ISA_A64/ISA_A64_xml_A_profile-{}_md.tar.gz")
        if not _head_ok(base.format(pins["arm-a64-xml"])):
            rows.append(("arm-a64-xml", "PINNED-PERMALINK-404", base.format(pins["arm-a64-xml"])))
            return
        newer = [m for m in _next_months(pins["arm-a64-xml"]) if _head_ok(base.format(m))]
        rows.append(("arm-a64-xml", newer[-1] if newer else pins["arm-a64-xml"],
                     "developer.arm.com Exploration-Tools permalinks"))

    def debs_bookworm():
        # "Latest" for the bookworm snapshot is the latest bookworm point version.
        for package, source in (("debs-bookworm-libc", "libc6"), ("debs-bookworm-gcc12", "gcc-12")):
            text = _fetch(
                f"https://qa.debian.org/cgi-bin/madison.cgi?package={source}&l=on&text=on"
            ).decode("utf-8", "ignore")
            rows.append((package, parse_madison(text, "bookworm"),
                         f"Debian madison ({source}, bookworm)"))

    for source in (clang, rust, mold, gcc, binutils, qemu, kernel, valgrind, icx, arm, debs_bookworm):
        attempt(source)
    return rows


INFO_ROWS = (
    ("rust-msrv", "deliberate MSRV floor (Cargo.toml rust-version); "
                  "the rust-cache-{v} tag must stay behind current stable by design"),
    ("clang-tag-tree", "historical harness-cache-4 git-tree payload; superseded by the "
                       "content-addressed Clang release asset in cache-lock.json"),
)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--offline", action="store_true",
                        help="cross-file consistency only; never touch the network")
    parser.add_argument("--json", action="store_true", help="machine-readable report")
    args = parser.parse_args(argv)

    pins = read_pins()
    problems = offline_problems(pins)
    report = {"pins": pins, "problems": problems, "upstream": {}, "stale": [], "info": [], "errors": []}
    for package, why in INFO_ROWS:
        report["info"].append({"package": package, "pinned": pins[package],
                                "note": why.format(v=pins[package])})

    errors = []
    if not args.offline:
        rows = upstream_latest(pins)
        errors = [row for row in rows if row[0] == "__error__"]
        rows = [row for row in rows if row[0] != "__error__"]
        report["errors"] = [{"source": row[1], "error": row[2]} for row in errors]
        for package, latest, evidence in rows:
            report["upstream"][package] = {"latest": latest, "evidence": evidence}
            if package.endswith("-asset-digest"):
                report["stale"].append({"package": package, "pinned": "see evidence", "latest": latest,
                                         "evidence": evidence})
            elif vtuple(latest) > vtuple(pins[package]):
                report["stale"].append({"package": package, "pinned": pins[package],
                                         "latest": latest, "evidence": evidence})

    if args.json:
        print(json.dumps(report, indent=2, sort_keys=True))
    else:
        checked = sorted(set(pins) - {name for name, _ in INFO_ROWS})
        print(f"pinned packages checked: {len(checked)} "
              f"({'offline consistency only' if args.offline else 'offline + upstream latest'})")
        for package in checked:
            latest = report["upstream"].get(package, {}).get("latest")
            state = "CURRENT" if latest is None or vtuple(latest) <= vtuple(pins[package]) else "STALE"
            suffix = f" (latest upstream: {latest})" if latest is not None else ""
            print(f"  {state:7} {package:22} {pins[package]}{suffix}")
        for row in report["info"]:
            print(f"  {'INFO':7} {row['package']:22} {row['pinned']} — {row['note']}")
        for problem in problems:
            print(f"  BROKEN  {problem}")
        for row in report["errors"]:
            print(f"  ERROR   upstream query {row['source']} failed: {row['error']}")
        for row in report["stale"]:
            print(f"  STALE   {row['package']}: pinned {row['pinned']}, latest {row['latest']} ({row['evidence']})")
    if problems or report["stale"]:
        return 1
    return 2 if report["errors"] else 0


if __name__ == "__main__":
    sys.exit(main())
