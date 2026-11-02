#!/usr/bin/env bash
set -euo pipefail
ROOT=/home/user/.cache/valgrind-bootstrap
PREFIX=/home/user/.local/gnu-bootstrap
mkdir -p "$ROOT" "$PREFIX"
export PATH="$PREFIX/bin:/usr/bin:/bin"
cd "$ROOT"
python3 - <<'PY'
import hashlib, io, pathlib, posixpath, tarfile, urllib.request
items = [('m4','Distrotech/m4','f1a9aff625f45be1a596048f9e2b4d8211e4c855'),
         ('autoconf','Distrotech/autoconf','9bc20f95c7243d3968152b8343ff135325b0ba8b'),
         ('automake','Distrotech/automake','2b9392c876851043af83feef21381a3199c4b5a5'),
         ('valgrind','tklengyel/valgrind','971c165fc4335bf04bedf58e0800a12776fce408')]
for name,repo,sha in items:
 dest=pathlib.Path(name)
 if dest.exists():continue
 data=urllib.request.urlopen(f'https://codeload.github.com/{repo}/tar.gz/{sha}',timeout=120).read()
 print(name,sha,hashlib.sha256(data).hexdigest(),flush=True)
 with tarfile.open(fileobj=io.BytesIO(data)) as a:
  for m in a.getmembers():
   parts=pathlib.PurePosixPath(m.name).parts
   if m.name.startswith('/') or '..' in parts or not (m.isfile() or m.isdir() or m.issym() or m.islnk()):raise ValueError(m.name)
   if m.issym() or m.islnk():
    target = m.linkname if m.islnk() else str(pathlib.PurePosixPath(m.name).parent / m.linkname)
    if m.linkname.startswith('/') or pathlib.PurePosixPath(posixpath.normpath(target)).parts[0] != parts[0]:raise ValueError((m.name,m.linkname))
  a.extractall('.')
 pathlib.Path(repo.split('/')[1]+'-'+sha).rename(dest)
PY
# Bootstrap-only compatibility for GNU m4's bundled old gnulib and glibc 2.36.
# No Valgrind instruction/counter implementation changes are made.
python3 - <<'PY'
from pathlib import Path
for p in Path('m4/lib').glob('*.c'):
 s=p.read_text();s=s.replace('defined _IO_ftrylockfile','defined _IO_EOF_SEEN')
 p.write_text(s)
p=Path('m4/lib/stdio-impl.h');s=p.read_text();p.write_text('#ifndef _IO_IN_BACKUP\n#define _IO_IN_BACKUP 0x100\n#endif\n'+s)
p=Path('m4/lib/c-stack.c');s=p.read_text();s=s.replace('#include <signal.h>', '#include <signal.h>\n/* Bootstrap-only fixed alternate-stack capacity for modern glibc. */\n#undef SIGSTKSZ\n#define SIGSTKSZ (64 * 1024)');p.write_text(s)
PY
(cd m4; if [[ ! -f Makefile ]]; then ./configure --prefix="$PREFIX" > configure.log 2>&1; fi; make -j1 > build.log 2>&1; make install > install.log 2>&1)
(cd autoconf; if [[ ! -f Makefile ]]; then ./configure --prefix="$PREFIX" > configure.log 2>&1; fi; make -j1 > build.log 2>&1; make install > install.log 2>&1)
# Archived generated Makefiles can predate generated include timestamps. Build
# the bootstrap tools without requiring those same not-yet-built tools first.
(cd automake; if [[ ! -f Makefile ]]; then ./configure --prefix="$PREFIX" > configure.log 2>&1; fi; make -o Makefile.in -o Makefile -o configure -j1 > build.log 2>&1; make -o Makefile.in -o Makefile -o configure install > install.log 2>&1)
m4 --version | head -1
autoconf --version | head -1
automake --version | head -1
(cd valgrind; ./autogen.sh > autogen.log 2>&1; ./configure --prefix=/home/user/.local/valgrind --enable-only64bit > configure.log 2>&1; make -j1 > build.log 2>&1; make install > install.log 2>&1)
/home/user/.local/valgrind/bin/valgrind --version
