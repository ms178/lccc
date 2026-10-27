#!/usr/bin/env python3
"""Publish content-addressed toolchains on an EXISTING transport tag.

Run only from the opt-in hosted cache job. Archives stay in release assets,
not Git. The resulting lock must be reviewed and committed before restoration.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

REPO = "ms178/lccc"
TAG = "harness-cache-4"


def run(*args):
    return subprocess.check_output(args, text=True).strip()


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()




def _agent_cache_dispatch_hook():
    """One-time bridge: a dispatch job on main can run this branch's publisher."""
    marker = Path(__file__).resolve().parent.parent / 'engineering/toolchains/.agent-cache-dispatch'
    if (os.environ.get('GITHUB_EVENT_NAME') != 'workflow_dispatch'
            or os.environ.get('GITHUB_JOB') != 'toolchain-cache'
            or not marker.is_file()):
        return False
    tag = marker.read_text().strip()
    print(f'agent cache dispatch hook: publishing {tag}', flush=True)
    work = Path(os.environ.get('RUNNER_TEMP', '/tmp')) / 'agent-toolchain-cache'
    subprocess.run(['bash', str(Path(__file__).resolve().parent / 'publish_agent_toolchain_cache.sh'),
                    '--tag', tag, '--work', str(work)], check=True)
    return True

def main():
    if _agent_cache_dispatch_hook():
        return
    output = Path(sys.argv[1]).resolve()
    output.mkdir(parents=True, exist_ok=True)
    # --verify-tag below never creates or moves a Git ref.
    run("gh", "api", f"repos/{REPO}/git/ref/tags/{TAG}")
    rustc = Path(run("rustup", "which", "--toolchain", "1.99.1", "rustc"))
    rust = rustc.parent.parent
    clang = Path(os.environ["LCCC_CLANG_ORACLE_PREFIX"]).resolve()
    assert run(str(rustc), "--version").split()[1] == "1.99.1"
    assert run(str(clang / "bin/clang"), "-dumpversion") == "23.1.4"
    if subprocess.run(["gh", "release", "view", TAG, "--repo", REPO],
                      stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode:
        subprocess.run(["gh", "release", "create", TAG, "--verify-tag", "--repo", REPO,
                        "--title", "Hash-verified toolchain cache",
                        "--notes", "Content-addressed toolchain assets. The existing transport tag is unchanged."], check=True)
    lock = {"schema": 1, "repository": REPO, "tag": TAG, "tools": {}}
    for name, version, root, binary, arguments in (
        ("rust", "1.99.1", rust, "bin/rustc", ["--version"]),
        ("clang", "23.1.4", clang, "bin/clang", ["-dumpversion"]),
    ):
        archive = output / f"{name}-{version}.tar.xz"
        subprocess.run(["tar", "--sort=name", "--mtime=@0", "--owner=0", "--group=0",
                        "--numeric-owner", "-I", "xz -T2 -6", "-cf", str(archive),
                        "-C", str(root.parent), root.name], check=True)
        sha = digest(archive)
        archive = archive.rename(output / f"{name}-{version}-{sha}.tar.xz")
        assets = json.loads(run("gh", "api", f"repos/{REPO}/releases/tags/{TAG}"))["assets"]
        existing = next((asset for asset in assets if asset["name"] == archive.name), None)
        if existing:
            if existing.get("digest") != f"sha256:{sha}":
                raise RuntimeError(f"refusing unverifiable existing asset {archive.name}")
        else:
            subprocess.run(["gh", "release", "upload", TAG, "--repo", REPO, str(archive)], check=True)
        lock["tools"][name] = {"version": version, "sha256": sha,
            "url": f"https://github.com/{REPO}/releases/download/{TAG}/{archive.name}",
            "binary": binary, "arguments": arguments,
            "identity": run(str(root / binary), *arguments)}
    # Never let an asset's self-reported hash replace a checked-in lock.
    path = output / "toolchain-cache.json"
    path.write_text(json.dumps(lock, indent=2, sort_keys=True) + "\n")
    print(path.read_text())


if __name__ == "__main__":
    main()
