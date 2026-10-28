#!/usr/bin/env python3
"""Validate Debian data archives and confine extracted sysroot symlinks.

Used by offline_deb_extract.sh before and after unprivileged dpkg -x. This is
not a substitute for authenticating packages: consume only trusted, verified
inputs. It rejects traversal, special files, hardlinks, and any archive member
whose parent is a symlink, even when that symlink comes from another package.
"""
from __future__ import annotations

import argparse
import os
from pathlib import Path, PurePosixPath
import posixpath
import subprocess
import sys
import tarfile


def archive_path(value: str) -> str:
    if value.startswith('/') or '..' in value.split('/'):
        raise ValueError(f'unsafe archive path: {value!r}')
    path = posixpath.normpath(value)
    if path == '..' or path.startswith('../'):
        raise ValueError(f'unsafe archive path: {value!r}')
    return path


def validate_packages(packages: list[str]) -> None:
    kinds: dict[str, set[bytes]] = {}
    symlinks: dict[str, str] = {}
    for package in packages:
        proc = subprocess.Popen(['dpkg-deb', '--fsys-tarfile', package], stdout=subprocess.PIPE)
        assert proc.stdout is not None
        try:
            with tarfile.open(fileobj=proc.stdout, mode='r|*') as archive:
                for member in archive:
                    name = archive_path(member.name)
                    if name == '.' and member.isdir():
                        continue
                    if not (member.isdir() or member.isfile() or member.issym()):
                        raise ValueError(f'{package}: unsupported archive member {member.name!r}')
                    kinds.setdefault(name, set()).add(member.type)
                    if member.issym():
                        target = member.linkname
                        if not target:
                            raise ValueError(f'{package}: empty symlink target for {name!r}')
                        if not target.startswith('/'):
                            resolved = posixpath.normpath(posixpath.join(posixpath.dirname(name), target))
                            if resolved == '..' or resolved.startswith('../'):
                                raise ValueError(f'{package}: symlink escapes sysroot: {name!r} -> {target!r}')
                        if name in symlinks and symlinks[name] != target:
                            raise ValueError(f'{package}: conflicting symlinks at {name!r}')
                        symlinks[name] = target
        except (ValueError, tarfile.TarError):
            proc.kill()
            proc.wait()
            raise
        finally:
            proc.stdout.close()
        if proc.wait() != 0:
            raise ValueError(f'cannot read Debian archive {package!r}')

    for name, member_types in kinds.items():
        if name in symlinks and member_types != {tarfile.SYMTYPE}:
            raise ValueError(f'archive path is both a symlink and another type: {name!r}')
        for parent in PurePosixPath(name).parents:
            if str(parent) in symlinks:
                raise ValueError(f'archive member {name!r} traverses symlink {str(parent)!r}')


def confine_symlinks(root: Path) -> None:
    root = root.resolve(strict=True)
    links: list[Path] = []
    for directory, dirs, files in os.walk(root, followlinks=False):
        for name in dirs + files:
            path = Path(directory) / name
            if path.is_symlink():
                links.append(path)
    # Debian's absolute symlinks are root-relative in an installed system.
    # Convert them to relative links inside the staged sysroot, before checking
    # chains, so using a package never silently falls back to the host's /lib.
    for link in links:
        target = os.readlink(link)
        if os.path.isabs(target):
            inside = root / target.lstrip('/')
            relative = os.path.relpath(inside, link.parent)
            link.unlink()
            link.symlink_to(relative)
    for link in links:
        try:
            target = link.resolve(strict=False)
        except RuntimeError as exc:
            raise ValueError(f'symlink loop at {link}') from exc
        if not target.is_relative_to(root):
            raise ValueError(f'extracted symlink escapes sysroot: {link} -> {target}')


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='action', required=True)
    check = sub.add_parser('check')
    check.add_argument('packages', nargs='+')
    fix = sub.add_parser('confine')
    fix.add_argument('root', type=Path)
    args = parser.parse_args()
    try:
        if args.action == 'check':
            validate_packages(args.packages)
        else:
            confine_symlinks(args.root)
    except (OSError, ValueError, tarfile.TarError) as exc:
        print(f'offline-deb: {exc}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
