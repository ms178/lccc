#!/usr/bin/env python3
"""Small exact-history session bundles with an explicit upstream prerequisite.

Unlike --all, this saves only BASE..HEAD. The original commit IDs are preserved;
no grafts, rewritten root commits or silently uncloneable 'full' bundles. A
fresh, isolated BASE checkout verifies that the bundle carries *every* session
object needed to restore HEAD. It is not a standalone full-history backup: the
tracked-source archive remains the offline source recovery path.

One shape needs BASE's ancestry rather than BASE alone: when the session range
*merges* upstream (the normal case after re-anchoring onto a newer main), git
records the old fork point as a second prerequisite beside BASE, and a
`--depth=1` BASE cannot satisfy it — the proof has to fall back to a full BASE
fetch.  The receipt says which path proved the bundle (`base_fetch`).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile


def git(repo: Path, *args: str) -> str:
    return subprocess.check_output(['git', '-C', str(repo), *args], stderr=subprocess.STDOUT,
                                   text=True, timeout=120).strip()


class BundlePrerequisiteError(ValueError):
    """`git bundle verify` rejected the bundle against the fetched BASE."""


def git_try(repo: Path, *args: str) -> tuple[int, str]:
    """Run git without raising, so a *verification* failure stays local.

    `git bundle verify` reporting missing prerequisites is the one expected
    failure mode here and it must not be confused with a broken bundle.
    """
    proc = subprocess.run(['git', '-C', str(repo), *args], stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, text=True, timeout=120)
    return proc.returncode, proc.stdout.strip()


def sha(path: Path) -> str:
    h=hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda:f.read(1048576), b''): h.update(block)
    return h.hexdigest()


def build(repo: Path, base: str, destination: Path) -> dict:
    base=git(repo, 'rev-parse', '--verify', base+'^{commit}')
    head=git(repo, 'rev-parse', 'HEAD')
    branch=git(repo, 'symbolic-ref', '--short', 'HEAD')
    git(repo, 'merge-base', '--is-ancestor', base, head)
    if base==head:
        raise ValueError('empty session cannot produce a nontrivial bundle')
    destination.parent.mkdir(parents=True, exist_ok=True)
    fd,name=tempfile.mkstemp(prefix='.session-bundle-', suffix='.tmp', dir=destination.parent)
    os.close(fd); temp=Path(name)
    try:
        git(repo, 'bundle', 'create', str(temp), f'{base}..{head}', f'refs/heads/{branch}')

        def prove(isolated: Path, shallow: bool) -> str:
            """Restore HEAD in a database that has BASE and nothing of ours.

            Reusing a worktree would share HEAD's objects and mask missing
            ones, so this is a fresh repository that only ever fetches BASE
            from the original and the bundle itself.
            """
            git(isolated, 'init', '-q')
            fetch = ['--depth=1'] if shallow else []
            git(isolated, 'fetch', '-q', *fetch, '--no-tags', str(repo), base)
            code, out = git_try(isolated, 'bundle', 'verify', str(temp))
            if code:
                raise BundlePrerequisiteError(out[-2000:])
            git(isolated, 'fetch', '-q', str(temp), f'refs/heads/{branch}')
            git(isolated, 'checkout', '-q', '-B', branch, 'FETCH_HEAD')
            if git(isolated, 'rev-parse', 'HEAD')!=head:
                raise ValueError('bundle restored the wrong HEAD')
            if git(isolated, 'rev-parse', 'HEAD^{tree}')!=git(repo, 'rev-parse', 'HEAD^{tree}'):
                raise ValueError('bundle restored the wrong tree')
            git(isolated, 'fsck', '--full', '--no-reflogs')
            return 'shallow' if shallow else 'base-with-ancestry'

        # Cheapest proof first: BASE alone.  A range that merges upstream needs
        # BASE's ancestry (see the module docstring), so that failure is
        # retried once against a full BASE fetch before it is reported.
        base_fetch = 'shallow'
        try:
            with tempfile.TemporaryDirectory(prefix='lccc-bundle-verify-', dir=repo.parent) as td:
                base_fetch = prove(Path(td), shallow=True)
        except BundlePrerequisiteError:
            with tempfile.TemporaryDirectory(prefix='lccc-bundle-verify-', dir=repo.parent) as td:
                base_fetch = prove(Path(td), shallow=False)
        with temp.open('rb') as f:os.fsync(f.fileno())
        temp.chmod(0o644);os.replace(temp, destination)
        return dict(format='lccc-session-bundle-v1', base=base, head=head, branch=branch,
                    bundle=destination.name, bundle_sha256=sha(destination),
                    tree=git(repo,'rev-parse','HEAD^{tree}'),
                    requires_upstream_base=True, base_fetch=base_fetch,
                    isolated_restore_verified=True)
    finally:
        temp.unlink(missing_ok=True)


def main(argv=None):
    ap=argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--repo',type=Path,required=True);ap.add_argument('--base',required=True)
    ap.add_argument('--output',type=Path,required=True)
    args=ap.parse_args(argv)
    try:result=build(args.repo,args.base,args.output)
    except (ValueError,OSError,subprocess.SubprocessError) as exc:
        ap.exit(1,f'session bundle FAILED (previous bundle preserved): {exc}\n'+str(getattr(exc,'output',''))[-2000:])
    print(json.dumps(result,sort_keys=True,allow_nan=False))
    return 0


if __name__=='__main__':raise SystemExit(main())
