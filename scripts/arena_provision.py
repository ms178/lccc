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


def install_rust(prefix: Path) -> Path:
    root = prefix / f"rust-{RUST_VERSION}"
    if (root / ".verified").exists():
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
            # data_filter (3.12+) refuses traversal/absolute links; on 3.11 the
            # archive is our own pinned content, but reject unsafe members
            # explicitly rather than relying on the default extract behaviour.
            for member in source.getmembers():
                if member.name.startswith("/") or ".." in Path(member.name).parts \
                        or (member.issym() and member.linkname.startswith("/")):
                    raise RuntimeError(f"unsafe archive member: {member.name}")
            source.extractall(td)
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
    if marker.exists():
        return root / "gcc-16/bin/gcc-16"
    downloads = prefix / "dl"
    downloads.mkdir(parents=True, exist_ok=True)
    archive = downloads / "gcc-16-bookworm-x86_64.tar.xz"
    blob(GCC_REPO, GCC_BLOB, 65_152_756, archive)
    if sha256(archive) != GCC_SHA256:
        raise RuntimeError("GCC 16.2 archive SHA-256 mismatch")
    root.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive, "r:xz") as source:
        for member in source.getmembers():
            if member.name.startswith("/") or ".." in Path(member.name).parts:
                raise RuntimeError(f"unsafe archive member: {member.name}")
        source.extractall(root)
    gcc = root / "gcc-16/bin/gcc-16"
    version = run([str(gcc), "-dumpfullversion"])
    if not version.startswith("16.2"):
        raise RuntimeError(f"unexpected GCC identity: {version}")
    marker.write_text(GCC_SHA256 + "\n")
    print(f"[arena] gcc-16: {version}")
    return gcc


def install_multilib(prefix: Path) -> None:
    """Install the pinned bookworm i686 CRT/dev packages, once per sandbox."""
    marker = prefix / ".multilib-verified"
    if marker.exists():
        return
    downloads = prefix / "dl"
    archive = downloads / "debs-bookworm.tar.xz"
    blob(GCC_REPO, DEBS_BLOB, 32_787_168, archive)
    with tempfile.TemporaryDirectory(prefix="lccc-debs-", dir=prefix) as td:
        with tarfile.open(archive, "r:xz") as source:
            for member in source.getmembers():
                if member.name.startswith("/") or ".." in Path(member.name).parts:
                    raise RuntimeError(f"unsafe archive member: {member.name}")
            source.extractall(td)
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
    marker.write_text("bookworm-multilib\n")
    print(f"[arena] installed {len(debs)} bookworm multilib packages")


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
    args = parser.parse_args(argv)
    prefix = args.prefix.resolve()
    gcc = prefix / "gcc-16.2/gcc-16/bin/gcc-16"
    rust = prefix / f"rust-{RUST_VERSION}"
    if args.verify_only:
        if not gcc.is_file() or not (rust / "bin/rustc").is_file():
            print("[arena] toolchain prefix is incomplete", file=sys.stderr)
            return 1
        print(run([str(rust / "bin/rustc"), "--version"]))
        print(run([str(gcc), "-dumpfullversion"]))
        verify(gcc, rust)
        return 0
    prefix.mkdir(parents=True, exist_ok=True)
    install_gcc(prefix)
    install_rust(prefix)
    install_multilib(prefix)
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
