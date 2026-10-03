#!/usr/bin/env python3
"""Restore the compact snapshot without toolchain installation or compilation.

Exact history uses lccc-session.bundle plus BASE (fetched, locally available,
or reconstructed OFFLINE from the verified source/archive/cumulative patch
and recorded raw BASE commit). If that chain is unavailable, a verified source
fallback explicitly reports exact_history=false, never inventing commit IDs.
Existing worktrees are untouched; --preserve-worktree installs .git only.
"""
from __future__ import annotations

import argparse
import base64
import hashlib
import inspect
import json
import os
from pathlib import Path,PurePosixPath
import re
import shutil
import subprocess
import tarfile
import tempfile

SHA=re.compile(r'^[0-9a-f]{40}$');DIGEST=re.compile(r'^[0-9a-f]{64}$')
UPSTREAM='https://github.com/ms178/lccc.git'


def git(repo,*args,input=None):
    p=subprocess.run(['git','-C',str(repo),*args],input=input,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=120)
    if p.returncode:raise ValueError('git '+args[0]+': '+p.stderr.decode('utf-8',errors='replace')[-2000:])
    return p.stdout.decode().strip()


def sha(path):
    h=hashlib.sha256()
    with Path(path).open('rb') as f:
        for block in iter(lambda:f.read(1048576),b''):h.update(block)
    return h.hexdigest()


def require(ok,msg):
    if not ok:raise ValueError(msg)


def extractall_takes_filter():
    """True when tarfile's extractall accepts PEP 706's filter= keyword.

    filter= landed in 3.12 and 3.11.4, but docs/getting-started.md declares
    Python 3.9+ as the floor, so it cannot be passed unconditionally.  Probe
    the signature instead of catching TypeError around the call: a bare
    `except TypeError` also swallows one raised from INSIDE extraction and
    then silently continues unfiltered, which is a worse outcome than the
    crash it was hiding.  Verified True on 3.12.3/3.12.15/3.13.14 and False
    for a simulated legacy signature.
    """
    try:
        return 'filter' in inspect.signature(tarfile.TarFile.extractall).parameters
    except (TypeError,ValueError):
        return False


def read_json(path):
    def pairs(values):
        r={}
        for k,v in values:
            require(k not in r,'duplicate recovery metadata key');r[k]=v
        return r
    def bad(value):raise ValueError('nonfinite recovery JSON: '+value)
    return json.loads(Path(path).read_text(),object_pairs_hook=pairs,parse_constant=bad)


def name(value):
    require(isinstance(value,str) and value and '/' not in value and '\\' not in value and value not in {'.','..'} and '\0' not in value,'unsafe artifact basename')
    return value


def digest(value,pattern):require(isinstance(value,str) and pattern.fullmatch(value) is not None,'invalid recovery identity')


def recipe_metadata(path):
    r=read_json(path);require(r['format']=='lccc-source-archive-v2','unknown source archive recipe')
    for f in ('head','tree','base'):digest(r[f],SHA)
    for f in ('archive_sha256','patch_sha256'):digest(r[f],DIGEST)
    name(r['archive']);name(r['patch']);require(r['prefix']=='lccc/','unknown source archive prefix')
    require(isinstance(r['branch'],str) and not r['branch'].startswith('-'),'unsafe branch')
    raw=base64.b64decode(r['base_commit_base64'],validate=True)
    require(len(raw)<=65536 and hashlib.sha1(b'commit '+str(len(raw)).encode()+b'\0'+raw).hexdigest()==r['base'],'BASE commit identity mismatch')
    return r


def bundle_metadata(path):
    r=read_json(path);require(r['format']=='lccc-session-bundle-v1','unknown session bundle format')
    for f in ('head','tree','base'):digest(r[f],SHA)
    digest(r['bundle_sha256'],DIGEST);name(r['bundle'])
    require(isinstance(r['branch'],str) and not r['branch'].startswith('-'),'unsafe branch')
    return r


