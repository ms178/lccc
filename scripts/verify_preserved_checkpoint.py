#!/usr/bin/env python3
"""Offline bridge proof for a preserved checkpoint after an upstream-base change.

Inputs are the independently restored CURRENT repository, preserved recipes,
current bundle-archive inventory and the authenticated old-to-new base patch.
No live audit checkout, remote, compiler or invented commit identity is used.
The output directory must not exist. A failure leaves it for diagnosis.
"""
import argparse
import base64
import hashlib
import json
import lzma
from pathlib import Path
import tarfile

import lccc_recover as recovery


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--restored', type=Path, required=True)
    p.add_argument('--artifacts', type=Path, required=True)
    p.add_argument('--preserved', type=Path, required=True)
    p.add_argument('--out', type=Path, required=True)
    args = p.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    out = args.out.resolve()
    old = recovery.read_json(args.preserved)
    recipe = old['SOURCE_ARCHIVE.json']
    current = recovery.recipe_metadata(args.artifacts / 'SOURCE_ARCHIVE.json')
    bridge = old['base_bridge']
    path = args.preserved.parent / recovery.name(bridge['archive'])
    recovery.require(recovery.sha(path) == bridge['sha256'], 'bridge transport hash')
    patch = lzma.decompress(path.read_bytes())
    recovery.require(hashlib.sha256(patch).hexdigest() == bridge['patch_sha256'], 'bridge patch hash')
    tar = out / 'base.tar'
    recovery.git(args.restored, 'archive', '--format=tar', '--prefix=lccc/',
                 '--output=' + str(tar), current['base'])
    recovery.extract(tar, out, {'archive_sha256': recovery.sha(tar), 'prefix': 'lccc/'})
    repo = out / 'lccc'
    recovery.init(repo, 'preserved-proof')
    # Restoring a tracked tree must include paths such as historical *.log
    # files even when the tree's own .gitignore would hide newly added files.
    recovery.git(repo, 'add', '-f', '-A')
    recovery.git(repo, 'apply', '--reverse', '--index', '-', input=patch)
    raw = base64.b64decode(old['old_base_commit_base64'], validate=True)
    old_base = recovery.git(repo, 'hash-object', '-w', '-t', 'commit', '--stdin', input=raw)
    recovery.require(old_base == recipe['base'], 'old BASE identity')
    tree = recovery.git(repo, 'write-tree')
    recovery.require(tree == raw.decode().splitlines()[0].removeprefix('tree '), 'old BASE tree')
    (repo / '.git/shallow').write_text(old_base + '\n')
    recovery.git(repo, 'update-ref', 'refs/heads/preserved-proof', old_base)
    inventory = recovery.read_json(args.artifacts / 'BUNDLE_ARCHIVE.json')
    archive = args.artifacts / recovery.name(inventory['archive'])
    recovery.require(recovery.sha(archive) == inventory['archive_sha256'], 'bundle archive hash')
    expected = old['SESSION_BUNDLE.json']['bundle_sha256']
    matches = [row for row in inventory['members'] if row['sha256'] == expected]
    recovery.require(len(matches) == 1, 'unique preserved bundle')
    row = matches[0]
    bundle = out / 'preserved.bundle'
    with tarfile.open(archive) as t:
        member = t.getmember(recovery.name(row['name']))
        recovery.require(member.isfile() and member.size == row['bytes'], 'bundle member type/size')
        with t.extractfile(member) as source, bundle.open('wb') as dest:
            import shutil
            shutil.copyfileobj(source, dest)
    recovery.require(recovery.sha(bundle) == expected, 'preserved bundle hash')
    recovery.git(repo, 'bundle', 'verify', str(bundle))
    recovery.git(repo, 'fetch', '-q', str(bundle), 'refs/heads/' + recipe['branch'])
    recovery.git(repo, 'checkout', '-q', '--detach', 'FETCH_HEAD')
    head = recovery.git(repo, 'rev-parse', 'HEAD')
    restored_tree = recovery.git(repo, 'rev-parse', 'HEAD^{tree}')
    recovery.require((head, restored_tree) == (recipe['head'], recipe['tree']), 'preserved identities')
    recovery.require(not recovery.git(repo, 'status', '--porcelain'), 'dirty restored worktree')
    print(json.dumps({'exact_history': True, 'old_base': old_base, 'old_base_tree': tree,
                      'head': head, 'tree': restored_tree, 'bundle_sha256': expected,
                      'network_used': False}, sort_keys=True))
    tar.unlink()
    bundle.unlink()
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
