#!/usr/bin/env python3
"""Physical-path C source selection for *first-party* compiler gates.

The imported tests/corpus subtree has its own execution contract. It must not
silently retarget a broad first-party invariance/differential gate. Symlinked
roots and symlinked C files resolve to the same physical policy; directory
symlinks are not traversed. Sampling is applied AFTER exclusion.
"""
from __future__ import annotations

import argparse
import os
from pathlib import Path
import sys

REPO = Path(__file__).resolve().parent.parent


def source_paths(root: Path, *, repo: Path = REPO, recursive: bool = True) -> list[Path]:
    root=root.resolve(strict=True); excluded=(repo/'tests/corpus').resolve()
    if not root.is_dir():raise ValueError(f'not a directory: {root}')
    def curated(p):return p==excluded or excluded in p.parents
    if curated(root):return []
    result=set()
    def error(exc):raise exc
    for directory, dirs, names in os.walk(root, followlinks=False, onerror=error):
        here=Path(directory)
        dirs[:]=sorted(d for d in dirs if not curated((here/d).resolve()) and not (here/d).is_symlink())
        for name in names:
            if not name.endswith('.c'):continue
            physical=(here/name).resolve(strict=True)
            if not curated(physical) and physical.is_file():result.add(physical)
        if not recursive:break
    return sorted(result, key=lambda p: os.fsencode(str(p)))


def main(argv=None):
    ap=argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('root',nargs='?',type=Path,default=REPO/'tests')
    ap.add_argument('--every',type=int,default=1)
    ap.add_argument('--null',action='store_true')
    args=ap.parse_args(argv)
    if args.every<1:ap.error('--every must be >= 1')
    try: paths=source_paths(args.root)[::args.every]
    except (OSError,ValueError) as exc:ap.exit(2,f'corpus selection: {exc}\n')
    if not args.null and any('\n' in str(p) or '\r' in str(p) for p in paths):
        ap.exit(2,'corpus selection: newline filenames require --null\n')
    delim=b'\0' if args.null else b'\n'
    for p in paths:sys.stdout.buffer.write(os.fsencode(str(p))+delim)
    return 0


if __name__=='__main__':raise SystemExit(main())