def extract(archive,stage,r):
    require(Path(archive).is_file() and not Path(archive).is_symlink() and sha(archive)==r['archive_sha256'],'source archive hash mismatch/missing')
    with tarfile.open(archive,'r:gz') as t:
        members=t.getmembers();require(len(members)<=20000 and sum(m.size for m in members)<=512*1024*1024,'source archive extraction budget exceeded')
        seen=set()
        for member in members:
            p=PurePosixPath(member.name)
            require(not p.is_absolute() and '..' not in p.parts and p.parts and p.parts[0]=='lccc','escaping archive member')
            require('.git' not in p.parts and '\\' not in member.name and '\0' not in member.name,'unsafe archive member')
            require(member.name not in seen,'duplicate archive member');seen.add(member.name)
            require(member.isfile() or member.isdir() or member.issym(),'unsupported archive entry')
            # Everything above validates the member NAME and never the symlink
            # TARGET, so on its own this loop accepts `lccc/evil ->
            # ../../../../etc/passwd`: verified -- the loop returns no verdict
            # for that member and only tarfile's data filter refuses it
            # (LinkOutsideDestinationError), while an unfiltered extractall
            # happily creates the escaping link.  The guarantee therefore cannot
            # rest on `filter='data'`, which is 3.12+/3.11.4+ against the
            # Python 3.9+ floor docs/getting-started.md declares.  Reject the
            # escaping target and the special mode bits here instead, so an old
            # interpreter loses redundancy rather than protection.  Stricter
            # than the data filter (which permits a `..` that resolves back
            # inside), which costs nothing: the snapshot archive this restores
            # carries 5620 members, zero links and zero special-bit modes.
            if member.issym():
                link=PurePosixPath(member.linkname)
                require(not link.is_absolute() and '..' not in link.parts,'escaping archive link target')
            require(not member.mode & 0o7000,'setuid/setgid/sticky archive member')
        # PEP 706 data filter where the interpreter provides it; the loop above
        # is the version-independent guarantee this falls back on.
        if extractall_takes_filter():
            t.extractall(stage,filter='data')
        else:
            t.extractall(stage)
    repo=stage/'lccc';require(repo.is_dir(),'source root missing');return repo


def init(repo,branch):
    repo.mkdir(parents=True,exist_ok=True);git(repo,'init','-q');git(repo,'check-ref-format','refs/heads/'+branch)
    git(repo,'symbolic-ref','HEAD','refs/heads/'+branch)
    git(repo,'config','user.name','LCCC source recovery');git(repo,'config','user.email','recovery@lccc.local')


def source_tree(repo,r):
    git(repo,'add','-f','-A');tree=git(repo,'write-tree');require(tree==r['tree'],'source archive does not reproduce the recorded Git tree')


def offline_base(stage,artifacts,r,*,patch=None):
    repo=extract(artifacts/r['archive'],stage,r);init(repo,r['branch']);source_tree(repo,r)
    patch=Path(patch) if patch is not None else artifacts.parent/r['patch']
    require(patch.is_file() and not patch.is_symlink() and sha(patch)==r['patch_sha256'],'cumulative patch identity mismatch/missing')
    git(repo,'apply','--reverse','--check',str(patch.resolve()))
    git(repo,'apply','--reverse',str(patch.resolve()));git(repo,'add','-f','-A')
    raw=base64.b64decode(r['base_commit_base64'],validate=True);base_tree=raw.splitlines()[0].decode().split(' ')
    require(len(base_tree)==2 and base_tree[0]=='tree' and git(repo,'write-tree')==base_tree[1],'reverse patch did not reconstruct BASE tree')
    value=git(repo,'hash-object','-t','commit','-w','--stdin',input=raw);require(value==r['base'],'reconstructed BASE commit mismatch')
    # Missing BASE parents are deliberate, like a depth=1 upstream fetch. No
    # commit identity is rewritten; fsck knows this authenticated shallow edge.
    (repo/'.git/shallow').write_text(r['base']+'\n')
    git(repo,'update-ref','refs/heads/'+r['branch'],r['base'])
    return repo


def exact_restore(stage,artifacts,b,recipe,upstream,offline,patch):
    bundle=artifacts/b['bundle'];require(bundle.is_file() and not bundle.is_symlink() and sha(bundle)==b['bundle_sha256'],'session bundle identity mismatch/missing')
    require(recipe is None or (recipe['head'],recipe['tree'],recipe['base'],recipe['branch'])==(b['head'],b['tree'],b['base'],b['branch']),'bundle/source generations differ; use latest verified source fallback')
    method='upstream-base';repo=stage/'lccc';network_error=None
    if not offline:
        init(repo,b['branch'])
        try:git(repo,'fetch','-q','--depth=1','--no-tags',upstream,b['base'])
        except (OSError,ValueError,subprocess.SubprocessError) as exc:network_error=str(exc);shutil.rmtree(repo)
    if offline or network_error:
        require(recipe is not None,'offline exact history requires the source/patch/BASE recipe')
        repo=offline_base(stage,artifacts,recipe,patch=patch);method='offline-reconstructed-base'
    git(repo,'bundle','verify',str(bundle.resolve()))
    git(repo,'fetch','-q',str(bundle.resolve()),'refs/heads/'+b['branch'])
    git(repo,'checkout','-q','-B',b['branch'],'FETCH_HEAD')
    require(git(repo,'rev-parse','HEAD')==b['head'] and git(repo,'rev-parse','HEAD^{tree}')==b['tree'],'session history restore mismatch')
    git(repo,'fsck','--full','--no-reflogs')
    return repo,dict(mode=method,exact_history=True,head=b['head'],tree=b['tree'],branch=b['branch'],base=b['base'],upstream_fetch_error=network_error)


