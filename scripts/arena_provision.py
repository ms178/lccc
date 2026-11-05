#!/usr/bin/env python3
"""Provision the LCCC research toolchains through the Arena host allowlist.

Arena's sandbox permits outbound traffic to ``github.com``,
``codeload.github.com``, ``api.github.com``, ``registry.npmjs.org``,
``pypi.org`` and ``files.pythonhosted.org`` only.  It does NOT permit
``release-assets.githubusercontent.com``, ``objects.githubusercontent.com``,
``raw.githubusercontent.com``, ``static.rust-lang.org`` or the Debian/Intel
mirrors.  That breaks every provisioning script which downloads a GitHub
release asset or an upstream tarball directly.

GitHub's *blob API* (``api.github.com/repos/O/R/git/blobs/<sha>``) returns a
git object's bytes as base64 in JSON and is reachable here.  Toolchains that
are already committed to a git ref can therefore be restored byte-exactly
with pinned blob SHAs and a pinned SHA-256 of the reassembled archive.  This
script uses ONLY that transport (plus the repository's own sparse git
provisioner for the corpus), verifies every payload before extraction, and is
idempotent: a verified prefix is reused, never re-downloaded.

Pins (all hashes are content, not version labels):

* Rust 1.99.1, the official ``rust-1.99.1-x86_64-unknown-linux-gnu.tar.xz``
  distribution, stored as three 90 MiB chunks in ``kelexine/rust-toolchain``
  at ``toolchains/1.99.1/``.  Reassembled SHA-256:
  231fcae47ee950621374725b5fc3b5441d2fbda2955ac45dd7dc816d8876cfe7
  (that repository's own ``rust-1.99.1.tar.xz.sha256``).  The cache-lock
  release asset ``harness-cache-4/rust-1.99.1-2876…tar.xz`` is the same
  toolchain but a different archive layout; the blob route is used because
  release assets are unreachable here.
* GCC 16.2.0 relocatable prefix from ``ms178/lccc`` tag ``harness-cache-1``,
  blob ``0612f4c9…`` / SHA-256 ``4fa6af4f…`` (the digest pinned by
  ``scripts/ci_install_gcc_oracle.sh``).
* Debian bookworm i686 multilib .deb bundle from the same tag, blob
  ``06185dd8…``; installed with dpkg so ``gcc-16 -m32`` links and runs.

The corpus is GCC 17-git at the campaign revision, provisioned by
``scripts/ensure_gcc_torture_git.py`` (sparse fetch from github.com).

Usage::

    python3 scripts/arena_provision.py                 # everything
    python3 scripts/arena_provision.py --verify-only   # no network, no writes
    python3 scripts/arena_provision.py --skip-corpus   # toolchains only

Exit 0 means every requested tool answered ``--version`` with its pinned
identity.  A missing, truncated, or hash-mismatched payload is a hard error;
nothing is half-installed (downloads go to ``.part`` + atomic rename).
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import inspect
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.request

REPO = Path(__file__).resolve().parent.parent
HOME = Path(os.environ.get("HOME", "/home/user"))
DEFAULT_PREFIX = Path(os.environ.get("LCCC_TOOLCHAIN_PREFIX",
                                    HOME / ".cache/lccc-toolchains"))
CORPUS = Path(os.environ.get("LCCC_GCC17_ROOT", HOME / ".cache/lccc-gcc17"))
GCC17_REF = "c94b688a83fc84015c6fab8ab0ff142051598fc8"

RUST_CHUNKS = (
    ("1ad84bce0cf04d4052c6d4dea5d4813853e31fd1", 94_371_840),
    ("ae2e402ced123dddf6c5be24e6e42602930ac0b8", 94_371_840),
    ("73f0ebf43146701307e189bdc300c1f7571da895", 21_772_056),
)
RUST_SHA256 = "231fcae47ee950621374725b5fc3b5441d2fbda2955ac45dd7dc816d8876cfe7"
RUST_REPO = "kelexine/rust-toolchain"
RUST_VERSION = "1.99.1"

GCC_BLOB = "0612f4c9bb3e5cfdf0d5326529caa183132b71a4"
GCC_SHA256 = "4fa6af4f6b29991e31631af18d83964a69caa9daf3f78546d4dc30f39880cd8a"
DEBS_BLOB = "06185dd859247494e5c489656e0488a7f3edaf70"
# harness-cache-7 MANIFEST.sha256: the debs bundle was previously the ONLY
# payload fetched without a content pin. A tampered/corrupt bundle of the
# right byte size would have been unpacked and `dpkg --unpack`ed.
DEBS_SHA256 = "24a4be08d5c6a28aed78f424ebdc9596f85cc2e8ebd0ba230f99a2bf9cdfb20d"
# Clang is NOT restored from a git blob any more. The harness-cache-7 payload
# (blob 83ddbd38..., 67,322,400 bytes) is a repacked 23.1.4, and a main-branch
# patch cannot re-issue a tag blob -- so a bumped version here would have named
# bytes that do not exist. The oracle now comes from the same digest-pinned
# upstream installer CI uses (scripts/ci_install_clang_oracle.sh), which pins
# the official LLVM 23.1.5 asset by the SHA-256 GitHub publishes for it and
# proves the identity with `clang -dumpversion` before exiting 0.
CLANG_VERSION = "23.1.5"
GDB_VERSION = "18.1"
GCC_REPO = "ms178/lccc"
GCC_TAG = "harness-cache-1"


def api_json(url: str, timeout: int = 300) -> dict:
    request = urllib.request.Request(
        url, headers={"Accept": "application/vnd.github+json",
                      "User-Agent": "lccc-arena-provision/1"})
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.load(response)


def blob(repo: str, sha: str, expected_size: int, destination: Path) -> None:
    """Fetch one git blob, verify its object id and size, write atomically."""
    if destination.exists() and destination.stat().st_size == expected_size:
        return
    payload = api_json(f"https://api.github.com/repos/{repo}/git/blobs/{sha}")
    if payload.get("sha") != sha or payload.get("size") != expected_size \
            or payload.get("encoding") != "base64":
        raise RuntimeError(f"{repo}: blob {sha} failed identity/size/encoding check")
    # The blob API wraps base64 at 60 columns; validate=True rejects those
    # newlines. Strip *only* ASCII whitespace (never arbitrary characters),
    # so a corrupted payload still fails strict decoding.
    encoded = "".join(payload["content"].split())
    data = base64.b64decode(encoded, validate=True)
    if len(data) != expected_size:
        raise RuntimeError(f"{repo}: blob {sha} decoded to {len(data)} bytes")
    destination.parent.mkdir(parents=True, exist_ok=True)
    tmp = destination.with_suffix(".part")
    with tmp.open("wb") as stream:
        stream.write(data)
    os.replace(tmp, destination)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def run(argv: list[str], **kwargs) -> str:
    return subprocess.check_output(argv, text=True, stderr=subprocess.STDOUT,
                                   timeout=kwargs.pop("timeout", 600), **kwargs).strip()


def extractall_takes_filter() -> bool:
    """Probe the stdlib capability actually used, not a Python version.

    ``hasattr(tarfile, "data_filter")`` is the WRONG question: the
    ``filter=`` keyword lives on ``TarFile.extractall``, and the two
    capabilities were introduced in the same backport but are different
    objects. Signature inspection is what the repository's other guarded
    extractors use (``restore_toolchain_cache``), so both now answer the
    same question. A third-party ``backports.tarfile`` monkeypatch is
    likewise detected without hardcoding a version tuple.
    """
    try:
        return "filter" in inspect.signature(tarfile.TarFile.extractall).parameters
    except (TypeError, ValueError):
        return False


def extract_untrusted(source: tarfile.TarFile, destination: Path) -> None:
    """Validate EVERY member, then extract a pinned archive.

    The three bundles this script installs are our OWN pinned payloads, but
    "trusted" is not a reason to accept a traversal, an absolute symlink or
    a device node. Validation is unconditional and independent of the
    stdlib filter: on Python 3.11 (the Arena host) it is the ONLY defence,
    on 3.12+ the native ``data`` filter is a second, independent layer.
    Symlink targets are resolved relative to the LINK's directory, hardlink
    targets relative to the archive root, matching tar semantics.

    This is the repository's guarded-extractor name (see
    ``restore_agent_toolchain_cache.extract_untrusted``); the corpus safety
    contract permits ``filter=`` only inside such a guarded helper.
    """
    dest = destination.resolve()
    for member in source.getmembers():
        name = Path(member.name)
        if name.is_absolute() or ".." in name.parts:
            raise RuntimeError(f"unsafe archive member: {member.name}")
        if member.issym() or member.islnk():
            if Path(member.linkname).is_absolute():
                raise RuntimeError(f"absolute link target: {member.name}")
            base = dest / name.parent if member.issym() else dest
            target = (base / member.linkname).resolve()
            if not target.is_relative_to(dest):
                raise RuntimeError(f"escaping link target: {member.name}")
        elif not (member.isfile() or member.isdir()):
            raise RuntimeError(f"unsupported archive member type: {member.name}")
    if extractall_takes_filter():
        source.extractall(destination, filter="data")
    else:
        source.extractall(destination)


def marker_ok(marker: Path, expected: str, probes: tuple[Path, ...]) -> bool:
    """A `.verified` stamp is only as good as the files it claims to cover.

    A harness wipe can leave a directory with a stamp but no binaries (or a
    stamp from a different bundle version). Reusing it silently produced a
    "provisioned" toolchain whose compiler could not start.
    """
    return (marker.is_file()
            and marker.read_text(encoding="utf-8").strip() == expected
            and all(probe.is_file() for probe in probes))


def install_rust(prefix: Path) -> Path:
    root = prefix / f"rust-{RUST_VERSION}"
    if marker_ok(root / ".verified", RUST_SHA256,
                 tuple(root / "bin" / tool for tool in
                       ("rustc", "cargo", "rustfmt", "cargo-clippy"))):
        return root
    downloads = prefix / "dl"
    downloads.mkdir(parents=True, exist_ok=True)
    archive = downloads / f"rust-{RUST_VERSION}.tar.xz"
    if not archive.exists() or sha256(archive) != RUST_SHA256:
        tmp = archive.with_suffix(".part")
        digest = hashlib.sha256()
        with tmp.open("wb") as out:
            for index, (blob_sha, size) in enumerate(RUST_CHUNKS):
                part = downloads / f"rust-{RUST_VERSION}.part{index:03d}"
                blob(RUST_REPO, blob_sha, size, part)
                data = part.read_bytes()
                digest.update(data)
                out.write(data)
        if digest.hexdigest() != RUST_SHA256:
            tmp.unlink(missing_ok=True)
            raise RuntimeError("Rust archive SHA-256 does not match the pinned manifest")
        os.replace(tmp, archive)
    with tempfile.TemporaryDirectory(prefix="lccc-rust-", dir=prefix) as td:
        with tarfile.open(archive, "r:xz") as source:
            extract_untrusted(source, Path(td))
        unpacked = Path(td) / f"rust-{RUST_VERSION}-x86_64-unknown-linux-gnu"
        if not (unpacked / "install.sh").is_file():
            raise RuntimeError("Rust archive lacks install.sh")
        run([str(unpacked / "install.sh"), f"--prefix={root}",
             "--disable-ldconfig", "--components="
             "rustc,cargo,rust-std-x86_64-unknown-linux-gnu,"
             "rustfmt-preview,clippy-preview"], timeout=900)
    for tool in ("rustc", "cargo", "rustfmt", "cargo-clippy"):
        if not (root / "bin" / tool).is_file():
            raise RuntimeError(f"Rust install lacks {tool}")
    version = run([str(root / "bin" / "rustc"), "--version"])
    if RUST_VERSION not in version:
        raise RuntimeError(f"unexpected rustc identity: {version}")
    (root / ".verified").write_text(RUST_SHA256 + "\n")
    print(f"[arena] rustc: {version}")
    return root


def install_gcc(prefix: Path) -> Path:
    root = prefix / "gcc-16.2"
    marker = root / ".verified"
    gcc = root / "gcc-16/bin/gcc-16"
    if marker_ok(marker, GCC_SHA256, (gcc,)):
        return gcc
    downloads = prefix / "dl"
    downloads.mkdir(parents=True, exist_ok=True)
    archive = downloads / "gcc-16-bookworm-x86_64.tar.xz"
    blob(GCC_REPO, GCC_BLOB, 65_152_756, archive)
    if sha256(archive) != GCC_SHA256:
        raise RuntimeError("GCC 16.2 archive SHA-256 mismatch")
    root.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive, "r:xz") as source:
        extract_untrusted(source, root)
    version = run([str(gcc), "-dumpfullversion"])
    if not version.startswith("16.2"):
        raise RuntimeError(f"unexpected GCC identity: {version}")
    marker.write_text(GCC_SHA256 + "\n")
    print(f"[arena] gcc-16: {version}")
    return gcc


def install_multilib(prefix: Path) -> None:
    """Install the pinned bookworm i686 CRT/dev packages, once per sandbox."""
    marker = prefix / ".multilib-verified"
    if marker_ok(marker, DEBS_SHA256, ()):
        return
    downloads = prefix / "dl"
    archive = downloads / "debs-bookworm.tar.xz"
    blob(GCC_REPO, DEBS_BLOB, 32_787_168, archive)
    if sha256(archive) != DEBS_SHA256:
        raise RuntimeError("bookworm multilib archive SHA-256 mismatch")
    with tempfile.TemporaryDirectory(prefix="lccc-debs-", dir=prefix) as td:
        with tarfile.open(archive, "r:xz") as source:
            extract_untrusted(source, Path(td))
        debs = sorted((Path(td) / "debs").glob("*.deb"))
        if not debs:
            raise RuntimeError("multilib bundle contains no .deb payloads")
        sudo = ["sudo", "-n"] if os.geteuid() != 0 else []
        # `--unpack`, not `-i`: this bundle intentionally omits the x32
        # packages that gcc-12-multilib's *configuration* depends on.  Their
        # files are not needed for ILP32 (`gcc-16 -m32`), but `dpkg -i` would
        # fail configuration and leave the sandbox in a half-installed state.
        # The contract we actually care about -- compile, link and RUN a
        # 32-bit program -- is asserted by verify() after provisioning.
        run([*sudo, "dpkg", "--unpack", *map(str, debs)], timeout=900)
    marker.write_text(DEBS_SHA256 + "\n")
    print(f"[arena] installed {len(debs)} bookworm multilib packages")


def install_clang(prefix: Path) -> Path:
    """Provision the pinned Clang oracle via its digest-pinned installer.

    The sandbox used to restore a repacked Clang from the `harness-cache-7`
    git blob because the release-asset CDN was believed unreachable from an
    agent sandbox. That was re-tested on 2026-11-05: a ranged GET of
    `github.com/llvm/llvm-project/releases/download/llvmorg-23.1.5/...`
    returns 206 from here, so the official asset IS reachable and the blob
    detour now costs more than it saves -- it can only ever serve the last
    published payload (23.1.4), which is exactly the "silently stale oracle"
    failure the pin exists to prevent.

    Delegating to scripts/ci_install_clang_oracle.sh means the sandbox and
    hosted CI resolve one oracle through one code path, one digest and one
    version probe. The installer exits non-zero unless `clang -dumpversion`
    prints CLANG_VERSION, so a provisioning failure is loud rather than a
    green gate that compared against the wrong compiler.
    """
    root = prefix / f"clang-{CLANG_VERSION}"
    clang = root / "bin/clang"
    installer = REPO / "scripts" / "ci_install_clang_oracle.sh"
    if not installer.is_file():
        raise RuntimeError(f"missing Clang installer: {installer}")
    # The asset is >1 GiB and the installer refuses a TMPDIR too small to hold
    # it; /tmp is a small tmpfs in this harness, so point the download at the
    # disk-backed prefix instead of letting curl die with a write error.
    downloads = prefix / "dl"
    downloads.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, TMPDIR=str(downloads),
               LCCC_CLANG_ORACLE_PREFIX=str(root))
    run(["bash", str(installer), "--prefix", str(root)], env=env, timeout=3600)
    if not clang.is_file():
        raise RuntimeError(f"Clang installer succeeded but {clang} is missing")
    version = run([str(clang), "--version"])
    if CLANG_VERSION not in version:
        raise RuntimeError(f"unexpected Clang identity: {version}")
    print(f"[arena] clang: {version.splitlines()[0]}")
    return clang


def install_gdb(prefix: Path) -> Path:
    """Provision the pinned GDB oracle (18.1, built with -march=haswell).

    Opt-in (`--with-gdb`) because it is a from-source build of several
    minutes, and only the DWARF/debug-info gates need it -- but when those
    gates run without a gdb on PATH they SKIP their debugger cross-check and
    still exit 0, so a session that touches debug info must ask for it.
    """
    root = prefix / f"gdb-{GDB_VERSION}"
    gdb = root / "bin/gdb"
    installer = REPO / "scripts" / "ci_install_gdb_oracle.sh"
    if not installer.is_file():
        raise RuntimeError(f"missing GDB installer: {installer}")
    downloads = prefix / "dl"
    downloads.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, TMPDIR=str(downloads),
               LCCC_GDB_ORACLE_PREFIX=str(root))
    run(["bash", str(installer), "--prefix", str(root)], env=env, timeout=7200)
    if not gdb.is_file():
        raise RuntimeError(f"GDB installer succeeded but {gdb} is missing")
    print(f"[arena] gdb: {run([str(gdb), '--version']).splitlines()[0]}")
    return gdb


def verify(gcc: Path, rust: Path) -> None:
    with tempfile.TemporaryDirectory(prefix="lccc-probe-") as td:
        probe = Path(td) / "p.c"
        probe.write_text("int main(void) { return 0; }\n")
        for extra, label in (([], "LP64"), (["-m32"], "ILP32")):
            binary = Path(td) / ("p64" if not extra else "p32")
            run([str(gcc), *extra, str(probe), "-o", str(binary)], timeout=120)
            run([str(binary)], timeout=30)
            print(f"[arena] gcc-16 {label}: compile+link+run OK")


def provision_corpus() -> None:
    if (CORPUS / "gcc/BASE-VER").is_file():
        print(f"[arena] GCC 17 corpus already present at {CORPUS}")
        return
    run([sys.executable, str(REPO / "scripts/ensure_gcc_torture_git.py"),
         "--dest", str(CORPUS), "--ref", GCC17_REF], timeout=1200)
    print(f"[arena] GCC 17 corpus: {(CORPUS / 'gcc/BASE-VER').read_text().strip()}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--prefix", type=Path, default=DEFAULT_PREFIX)
    parser.add_argument("--verify-only", action="store_true",
                        help="probe an existing install; never download or write")
    parser.add_argument("--skip-corpus", action="store_true")
    parser.add_argument("--with-gdb", action="store_true",
                        help="also build the pinned GDB oracle (needed by the "
                             "DWARF line-table gates, several minutes)")
    args = parser.parse_args(argv)
    prefix = args.prefix.resolve()
    gcc = prefix / "gcc-16.2/gcc-16/bin/gcc-16"
    rust = prefix / f"rust-{RUST_VERSION}"
    clang = prefix / f"clang-{CLANG_VERSION}/bin/clang"
    if args.verify_only:
        if not gcc.is_file() or not (rust / "bin/rustc").is_file() or not clang.is_file():
            print("[arena] toolchain prefix is incomplete", file=sys.stderr)
            return 1
        print(run([str(rust / "bin/rustc"), "--version"]))
        print(run([str(gcc), "-dumpfullversion"]))
        print(run([str(clang), "--version"]).splitlines()[0])
        verify(gcc, rust)
        return 0
    prefix.mkdir(parents=True, exist_ok=True)
    install_gcc(prefix)
    install_rust(prefix)
    install_multilib(prefix)
    install_clang(prefix)
    if args.with_gdb:
        install_gdb(prefix)
    verify(gcc, rust)
    if not args.skip_corpus:
        provision_corpus()
    print(f"[arena] ready: PATH={rust / 'bin'}:{gcc.parent}:$PATH")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"[arena] FATAL: {error}", file=sys.stderr)
        raise SystemExit(1)
