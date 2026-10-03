#!/usr/bin/env python3
"""Small exact-history session bundles with an explicit upstream prerequisite.

Unlike --all, this saves only BASE..HEAD. The original commit IDs are preserved;
no grafts, rewritten root commits or silently uncloneable 'full' bundles. A
fresh, isolated shallow BASE checkout verifies that the bundle carries *every*
session object needed to restore HEAD. It is not a standalone full-history
backup: the tracked-source archive remains the offline source recovery path.
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
        # Fetch ONLY BASE and its tree into an independent object database.
        # Reusing a worktree would share HEAD's objects and mask missing ones.
        with tempfile.TemporaryDirectory(prefix='lccc-bundle-verify-', dir=repo.parent) as td:
            isolated=Path(td)
            git(isolated, 'init', '-q')
            git(isolated, 'fetch', '-q', '--depth=1', '--no-tags', str(repo), base)
            git(isolated, 'bundle', 'verify', str(temp))
            git(isolated, 'fetch', '-q', str(temp), f'refs/heads/{branch}')
            git(isolated, 'checkout', '-q', '-B', branch, 'FETCH_HEAD')
            if git(isolated, 'rev-parse', 'HEAD')!=head:
                raise ValueError('bundle restored the wrong HEAD')
            if git(isolated, 'rev-parse', 'HEAD^{tree}')!=git(repo, 'rev-parse', 'HEAD^{tree}'):
                raise ValueError('bundle restored the wrong tree')
            git(isolated, 'fsck', '--full', '--no-reflogs')
        with temp.open('rb') as f:os.fsync(f.fileno())
        temp.chmod(0o644);os.replace(temp, destination)
        return dict(format='lccc-session-bundle-v1', base=base, head=head, branch=branch,
                    bundle=destination.name, bundle_sha256=sha(destination),
                    tree=git(repo,'rev-parse','HEAD^{tree}'),
                    requires_upstream_base=True, isolated_restore_verified=True)
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