def recover(repo,artifacts,*,upstream=UPSTREAM,offline=False,preserve_worktree=False,patch=None,allow_source_fallback=True):
    repo=Path(repo).absolute();artifacts=Path(artifacts).absolute()
    require(not repo.is_symlink(),'destination repository must not be a symlink')
    require(not (repo/'.git').exists(),'Git metadata already exists; recovery never overwrites it')
    require(not repo.exists() or repo.is_dir(),'destination is not a directory')
    require(not repo.exists() or not any(repo.iterdir()) or preserve_worktree,'nonempty worktree requires --preserve-worktree')
    repo.parent.mkdir(parents=True,exist_ok=True)
    recipe=recipe_metadata(artifacts/'SOURCE_ARCHIVE.json') if (artifacts/'SOURCE_ARCHIVE.json').is_file() else None
    b=bundle_metadata(artifacts/'SESSION_BUNDLE.json') if (artifacts/'SESSION_BUNDLE.json').is_file() else None
    with tempfile.TemporaryDirectory(prefix='.lccc-recover-',dir=repo.parent) as td:
        stage=Path(td);error=None
        try:
            if b is not None:restored,result=exact_restore(stage,artifacts,b,recipe,upstream,offline,patch)
            elif (artifacts/'lccc.bundle').is_file():
                legacy=stage/'lccc';git(stage,'clone','-q',str((artifacts/'lccc.bundle').resolve()),str(legacy))
                restored=legacy;result=dict(mode='legacy-full-bundle',exact_history=True,head=git(legacy,'rev-parse','HEAD'),tree=git(legacy,'rev-parse','HEAD^{tree}'),branch=git(legacy,'branch','--show-current'))
            else:raise ValueError('no session/legacy bundle available')
        except (OSError,ValueError,subprocess.SubprocessError) as exc:
            error=str(exc)
            require(allow_source_fallback and recipe is not None,error+'; verified source fallback unavailable/disallowed')
            shutil.rmtree(stage/'lccc',ignore_errors=True)
            restored=extract(artifacts/recipe['archive'],stage,recipe);init(restored,recipe['branch']);source_tree(restored,recipe)
            git(restored,'commit','-qm','Offline source snapshot recovery (not original history)')
            result=dict(mode='verified-source-fallback',exact_history=False,original_head=recipe['head'],head=git(restored,'rev-parse','HEAD'),
                        tree=recipe['tree'],branch=recipe['branch'],history_restore_error=error,
                        warning='source is complete; original commits/BASE ancestry unavailable: restore exact bundle before rebasing/snapshotting')
        git(restored,'remote','add','origin',upstream) if not git(restored,'remote') else git(restored,'remote','set-url','origin',upstream)
        if preserve_worktree and repo.exists():
            os.replace(restored/'.git',repo/'.git');result['worktree_preserved']=True
        else:
            os.replace(restored,repo);result['worktree_preserved']=False
        require(git(repo,'rev-parse','HEAD')==result['head'],'published Git metadata identity mismatch')
        return result


def main(argv=None):
    ap=argparse.ArgumentParser(description=__doc__.splitlines()[0]);ap.add_argument('--repo',type=Path,required=True)
    ap.add_argument('--artifacts',type=Path,default=Path('/home/user/artifacts'));ap.add_argument('--upstream',default=UPSTREAM)
    ap.add_argument('--offline',action='store_true');ap.add_argument('--preserve-worktree',action='store_true');ap.add_argument('--patch',type=Path)
    ap.add_argument('--require-exact-history',action='store_true');args=ap.parse_args(argv)
    try:r=recover(args.repo,args.artifacts,upstream=args.upstream,offline=args.offline,preserve_worktree=args.preserve_worktree,patch=args.patch,allow_source_fallback=not args.require_exact_history)
    except (OSError,ValueError,KeyError,subprocess.SubprocessError,tarfile.TarError) as exc:
        print('recovery FAILED; destination/user worktree preserved: '+str(exc),file=__import__('sys').stderr);return 1
    print(json.dumps(r,sort_keys=True,allow_nan=False));return 0


if __name__=='__main__':raise SystemExit(main())
