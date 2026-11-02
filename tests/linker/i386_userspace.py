"""ELF32/i386 userspace link tests for `lccc-ld -m elf_i386`.

Every case links through the real gcc driver (`gcc -m32 -B<shim>` where the
shim's `ld` is lccc-ld), so the inputs are exactly what a distribution build
hands the linker: crt1/crti/crtbegin(S)/crtend(S)/crtn, libgcc, libgcc_s,
the libc.so linker script, positional -Bstatic/--as-needed/--push-state.

Oracle policy: the system linker (GNU ld via plain `gcc -m32`) links the
same objects, and every behavioural expectation is taken from the reference
build's run rather than hard-coded, so a toolchain difference cannot turn
into a false pass.  Multilib eligibility is probed once, independently, up
front (compile, link AND run a trivial -m32 program with the system
linker).  Only a failed probe may SKIP; after a successful probe a failing
reference build is a FAIL (the fixture or the environment is broken, and
hiding that behind SKIP is how the previous version of this test passed
while testing nothing).  With LCCC_REQUIRE_I386=1 (CI) even the probe
failure is a FAIL.

Binding: every run is repeated lazily and with immediate binding.  `sh()`
in run_linker_tests.py merges the parent environment, so an inherited
LD_BIND_NOW would silently turn the "lazy" run into a second "now" run;
`_run` builds the environment explicitly and removes LD_BIND_NOW (and
LD_PRELOAD) for the lazy case.
"""

import os
import re
import shutil
import struct
import subprocess
import tempfile
import textwrap

# ── ELF32 inspection (program headers only: the dynamic linker never reads
# section headers; the cases that check those use readelf/nm) ─────────────

PT_LOAD, PT_DYNAMIC, PT_NOTE, PT_TLS = 1, 2, 4, 7
PT_GNU_EH_FRAME, PT_GNU_STACK, PT_GNU_RELRO = 0x6474E550, 0x6474E551, 0x6474E552
PT_GNU_PROPERTY = 0x6474E553
NT_GNU_PROPERTY_TYPE_0 = 5
DT = {
    "NEEDED": 1, "PLTRELSZ": 2, "HASH": 4, "STRTAB": 5, "SYMTAB": 6,
    "INIT": 12, "FINI": 13, "SONAME": 14, "RPATH": 15, "SYMBOLIC": 16,
    "TEXTREL": 22, "BIND_NOW": 24, "RUNPATH": 29, "FLAGS": 30,
    "GNU_HASH": 0x6FFFFEF5, "FLAGS_1": 0x6FFFFFFB,
}
DF_SYMBOLIC, DF_TEXTREL, DF_BIND_NOW, DF_STATIC_TLS = 0x2, 0x4, 0x8, 0x10
DF_1_NOW = 0x1


class Elf32:
    def __init__(self, path):
        with open(path, "rb") as f:
            self.d = f.read()
        d = self.d
        if d[:4] != b"\x7fELF" or d[4] != 1:
            raise ValueError(f"{path}: not ELF32")
        (self.e_type, self.e_machine, _, self.e_entry, e_phoff, _, _, _,
         e_phentsize, e_phnum) = struct.unpack_from("<HHIIIIIHHH", d, 16)
        self.phdrs = [struct.unpack_from("<IIIIIIII", d, e_phoff + i * e_phentsize)
                      for i in range(e_phnum)]
        self.dyn = []
        for p in self.phdrs:
            if p[0] == PT_DYNAMIC:
                for off in range(p[1], p[1] + p[4], 8):
                    tag, val = struct.unpack_from("<iI", d, off)
                    if tag == 0:
                        break
                    self.dyn.append((tag & 0xFFFFFFFF, val))

    def ptypes(self):
        return [p[0] for p in self.phdrs]

    def phdr(self, ptype):
        return next((p for p in self.phdrs if p[0] == ptype), None)

    def off_of(self, vaddr):
        for p in self.phdrs:
            if p[0] == PT_LOAD and p[2] <= vaddr < p[2] + p[4]:
                return vaddr - p[2] + p[1]
        raise ValueError(f"vaddr 0x{vaddr:x} is not file-backed")

    def tag(self, name):
        return [v for t, v in self.dyn if t == DT[name]]

    def has(self, name):
        return bool(self.tag(name))

    def dynstr(self, off):
        base = self.off_of(self.tag("STRTAB")[0])
        end = self.d.index(b"\0", base + off)
        return self.d[base + off:end].decode()

    def strs(self, name):
        return [self.dynstr(v) for v in self.tag(name)]

    def flags(self):
        return (self.tag("FLAGS") or [0])[0]

    def flags_1(self):
        return (self.tag("FLAGS_1") or [0])[0]

    def notes(self):
        """(type, name, desc) of every note in every PT_NOTE segment."""
        out = []
        for p in self.phdrs:
            if p[0] != PT_NOTE:
                continue
            off, end = p[1], p[1] + p[4]
            while off + 12 <= end:
                namesz, descsz, ntype = struct.unpack_from("<III", self.d, off)
                name_end = off + 12 + ((namesz + 3) & ~3)
                out.append((ntype, self.d[off + 12:off + 12 + namesz],
                            self.d[name_end:name_end + descsz]))
                off = name_end + ((descsz + 3) & ~3)
        return out

    def properties(self):
        """{pr_type: value} of the one GNU property note, None without one.

        ELF32 property entries are type(4) + datasz(4) + data padded to 4,
        i.e. 12 bytes per x86 bitmask; a 4-byte value maps to an int, any
        other size to (datasz, value).  More than one property note is
        itself malformed (the loader reads only the PT_GNU_PROPERTY one), so
        it is reported as a list of maps for the caller to reject."""
        maps = []
        for ntype, name, desc in self.notes():
            if ntype != NT_GNU_PROPERTY_TYPE_0 or name != b"GNU\0":
                continue
            m, o = {}, 0
            while o + 8 <= len(desc):
                t, sz = struct.unpack_from("<II", desc, o)
                v = int.from_bytes(desc[o + 8:o + 8 + sz], "little")
                m[t] = v if sz == 4 else (sz, v)
                o += 8 + ((sz + 3) & ~3)
            maps.append(m)
        if not maps:
            return None
        return maps[0] if len(maps) == 1 else maps

    def build_id(self):
        for p in self.phdrs:
            if p[0] != PT_NOTE:
                continue
            off, end = p[1], p[1] + p[4]
            while off + 12 <= end:
                namesz, descsz, ntype = struct.unpack_from("<III", self.d, off)
                name_end = off + 12 + ((namesz + 3) & ~3)
                if ntype == 3 and self.d[off + 12:off + 12 + namesz] == b"GNU\0":
                    return self.d[name_end:name_end + descsz]
                off = name_end + ((descsz + 3) & ~3)
        return None


# ── fixtures ─────────────────────────────────────────────────────────────

TLS_LIB = r"""
#include <stdio.h>
__thread int t_gd = 11;                       /* exported, reached with GD */
static __thread int t_ld = 22;                /* local dynamic */
__attribute__((visibility("hidden"))) __thread int t_hid = 33;
__thread char t_big[64] = "x";
__attribute__((tls_model("initial-exec"))) extern __thread int t_ie_ext;
int *addr_gd(void) { return &t_gd; }
int get_ld(void) { return t_ld++; }
int get_hid(void) { return t_hid; }
int get_ie(void) { return t_ie_ext; }             /* static TLS in a DSO */
const char *big(void) { return t_big; }
int prot_fn(void) __attribute__((visibility("protected")));
int prot_fn(void) { return 5; }
int call_prot(void) { return prot_fn(); }
int interpose(void) { return 1; }                /* the executable's wins */
int call_interpose(void) { return interpose(); }
__attribute__((weak)) extern int maybe_missing(void);
int has_missing(void) { return maybe_missing ? 1 : 0; }
static int arr[4] = {1, 2, 3, 4};
int *parr = &arr[2];                              /* R_386_RELATIVE */
int get_parr(void) { return *parr; }
"""

TLS_MAIN = r"""
#include <stdio.h>
#include <pthread.h>
extern __thread int t_gd;
__thread int t_ie_ext = 44;
int *addr_gd(void); int get_ld(void); int get_hid(void); int get_ie(void);
const char *big(void); int call_prot(void); int call_interpose(void);
int has_missing(void); int get_parr(void);
int interpose(void) { return 2; }
static void *thr(void *p) {
    (void)p;
    t_gd = 99;                                   /* a fresh thread's block */
    return (void *)(long)(*addr_gd() == 99 && get_ld() == 22 && t_ie_ext == 44);
}
int main(void) {
    pthread_t th; void *r;
    pthread_create(&th, 0, thr, 0); pthread_join(th, &r);
    int a = t_gd, b = *addr_gd(), c = get_ld(), d = get_ld(), e = get_hid();
    printf("%d %d %d %d %d %s %d %d %d %d %ld %d\n", a, b, c, d, e, big(),
           get_ie(), call_prot(), call_interpose(), has_missing(), (long)r,
           get_parr());
    t_gd = 5;
    printf("%d %p\n", *addr_gd(), (void *)(addr_gd() == &t_gd));
    return 0;
}
"""

TLS_EXE_ONLY = r"""
#include <stdio.h>
#include <stdint.h>
__thread int t_gd = 11;
static __thread int t_ld = 22;
static __thread long long t_ld2 = 5;
__thread char t_bss[100];
__thread char t_al[3] __attribute__((aligned(64)));  /* .tbss > .tdata align */
extern __thread int t_def;
int main(void) {
    t_bss[99] = 7; t_al[2] = 3;
    printf("%d %d %lld %d %d %d %d\n", t_gd, t_ld++, t_ld2, t_bss[99], t_def,
           t_al[2], (int)((uintptr_t)t_al % 64));
    printf("%d\n", t_ld);
    return 0;
}
"""


def _w(td, name, body):
    with open(os.path.join(td, name), "w") as f:
        f.write(textwrap.dedent(body))


class Env:
    """Per-run scratch dir, shim and command helpers."""

    def __init__(self, args, cc):
        self.cc = cc
        # CI points LCCC_I386_SCRATCH_ROOT at a directory it uploads on
        # failure; scratch dirs of passing cases are removed, so whatever
        # survives there is exactly the evidence of the failing cases.
        root = os.environ.get("LCCC_I386_SCRATCH_ROOT") or None
        if root:
            os.makedirs(root, exist_ok=True)
        self.td = tempfile.mkdtemp(prefix="lnk.i386.", dir=root)
        shim = os.path.join(self.td, "shim")
        os.mkdir(shim)
        os.symlink(os.path.join(os.path.dirname(args.lccc), "lccc-ld"),
                   os.path.join(shim, "ld"))
        self.shim = shim

    def cmd(self, argv, cwd=None):
        return subprocess.run(argv, cwd=cwd or self.td, timeout=120,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    def gcc(self, *argv, lccc=True):
        pre = [self.cc, "-m32"] + ([f"-B{self.shim}"] if lccc else [])
        return self.cmd(pre + list(argv))

    def run(self, exe, bind_now, libdir=None, extra_env=None):
        env = {k: v for k, v in os.environ.items()
               if k not in ("LD_BIND_NOW", "LD_PRELOAD", "LD_LIBRARY_PATH")}
        if bind_now:
            env["LD_BIND_NOW"] = "1"
        if libdir:
            env["LD_LIBRARY_PATH"] = libdir
        env.update(extra_env or {})
        return subprocess.run([exe], cwd=self.td, env=env, timeout=60,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    def path(self, *p):
        return os.path.join(self.td, *p)

    def close(self, keep):
        if not keep:
            shutil.rmtree(self.td, ignore_errors=True)


class Fail(Exception):
    pass


def _ok(r, what):
    if r.returncode != 0:
        raise Fail(f"{what}: rc={r.returncode}: {r.stderr.decode(errors='replace')[:600]}")
    return r


def _probe(e):
    """Independent -m32 eligibility: compile, link and run with the system
    linker.  Returns None when eligible, else the reason.

    The probe links -no-pie, the same mode as every -m32 reference link in
    this file, so the probe checks the mode the suite actually runs."""
    _w(e.td, "probe.c", "#include <stdio.h>\nint main(void){puts(\"ok\");return 0;}\n")
    r = e.gcc("-no-pie", "probe.c", "-o", "probe", lccc=False)
    if r.returncode != 0:
        return "gcc -m32 cannot link (no i386 multilib): " + r.stderr.decode()[:200]
    r = e.run(e.path("probe"), False)
    if r.returncode != 0 or r.stdout != b"ok\n":
        return "the host cannot run i386 executables"
    return None


def _expect_same_runs(e, exe_ref, exe_new, libdir_ref, libdir_new, label):
    """Run the reference and the candidate lazily and with immediate binding;
    every run must succeed with output identical to the reference's lazy
    run."""
    want = _ok(e.run(exe_ref, False, libdir_ref), f"{label}: reference run")
    for now in (False, True):
        r = e.run(exe_ref, now, libdir_ref)
        if r.returncode != 0 or r.stdout != want.stdout:
            raise Fail(f"{label}: reference {'now' if now else 'lazy'} run differs")
        r = e.run(exe_new, now, libdir_new)
        if r.returncode != 0 or r.stdout != want.stdout:
            raise Fail(f"{label} ({'LD_BIND_NOW=1' if now else 'lazy'}): rc={r.returncode} "
                       f"out={r.stdout.decode(errors='replace')!r} "
                       f"want={want.stdout.decode()!r} "
                       f"err={r.stderr.decode(errors='replace')[:300]!r}")
    return want.stdout


# ── cases ────────────────────────────────────────────────────────────────

def case_tls_matrix(e):
    """4-way {reference, lccc} executable x {reference, lccc} shared object
    matrix over every i386 TLS access model, plus codegen variants.

    The DSO exercises GD (exported), LD (static), hidden, IE of a variable
    the executable defines (static TLS in a DSO: DF_STATIC_TLS), protected
    and interposed functions, an undefined weak and a RELATIVE pointer.  The
    executable reaches the DSO's TLS from a fresh thread too, so a
    link-time-constant offset would be caught.  Executable variants cover
    IE (non-PIC), GD->IE (-fPIC), GD->IE with an indirect ___tls_get_addr
    call (-fno-plt) and TLS descriptors (-mtls-dialect=gnu2)."""
    _w(e.td, "tlslib.c", TLS_LIB)
    _w(e.td, "tlsmain.c", TLS_MAIN)
    for sub in ("ref", "new"):
        os.mkdir(e.path(sub))
    for lib_flags, tagname in (([], ""), (["-mtls-dialect=gnu2"], "-gnu2")):
        for sub, lccc in (("ref", False), ("new", True)):
            _ok(e.gcc("-O2", "-fPIC", "-shared", *lib_flags, "tlslib.c",
                      "-o", f"{sub}/libtl.so", lccc=lccc), f"{sub} libtl.so{tagname}")
        ref_dso, new_dso = Elf32(e.path("ref/libtl.so")), Elf32(e.path("new/libtl.so"))
        for want, what in ((DF_STATIC_TLS, "DF_STATIC_TLS"), (DF_TEXTREL, "DF_TEXTREL")):
            if bool(ref_dso.flags() & want) != bool(new_dso.flags() & want):
                raise Fail(f"libtl.so{tagname}: {what} disagrees with the reference "
                           f"(ref FLAGS=0x{ref_dso.flags():x}, lccc 0x{new_dso.flags():x})")
        for exe_flags in ([], ["-fPIC"], ["-fPIC", "-fno-plt"], ["-fPIC", "-mtls-dialect=gnu2"]):
            label = f"lib{tagname} exe{''.join(exe_flags) or '-nopic'}"
            for sub, lccc in (("ref", False), ("new", True)):
                _ok(e.gcc("-O2", "-no-pie", *exe_flags, "tlsmain.c", f"-L{sub}", "-ltl",
                          "-pthread", "-o", f"{sub}/main", lccc=lccc), f"{label}: {sub} exe")
            for exe_sub, lib_sub in (("new", "new"), ("new", "ref"), ("ref", "new")):
                _expect_same_runs(e, e.path("ref/main"), e.path(exe_sub, "main"),
                                  e.path("ref"), e.path(lib_sub),
                                  f"{label} [{exe_sub} exe, {lib_sub} dso]")


def _reverse_relocs(src, dst):
    """Copy an ELF32 relocatable object with every SHT_REL table reversed.

    ELF gives relocation order no meaning, and GNU as happens to emit
    offset order; the reversed copy puts each `call ___tls_get_addr`
    relocation ahead of its `@tlsgd`/`@tlsldm` one, so a linker that only
    learns about a transitioned call while rewriting the sequence would
    mis-handle it."""
    with open(src, "rb") as f:
        b = bytearray(f.read())
    if b[:4] != b"\x7fELF" or b[4] != 1 or b[5] != 1:
        raise Fail(f"{src}: not an ELF32 LSB object")
    shoff, = struct.unpack_from("<I", b, 0x20)
    shentsize, shnum = struct.unpack_from("<HH", b, 0x2e)
    for i in range(shnum):
        _, sh_type, _, _, off, size, _, _, _, entsize = struct.unpack_from(
            "<10I", b, shoff + i * shentsize)
        if sh_type != 9 or entsize != 8 or size % 8:
            continue
        ents = [bytes(b[off + k:off + k + 8]) for k in range(0, size, 8)]
        b[off:off + size] = b"".join(reversed(ents))
    with open(dst, "wb") as f:
        f.write(b)


def case_exe_tls_transitions(e):
    """Executable-local TLS under every access model, dynamic and -static
    (libc.a's own IE/GOTIE sequences and weak undefined TLS included), with
    a .tbss more aligned than .tdata (PT_TLS must start p_align-aligned)."""
    _w(e.td, "tls.c", TLS_EXE_ONLY)
    _w(e.td, "def.c", "__thread int t_def = 44;\n")
    variants = ([], ["-fPIC"], ["-fPIC", "-fno-plt"], ["-fPIC", "-mtls-dialect=gnu2"],
                ["-fPIC", "-ftls-model=initial-exec"], ["-fPIC", "-ftls-model=local-dynamic"],
                ["-fPIC", "-O0"])
    for flags in variants:
        label = " ".join(flags) or "-fno-pic"
        _ok(e.gcc("-O2", *flags, "-c", "tls.c", "-o", "tls.o", lccc=False), label)
        _ok(e.gcc("-O2", *flags, "-c", "def.c", "-o", "def.o", lccc=False), label)
        for link in (["-no-pie"], ["-static"]):
            _ok(e.gcc(*link, "tls.o", "def.o", "-o", "ref", lccc=False), f"{label} ref {link}")
            _ok(e.gcc(*link, "tls.o", "def.o", "-o", "new"), f"{label} lccc {link}")
            _expect_same_runs(e, e.path("ref"), e.path("new"), None, None, f"{label} {link[0]}")
            if "-fPIC" in flags:
                # Same link with the call relocations ahead of the GD/LDM ones.
                _reverse_relocs(e.path("tls.o"), e.path("tls_rev.o"))
                _ok(e.gcc(*link, "tls_rev.o", "def.o", "-o", "rev"), f"{label} rev {link}")
                _expect_same_runs(e, e.path("ref"), e.path("rev"), None, None,
                                  f"{label} {link[0]} reversed relocations")
            tls = Elf32(e.path("new")).phdr(PT_TLS)
            if tls is None or tls[2] % tls[7]:
                raise Fail(f"{label} {link[0]}: PT_TLS p_vaddr 0x{tls[2] if tls else 0:x} "
                           f"not a multiple of p_align {tls[7] if tls else 0}")


def case_dso_chain_and_archive(e):
    """-l resolution of user shared objects with DT_NEEDED, a DSO depending
    on a DSO, and an archive member satisfying a DSO's undefined reference
    (the executable must extract it and export it for the DSO)."""
    _w(e.td, "prov.c", "int prov(int x) { return x * 7; }\n")
    _w(e.td, "cons.c", "int prov(int); int helper(void);\n"
                       "int cons(int x) { return prov(x) + helper(); }\n")
    _w(e.td, "helper.c", "int helper(void) { return 1000; }\n")
    _w(e.td, "unused.c", "#include <stdio.h>\n"
                         "__attribute__((constructor)) static void c(void) { puts(\"unused-linked\"); }\n"
                         "int unused_fn(void) { return 3; }\n")
    _w(e.td, "main.c", "#include <stdio.h>\nint cons(int);\n"
                       "int main(void) { printf(\"%d\\n\", cons(6)); return 0; }\n")
    for sub, lccc in (("ref", False), ("new", True)):
        os.mkdir(e.path(sub))
        _ok(e.gcc("-fPIC", "-shared", "-Wl,-soname,libprovider.so", "prov.c",
                  "-o", f"{sub}/libprovider.so", lccc=lccc), f"{sub} libprovider.so")
        _ok(e.gcc("-fPIC", "-shared", "cons.c", f"-L{sub}", "-lprovider",
                  "-o", f"{sub}/libconsumer.so", lccc=lccc), f"{sub} libconsumer.so")
        _ok(e.gcc("-c", "helper.c", "-o", f"{sub}/helper.o", lccc=False), "helper.o")
        _ok(e.gcc("-c", "unused.c", "-o", f"{sub}/unused.o", lccc=False), "unused.o")
        _ok(e.cmd(["ar", "rcs", f"{sub}/libhelper.a", f"{sub}/helper.o", f"{sub}/unused.o"]),
            "ar")
        _ok(e.gcc("-no-pie", "main.c", f"-L{sub}", "-lconsumer", "-lhelper",
                  f"-Wl,-rpath-link,{sub}", "-o", f"{sub}/main", lccc=lccc), f"{sub} main")
    cons = Elf32(e.path("new/libconsumer.so"))
    if "libprovider.so" not in cons.strs("NEEDED"):
        raise Fail(f"libconsumer.so DT_NEEDED {cons.strs('NEEDED')} lacks libprovider.so")
    exe, ref = Elf32(e.path("new/main")), Elf32(e.path("ref/main"))
    if exe.strs("NEEDED") != ref.strs("NEEDED"):
        raise Fail(f"exe DT_NEEDED {exe.strs('NEEDED')} != reference {ref.strs('NEEDED')}")
    out = _expect_same_runs(e, e.path("ref/main"), e.path("new/main"),
                            e.path("ref"), e.path("new"), "dso chain")
    if out != b"1042\n":
        raise Fail(f"unexpected reference output {out!r}")


def case_archive_semantics(e):
    """--whole-archive / --no-whole-archive positional state, -u pulling a
    member nothing else references, thin archives, and -Bstatic/-Bdynamic
    choosing libNAME.a over libNAME.so for the libraries in between."""
    for n in ("wa", "na", "um", "thin"):
        _w(e.td, f"{n}.c", "#include <stdio.h>\n"
                           f"__attribute__((constructor)) static void c(void) {{ puts(\"{n}\"); }}\n"
                           f"int {n}_sym(void) {{ return 1; }}\n")
        _ok(e.gcc("-c", f"{n}.c", "-o", f"{n}.o", lccc=False), f"{n}.o")
    _ok(e.cmd(["ar", "rcs", "libwa.a", "wa.o"]), "ar")
    _ok(e.cmd(["ar", "rcs", "libna.a", "na.o"]), "ar")
    _ok(e.cmd(["ar", "rcs", "libum.a", "um.o"]), "ar")
    _ok(e.cmd(["ar", "rcsT", "libthin.a", "thin.o"]), "ar T")
    _w(e.td, "main.c", "#include <stdio.h>\nint thin_sym(void);\n"
                       "int main(void) { printf(\"main %d\\n\", thin_sym()); return 0; }\n")
    link = ["-no-pie", "main.c", "-Wl,--whole-archive", "libwa.a", "-Wl,--no-whole-archive",
            "libna.a", "-Wl,-u,um_sym", "libum.a", "libthin.a"]
    _ok(e.gcc(*link, "-o", "ref", lccc=False), "reference link")
    _ok(e.gcc(*link, "-o", "new"), "lccc link")
    out = _expect_same_runs(e, e.path("ref"), e.path("new"), None, None, "archive semantics")
    lines = sorted(out.decode().split())
    if "wa" not in lines or "um" not in lines or "thin" not in lines or "na" in lines:
        raise Fail(f"reference archive behaviour unexpected: {out!r}")

    # -Bstatic / -Bdynamic are positional.
    _w(e.td, "bs.c", "int bs_val(void) { return 77; }\n")
    _ok(e.gcc("-c", "bs.c", "-o", "bs.o", lccc=False), "bs.o")
    _ok(e.cmd(["ar", "rcs", "libbs.a", "bs.o"]), "ar")
    _ok(e.gcc("-fPIC", "-shared", "bs.c", "-o", "libbs.so", lccc=False), "libbs.so")
    _w(e.td, "bsm.c", "#include <stdio.h>\nint bs_val(void);\n"
                      "int main(void) { printf(\"%d\\n\", bs_val()); return 0; }\n")
    link = ["-no-pie", "bsm.c", "-L.", "-Wl,-Bstatic", "-lbs", "-Wl,-Bdynamic"]
    _ok(e.gcc(*link, "-o", "bsref", lccc=False), "reference -Bstatic link")
    _ok(e.gcc(*link, "-o", "bsnew"), "lccc -Bstatic link")
    new, ref = Elf32(e.path("bsnew")), Elf32(e.path("bsref"))
    if new.strs("NEEDED") != ref.strs("NEEDED") or "libbs.so" in new.strs("NEEDED"):
        raise Fail(f"-Bstatic: DT_NEEDED {new.strs('NEEDED')} vs reference {ref.strs('NEEDED')}")
    if not new.has("NEEDED") or "libc.so.6" not in new.strs("NEEDED"):
        raise Fail("-Bdynamic did not restore dynamic libc")
    _expect_same_runs(e, e.path("bsref"), e.path("bsnew"), e.td, e.td, "-Bstatic")

    # --push-state/--pop-state around --as-needed must not leak.
    link = ["-no-pie", "bsm.c", "-L.", "-Wl,--push-state,--as-needed", "-lbs",
            "-Wl,--pop-state", "-lm"]
    _ok(e.gcc(*link, "-o", "psref", lccc=False), "reference push-state link")
    _ok(e.gcc(*link, "-o", "psnew"), "lccc push-state link")
    new, ref = Elf32(e.path("psnew")), Elf32(e.path("psref"))
    if new.strs("NEEDED") != ref.strs("NEEDED"):
        raise Fail(f"--push-state: DT_NEEDED {new.strs('NEEDED')} vs reference {ref.strs('NEEDED')}")


def case_options_and_tags(e):
    """Semantic options land in the image: -e, -rpath with explicit
    --enable-new-dtags (RUNPATH), -soname, -z now/relro/norelro/execstack,
    --hash-style, --build-id, -Bsymbolic, --export-dynamic; unsupported
    ones are refused with a diagnostic instead of being ignored."""
    _w(e.td, "entry.c", r"""
        #include <unistd.h>
        #include <stdlib.h>
        void my_entry(void) { write(1, "entry-ok\n", 9); _exit(0); }
        int main(void) { write(1, "main\n", 5); return 0; }
    """)
    for sub, lccc in (("ref", False), ("new", True)):
        _ok(e.gcc("-no-pie", "entry.c", "-Wl,-e,my_entry", "-o", f"entry_{sub}", lccc=lccc),
            f"{sub} -e link")
    out = _expect_same_runs(e, e.path("entry_ref"), e.path("entry_new"), None, None, "-e")
    if out != b"entry-ok\n":
        raise Fail(f"-e: reference output {out!r}")

    os.mkdir(e.path("lib"))
    _w(e.td, "rl.c", "int rl(void) { return 9; }\n")
    _w(e.td, "rm.c", "#include <stdio.h>\nint rl(void);\n"
                     "int main(void) { printf(\"%d\\n\", rl()); return 0; }\n")
    _ok(e.gcc("-fPIC", "-shared", "-Wl,-soname,librl.so.1", "rl.c", "-o", "lib/librl.so.1"),
        "lccc -soname link")
    os.symlink("librl.so.1", e.path("lib/librl.so"))
    if Elf32(e.path("lib/librl.so.1")).strs("SONAME") != ["librl.so.1"]:
        raise Fail("-soname not recorded")
    for sub, lccc in (("ref", False), ("new", True)):
        _ok(e.gcc("-no-pie", "rm.c", "-Llib", "-lrl", "-Wl,-rpath,$ORIGIN/lib",
                  "-Wl,--enable-new-dtags", "-o", f"rp_{sub}", lccc=lccc),
            f"{sub} -rpath link")
    new, ref = Elf32(e.path("rp_new")), Elf32(e.path("rp_ref"))
    for kind in ("RPATH", "RUNPATH"):
        if new.strs(kind) != ref.strs(kind):
            raise Fail(f"DT_{kind} {new.strs(kind)} != reference {ref.strs(kind)}")
    if "librl.so.1" not in new.strs("NEEDED"):
        raise Fail(f"DT_NEEDED should use the soname: {new.strs('NEEDED')}")
    _expect_same_runs(e, e.path("rp_ref"), e.path("rp_new"), None, None, "-rpath $ORIGIN")

    checks = [
        (["-Wl,-z,now"], lambda x: x.flags() & DF_BIND_NOW and x.flags_1() & DF_1_NOW,
         "-z now: DF_BIND_NOW + DF_1_NOW"),
        (["-Wl,-z,relro"], lambda x: x.phdr(PT_GNU_RELRO) is not None, "-z relro: PT_GNU_RELRO"),
        (["-Wl,-z,norelro"], lambda x: x.phdr(PT_GNU_RELRO) is None, "-z norelro: no PT_GNU_RELRO"),
        (["-Wl,-z,execstack"], lambda x: x.phdr(PT_GNU_STACK)[6] & 1, "-z execstack: PF_X"),
        (["-Wl,-z,noexecstack"], lambda x: not x.phdr(PT_GNU_STACK)[6] & 1, "-z noexecstack"),
        (["-Wl,--hash-style=sysv"], lambda x: x.has("HASH") and not x.has("GNU_HASH"), "sysv hash"),
        (["-Wl,--hash-style=gnu"], lambda x: x.has("GNU_HASH") and not x.has("HASH"), "gnu hash"),
        (["-Wl,--hash-style=both"], lambda x: x.has("GNU_HASH") and x.has("HASH"), "both hashes"),
        (["-Wl,--build-id=sha1"], lambda x: x.build_id() is not None and len(x.build_id()) == 20,
         "--build-id=sha1 note"),
        (["-Wl,--build-id=none"], lambda x: x.build_id() is None, "--build-id=none"),
    ]
    for flags, pred, what in checks:
        for shared in (False, True):
            out = "o_so" if shared else "o_exe"
            # The library after the object that uses it: gcc links -l
            # libraries --as-needed, so GNU ld drops one named earlier.
            inputs = (["-fPIC", "-shared", "rl.c"] if shared
                      else ["-no-pie", "rm.c", "-Llib", "-lrl"])
            _ok(e.gcc(*inputs, *flags, "-o", out), f"{what} ({'so' if shared else 'exe'})")
            if not pred(Elf32(e.path(out))):
                raise Fail(f"{what} not reflected in the {'shared object' if shared else 'executable'}")
            if what.startswith("-z now") or "hash" in what:
                if not shared:
                    _expect_same_runs(e, e.path("rp_ref"), e.path(out), None, e.path("lib"), what)

    # -Bsymbolic: tags plus the binding it promises.
    _w(e.td, "sym.c", "int f(void) { return 1; } int g(void) { return f(); }\n")
    _w(e.td, "symm.c", "#include <stdio.h>\nint g(void);\nint f(void) { return 2; }\n"
                       "int main(void) { printf(\"%d\\n\", g()); return 0; }\n")
    for flags, want in (([], b"2\n"), (["-Wl,-Bsymbolic"], b"1\n"),
                        (["-Wl,-Bsymbolic-functions"], b"1\n")):
        _ok(e.gcc("-fPIC", "-shared", "sym.c", *flags, "-o", "libsym.so"), f"{flags} so")
        dso = Elf32(e.path("libsym.so"))
        if flags == ["-Wl,-Bsymbolic"] and not (dso.has("SYMBOLIC") and dso.flags() & DF_SYMBOLIC):
            raise Fail("-Bsymbolic: DT_SYMBOLIC/DF_SYMBOLIC missing")
        _ok(e.gcc("-no-pie", "symm.c", "-L.", "-lsym", "-o", "symm"), "symm")
        for now in (False, True):
            r = e.run(e.path("symm"), now, e.td)
            if r.stdout != want:
                raise Fail(f"{flags or 'default'} binding: got {r.stdout!r}, want {want!r}")

    # --export-dynamic: a dlopen'ed plugin binds back into the executable.
    _w(e.td, "plug.c", "int host_fn(void); int plug(void) { return host_fn() + 1; }\n")
    _w(e.td, "host.c", r"""
        #include <dlfcn.h>
        #include <stdio.h>
        int host_fn(void) { return 41; }
        int main(void) {
            void *h = dlopen("./libplug.so", RTLD_NOW);
            if (!h) { printf("dlopen: %s\n", dlerror()); return 1; }
            int (*p)(void) = (int (*)(void))dlsym(h, "plug");
            printf("%d\n", p());
            return 0;
        }
    """)
    _ok(e.gcc("-fPIC", "-shared", "plug.c", "-o", "libplug.so"), "plugin")
    _ok(e.gcc("-no-pie", "-rdynamic", "host.c", "-ldl", "-o", "host"), "-rdynamic host")
    for now in (False, True):
        r = e.run(e.path("host"), now)
        if r.stdout != b"42\n":
            raise Fail(f"--export-dynamic: {r.stdout!r} {r.stderr.decode()[:200]!r}")

    # PIE and -z max-page-size: both land in the image and run identically to GNU ld.
    for sub, lccc in (("ref", False), ("new", True)):
        _ok(e.gcc("-pie", "-fPIE", "rm.c", "-Llib", "-lrl", "-Wl,-rpath,$ORIGIN/lib",
                  "-o", f"pie_{sub}", lccc=lccc), f"{sub} -pie link")
    pie_new = Elf32(e.path("pie_new"))
    if pie_new.e_type != 3 or not (pie_new.flags_1() & 0x08000000) or pie_new.phdr(3) is None:
        raise Fail(f"-pie: expected ET_DYN + DF_1_PIE + PT_INTERP, got e_type={pie_new.e_type} "
                   f"flags_1={pie_new.flags_1():#x}")
    _expect_same_runs(e, e.path("pie_ref"), e.path("pie_new"), None, None, "-pie")

    _ok(e.gcc("-no-pie", "rm.c", "-Llib", "-lrl", "-Wl,-rpath,$ORIGIN/lib",
              "-Wl,-z,max-page-size=0x200000", "-o", "mps_new"), "-z max-page-size=0x200000 link")
    mps_loads = [p for p in Elf32(e.path("mps_new")).phdrs if p[0] == PT_LOAD]
    if not mps_loads or any(p[7] != 0x200000 for p in mps_loads):
        raise Fail(f"-z max-page-size=0x200000: PT_LOAD alignments {[hex(p[7]) for p in mps_loads]}")
    _expect_same_runs(e, e.path("rp_ref"), e.path("mps_new"), None, None, "-z max-page-size=0x200000")

    # Refusals must be diagnostics, never silently different output.
    for flags, needle in ((["-static-pie", "-fPIE"], "static-pie"),
                          (["-no-pie", "-Wl,-z,nocopyreloc"], "nocopyreloc"),
                          (["-no-pie", "-Wl,-z,max-page-size=0x3000"], "page size"),
                          (["-no-pie", "-Wl,--emit-relocs"], "--emit-relocs")):
        r = e.gcc(*flags, "rm.c", "-Llib", "-lrl", "-o", "refused")
        err = r.stderr.decode()
        if r.returncode == 0 or needle not in err:
            raise Fail(f"{flags}: expected a refusal mentioning {needle!r}, got rc={r.returncode} "
                       f"{err[:300]!r}")


FEATURE_1_AND, ISA_1_NEEDED, ISA_1_USED = 0xC0000002, 0xC0008002, 0xC0010002


def _prop_asm(entries, body, *more_notes):
    """`body` plus a `.note.gnu.property` section holding one
    NT_GNU_PROPERTY_TYPE_0 note per entry list (`entries`, then
    `more_notes`, all in the SAME section — the shape gas 2.47 produces when
    it appends its x86 used-note to a hand-written one).  Entries are
    (type, value) 4-byte bitmasks or (type, value, datasz) with datasz 0
    (marker); ELF32 pads each entry's data to 4."""
    out = ['    .section .note.gnu.property,"a"', "    .p2align 2"]
    for note in (entries,) + more_notes:
        out += ["    .long 4", "    .long 2f - 1f", "    .long 5", '    .asciz "GNU"', "1:"]
        for ent in note:
            t, v, sz = ent if len(ent) == 3 else (ent[0], ent[1], 4)
            out += [f"    .long {t:#x}", f"    .long {sz}"] + ([f"    .long {v:#x}"] if sz else [])
        out += ["2:"]
    return "\n".join(out + ["    .text", body]) + "\n"


def _check_property_image(img, label):
    """The shape every property-note output must have: at most one GNU
    property note; when present, a PT_GNU_PROPERTY covering exactly that
    note inside a PT_NOTE; and no PT_GNU_EH_FRAME outside the image."""
    props = img.properties()
    if isinstance(props, list):
        raise Fail(f"{label}: {len(props)} GNU property notes (one merged note expected): {props}")
    gp = img.phdr(PT_GNU_PROPERTY)
    if (props is None) != (gp is None):
        raise Fail(f"{label}: property note {props} but PT_GNU_PROPERTY {gp}")
    if gp is not None:
        if not any(p[0] == PT_NOTE and p[1] <= gp[1] and gp[1] + gp[4] <= p[1] + p[4]
                   for p in img.phdrs):
            raise Fail(f"{label}: PT_GNU_PROPERTY {gp} outside every PT_NOTE")
        namesz, descsz, ntype = struct.unpack_from("<III", img.d, gp[1])
        if ntype != NT_GNU_PROPERTY_TYPE_0 or gp[4] != 12 + ((namesz + 3) & ~3) + descsz:
            raise Fail(f"{label}: PT_GNU_PROPERTY does not cover exactly the property note")
    for p in img.phdrs:
        if p[0] == PT_GNU_EH_FRAME and (p[5] == 0 or not any(
                q[0] == PT_LOAD and q[2] <= p[2] and p[2] + p[5] <= q[2] + q[5]
                for q in img.phdrs)):
            raise Fail(f"{label}: PT_GNU_EH_FRAME {p} does not describe an .eh_frame_hdr")
    return props


def case_property_notes(e):
    """GNU property notes (CET / ISA level) are merged, not concatenated.

    Differential against GNU ld on identical objects: the output carries
    exactly one NT_GNU_PROPERTY_TYPE_0 note with GNU's merged value, covered
    by PT_GNU_PROPERTY.  The ELF32 linker used to append every input's note
    to the note region (three notes for one -fcf-protection object on
    Debian, claiming IBT|SHSTK although crti.o carries no note and vetoes
    both), emitted no PT_GNU_PROPERTY, and ignored -z ibt / -z shstk /
    -z x86-64-* — which elf_i386 supports (binutils sources cet.sh and
    x86-64-level.sh for it; only -z lam-* is x86-64-only and warned about).
    Also pinned: an image without FDEs has no PT_GNU_EH_FRAME (it used to
    get one at p_vaddr 0, which libgcc's unwinder dereferences)."""
    # 1. Every input annotated (-nostdlib): AND intersects, OR accumulates.
    _w(e.td, "pa.s", _prop_asm([(FEATURE_1_AND, 3), (ISA_1_NEEDED, 4)],
                               "    .globl _start\n_start:\n    movl $1, %eax\n"
                               "    xorl %ebx, %ebx\n    int $0x80"))
    _w(e.td, "pb.s", _prop_asm([(FEATURE_1_AND, 1)], "    .globl pb\npb:\n    ret"))
    # Two notes in one section: every note counts (the second one's
    # ISA_1_NEEDED ORs with the first's inside the object), the all-zero
    # OR-AND ISA_1_USED is kept when every input has it, the generic OR type
    # 0xb0008000 and the NO_COPY_ON_PROTECTED marker survive from one input,
    # STACK_SIZE takes the maximum and the unknown 0xc0028000 is dropped.
    # lccc used to read only the first note.
    _w(e.td, "pm.s", _prop_asm([(FEATURE_1_AND, 3), (ISA_1_NEEDED, 1)],
                               "    .globl _start\n_start:\n    movl $1, %eax\n"
                               "    xorl %ebx, %ebx\n    int $0x80",
                               [(ISA_1_NEEDED, 2), (ISA_1_USED, 0), (0xB0008000, 2),
                                (1, 0x1000)]))
    _w(e.td, "pn.s", _prop_asm([(1, 0x3000), (2, 0, 0), (FEATURE_1_AND, 1), (ISA_1_USED, 0),
                                (0xC0028000, 7)], "    .globl pn\npn:\n    ret"))
    for src in ("pa", "pb", "pm", "pn"):
        _ok(e.gcc("-c", f"{src}.s", "-o", f"{src}.o", lccc=False), f"assemble {src}.s")
    variants = [
        ("all-annotated", ["-nostdlib", "-static", "pa.o", "pb.o"], []),
        ("-z ibt,shstk,x86-64-v2", ["-nostdlib", "-static", "pa.o", "pb.o"],
         ["-Wl,-z,shstk,-z,x86-64-v2"]),
        ("multi-note", ["-nostdlib", "-static", "pm.o", "pn.o"], []),
    ]
    # 2. Driver links of a -fcf-protection object (crt files decide the veto:
    #    Debian's crti.o has no note, Ubuntu's does) — executables and DSOs.
    _w(e.td, "cf.c", '#include <stdio.h>\nint cf(void){ return 3; }\n'
                     'int main(void){ printf("cf %d\\n", cf()); return 0; }\n')
    _ok(e.gcc("-O1", "-fcf-protection", "-fPIC", "-c", "cf.c", "-o", "cf.o", lccc=False),
        "compile cf.c")
    for tag, extra in (("exe", ["-no-pie"]), ("dso", ["-shared"])):
        variants += [
            (f"{tag} default", extra + ["cf.o"], []),
            (f"{tag} -z ibt -z x86-64-v3", extra + ["cf.o"], ["-Wl,-z,ibt", "-Wl,-z,x86-64-v3"]),
            (f"{tag} -z shstk", extra + ["cf.o"], ["-Wl,-z,shstk"]),
        ]
    for label, argv, zflags in variants:
        imgs = {}
        for sub, lccc in (("ref", False), ("new", True)):
            out = f"prop_{sub}"
            _ok(e.gcc(*argv, *zflags, "-o", out, lccc=lccc), f"{label}: {sub} link")
            imgs[sub] = Elf32(e.path(out))
            if "-shared" not in argv:
                r = e.run(e.path(out), False)
                if r.returncode != 0:
                    raise Fail(f"{label}: {sub} run rc={r.returncode}")
        want = imgs["ref"].properties()
        got = _check_property_image(imgs["new"], label)
        if got != want:
            raise Fail(f"{label}: property note {got} != GNU ld's {want}")
        if label == "all-annotated" and imgs["new"].phdr(PT_GNU_EH_FRAME) is not None:
            raise Fail(f"{label}: PT_GNU_EH_FRAME without any FDE")
    # 3. -z lam-u48 is not an elf_i386 keyword: warned about (GNU's wording)
    #    and ignored; an invalid ISA level is fatal on both linkers.
    r = _ok(e.gcc("-no-pie", "cf.o", "-Wl,-z,lam-u48", "-o", "lam"), "-z lam-u48 link")
    if b"-z lam-u48 ignored" not in r.stderr:
        raise Fail(f"-z lam-u48: expected GNU's 'ignored' warning, got {r.stderr[:200]!r}")
    lam = _check_property_image(Elf32(e.path("lam")), "-z lam-u48")
    _ok(e.gcc("-no-pie", "cf.o", "-o", "lam_ref", lccc=False), "reference link")
    if lam != Elf32(e.path("lam_ref")).properties():
        raise Fail(f"-z lam-u48 changed the property note: {lam}")
    for lccc in (False, True):
        r = e.gcc("-no-pie", "cf.o", "-Wl,-z,x86-64-v9", "-o", "bad", lccc=lccc)
        if r.returncode == 0 or b"invalid x86-64 ISA level" not in r.stderr:
            raise Fail(f"-z x86-64-v9 ({'lccc' if lccc else 'GNU'}): rc={r.returncode} "
                       f"{r.stderr[:200]!r}")


TEXTREL_ASM = """\
    .text
    .globl get41
    .type get41,@function
get41:
    movl $v41, %eax
    movl (%eax), %eax
    ret
    .size get41, .-get41
    .section .rodata,"a"
    .p2align 2
    .globl ptr42
    .type ptr42,@object
    .size ptr42,4
ptr42: .long v42
    .data
v41: .long 41
v42: .long 42
    .section .note.GNU-stack,"",@progbits
"""


def case_text_relocations(e):
    """Text relocations in an i386 shared object follow GNU ld's policy.

    Absolute `R_386_32` fields in `.text` and `.rodata` of a non-PIC shared
    object become dynamic relocations in read-only storage. Differential
    against GNU ld: by default both warn about the first site and identify
    text relocations with DT_TEXTREL and/or DF_TEXTREL; BFD 2.47 can omit the
    redundant DF_TEXTREL bit. `-z text` is refused with GNU's message;
    `-z notext` is silent. lccc-ld used to
    emit the tags silently and reject `-z text` with a message of its own;
    the library must still run.
    """
    _w(e.td, "tr.s", TEXTREL_ASM)
    _w(e.td, "trm.c", "#include <stdio.h>\nint get41(void); extern int *const ptr42;\n"
                      "int main(void){ printf(\"%d %d\\n\", get41(), *ptr42); return 0; }\n")
    _ok(e.gcc("-c", "tr.s", "-o", "tr.o", lccc=False), "assemble tr.s")
    _ok(e.gcc("-O1", "-fno-pic", "-c", "trm.c", "-o", "trm.o", lccc=False), "compile trm.c")
    for lccc in (False, True):
        who = "lccc" if lccc else "GNU"
        r = e.gcc("-shared", "tr.o", "-o", "libtr.so", lccc=lccc)
        _ok(r, f"{who}: default link")
        if (b"creating DT_TEXTREL in a shared object" not in r.stderr
                or b"tr.o: warning: relocation in read-only section `." not in r.stderr):
            raise Fail(f"{who}: default link must warn like GNU ld: {r.stderr[:300]!r}")
        img = Elf32(e.path("libtr.so"))
        if not img.has("TEXTREL") and not img.flags() & DF_TEXTREL:
            raise Fail(f"{who}: DT_TEXTREL/DF_TEXTREL marker missing")
        r = e.gcc("-shared", "tr.o", "-Wl,-z,text", "-o", "libtr_zt.so", lccc=lccc)
        if r.returncode == 0 or b"read-only segment has dynamic relocations" not in r.stderr:
            raise Fail(f"{who} -z text: rc={r.returncode} {r.stderr[:300]!r}")
        r = _ok(e.gcc("-shared", "tr.o", "-Wl,-z,notext", "-o", "libtr_nt.so", lccc=lccc),
                f"{who}: -z notext")
        if b"TEXTREL" in r.stderr or b"read-only" in r.stderr:
            raise Fail(f"{who} -z notext must be silent: {r.stderr[:300]!r}")
        _ok(e.gcc("-no-pie", "trm.o", "-o", "trm", e.path("libtr.so"), lccc=lccc),
            f"{who}: main link")
        r = e.run(e.path("trm"), False, libdir=e.td)
        if r.returncode != 0 or r.stdout != b"41 42\n":
            raise Fail(f"{who}: run rc={r.returncode} out={r.stdout!r}")


COPY_ALIAS_LIB = """\
int real_obj = 5;
extern int weak_obj __attribute__((weak, alias("real_obj")));
int lib_get(void) { return real_obj; }
void lib_set(int v) { real_obj = v; }
"""

COPY_ALIAS_MAIN = """\
#include <stdio.h>
#include <stdlib.h>
extern int weak_obj __attribute__((weak));
extern char **environ;
int lib_get(void);
void lib_set(int);
static char *my_env[] = { "LCCC_ALIAS=yes", 0 };
int main(void)
{
    weak_obj = 11;
    int a = lib_get();
    lib_set(22);
    environ = my_env;
    const char *v = getenv("LCCC_ALIAS");
    printf("%d %d %s\\n", a, weak_obj, v ? v : "(null)");
    return 0;
}
"""


def _dyn_relocs_and_syms(e, exe):
    """(R_386_COPY target names, {dynsym name: (value, binding)}) via readelf."""
    rel = _ok(e.cmd(["readelf", "-rW", "-D", exe]), "readelf -r").stdout.decode()
    copies = [ln.split()[-1].split("@")[0] for ln in rel.splitlines() if "R_386_COPY" in ln]
    out = _ok(e.cmd(["readelf", "-sW", "-D", exe]), "readelf -s").stdout.decode()
    syms = {}
    for ln in out.splitlines():
        f = ln.split()
        if len(f) >= 8 and f[0][:-1].isdigit() and f[0].endswith(":") and f[6] != "UND":
            syms[f[7].split("@")[0]] = (int(f[1], 16), f[4])
    return copies, syms


def case_copy_relocation_aliases(e):
    """A copy relocation moves an object together with all its names.

    A non-PIC executable referencing a shared library's WEAK data symbol
    whose object also has a strong name (glibc's `environ` / `__environ`,
    here `weak_obj` / `real_obj`) must define every name at the one copy,
    or the library keeps working on its original while the program sees
    the copy.  lccc-ld used to dodge the copy for WEAK data and patch every
    reference in `.text` instead: a DT_TEXTREL executable (now warned
    about) for every non-PIC program naming `environ`.  Differential
    against GNU ld: one R_386_COPY per object, every alias exported at the
    copy with the library's binding, no DT_TEXTREL, same output.
    """
    _w(e.td, "ca.c", COPY_ALIAS_LIB)
    _w(e.td, "cam.c", COPY_ALIAS_MAIN)
    _ok(e.gcc("-O1", "-fPIC", "-shared", "ca.c", "-o", "libca.so", lccc=False), "build libca.so")
    _ok(e.gcc("-O1", "-fno-pic", "-c", "cam.c", "-o", "cam.o", lccc=False), "compile cam.c")
    views = {}
    for lccc in (False, True):
        who = "lccc" if lccc else "GNU"
        exe = f"cam_{who}"
        r = _ok(e.gcc("-no-pie", "cam.o", "-o", exe, e.path("libca.so"), "-Wl,-z,text",
                      lccc=lccc), f"{who}: link")
        if r.stderr:
            raise Fail(f"{who}: unexpected diagnostics {r.stderr[:300]!r}")
        img = Elf32(e.path(exe))
        if img.has("TEXTREL") or img.flags() & DF_TEXTREL:
            raise Fail(f"{who}: DT_TEXTREL in a copy-relocating executable")
        for bind_now in (False, True):
            r = e.run(e.path(exe), bind_now, libdir=e.td)
            if r.returncode != 0 or r.stdout != b"11 22 yes\n":
                raise Fail(f"{who}: run rc={r.returncode} out={r.stdout!r}")
        copies, syms = _dyn_relocs_and_syms(e, e.path(exe))
        groups = (("weak_obj", "real_obj"), ("environ", "__environ"))
        for group in groups:
            if not all(n in syms for n in group):
                raise Fail(f"{who}: {group} not all exported: {sorted(syms)}")
            if len({syms[n][0] for n in group}) != 1:
                raise Fail(f"{who}: aliases {group} at different addresses: "
                           f"{[hex(syms[n][0]) for n in group]}")
            if sum(1 for c in copies if syms.get(c, (None,))[0] == syms[group[0]][0]) != 1:
                raise Fail(f"{who}: want exactly one R_386_COPY for {group}, got {copies}")
        views[who] = {n: syms[n][1] for g in groups for n in g}
    if views["lccc"] != views["GNU"]:
        raise Fail(f"alias bindings {views['lccc']} != GNU ld's {views['GNU']}")


COPY_NEED_LIB = """\
int got_only = 7;
int direct = 8;
int dbg_only = 9;
void lib_bump(void) { got_only += 10; direct += 10; }
"""

COPY_NEED_PIC = """\
extern int got_only;
int read_got_only(void) { return got_only; }
"""

COPY_NEED_MAIN = """\
#include <stdio.h>
extern int direct;
int read_got_only(void);
void lib_bump(void);
int main(void)
{
    lib_bump();
    printf("%d %d\\n", read_got_only(), direct);
    return 0;
}
"""

COPY_NEED_DBG = """\
\t.section .debug_lccc_note,"",@progbits
\t.long dbg_only
"""


def case_copy_only_when_addressed(e):
    """Only a DSO object the executable addresses directly is copied.

    `got_only` is reached from PIC code through its GOT slot alone,
    `dbg_only` is named only by a non-allocated (DWARF-like) section, and
    `direct` is addressed by non-PIC code (R_386_32).  GNU ld copies just
    `direct`; lccc-ld used to copy every DSO data object it resolved,
    whatever the reference: a COPY relocation and .bss for nothing, which
    ld.so also refuses for protected-visibility objects.  Exact COPY set,
    GLOB_DAT for the GOT-only object, and the same output as GNU ld's
    link, lazily and with immediate binding.
    """
    _w(e.td, "cn.c", COPY_NEED_LIB)
    _w(e.td, "cnp.c", COPY_NEED_PIC)
    _w(e.td, "cnm.c", COPY_NEED_MAIN)
    _w(e.td, "cnd.s", COPY_NEED_DBG)
    _ok(e.gcc("-O1", "-fPIC", "-shared", "cn.c", "-o", "libcn.so", lccc=False), "build libcn.so")
    _ok(e.gcc("-O1", "-fPIC", "-c", "cnp.c", "-o", "cnp.o", lccc=False), "compile cnp.c")
    _ok(e.gcc("-O1", "-fno-pic", "-c", "cnm.c", "-o", "cnm.o", lccc=False), "compile cnm.c")
    _ok(e.gcc("-c", "cnd.s", "-o", "cnd.o", lccc=False), "assemble cnd.s")
    for lccc in (False, True):
        who = "lccc" if lccc else "GNU"
        exe = f"cn_{who}"
        _ok(e.gcc("-no-pie", "cnm.o", "cnp.o", "cnd.o", "-o", exe, e.path("libcn.so"),
                  "-Wl,-z,text", lccc=lccc), f"{who}: link")
        for bind_now in (False, True):
            r = e.run(e.path(exe), bind_now, libdir=e.td)
            if r.returncode != 0 or r.stdout != b"17 18\n":
                raise Fail(f"{who}: run rc={r.returncode} out={r.stdout!r}")
        rel = _ok(e.cmd(["readelf", "-rW", "-D", e.path(exe)]), "readelf -r").stdout.decode()
        by_type = {}
        for ln in rel.splitlines():
            f = ln.split()
            if len(f) >= 5 and f[2].startswith("R_386_"):
                by_type.setdefault(f[2], set()).add(f[4].split("@")[0])
        copies = by_type.get("R_386_COPY", set())
        if copies != {"direct"}:
            raise Fail(f"{who}: R_386_COPY for {sorted(copies)}, want ['direct']")
        if "got_only" not in by_type.get("R_386_GLOB_DAT", set()):
            raise Fail(f"{who}: got_only has no R_386_GLOB_DAT: {by_type}")


DBG_LIB = r"""
#include <stdio.h>
int lib_counter = 3;
__thread int lib_tls = 4;
static int lib_static(int v) { return v * lib_counter; }
int lib_fn(int v) { lib_tls += v; return lib_static(v) + lib_tls; }
"""

DBG_MAIN = r"""
#include <stdio.h>
extern int lib_fn(int);
extern int lib_counter;
__thread int t_var = 6;
int g_init = 7;
int g_zero;
const int g_const = 8;
static int static_fn(int x) {
    int r = x + g_init;
    return r * 2;
}
int helper(int x) {
    g_zero += static_fn(x);
    return g_zero + t_var + lib_fn(x) + lib_counter;
}
int main(void) {
    printf("%d %d\n", helper(2), g_const);
    return 0;
}
"""

USER_ANCHORS = r"""
#include <stdio.h>
/* Legal C names GNU ld only PROVIDEs: the program's definitions win. */
int end = 42;
int etext = 7;
int edata = 9;
int main(void) {
    end++;
    printf("%d %d %d\n", end, etext, edata);
    return 0;
}
"""

ALIGN_LIB = r"""
char a_pad = 1;
_Alignas(16) float b_vec[4] = {1, 2, 3, 4};
long long c_wide = 5;
"""

ALIGN_MAIN = r"""
#include <stdio.h>
#include <stdint.h>
#include <xmmintrin.h>
extern char a_pad;
extern float b_vec[4];
extern long long c_wide;
/* GCC folds `&c_wide % 8` to 0: it assumes the ABI alignment of an extern
   long long -- which is exactly what the COPY slot must provide. Launder the
   addresses so the run measures them. */
static uintptr_t opaque(const void *p) {
    uintptr_t v = (uintptr_t)p;
    __asm__("" : "+r"(v));
    return v;
}
int main(void) {
    __m128 v = _mm_load_ps(b_vec);          /* movaps: faults if misaligned */
    float out[4];
    _mm_storeu_ps(out, _mm_add_ps(v, v));
    printf("%d %d %g %d %lld\n", (int)(opaque(b_vec) % 16),
           (int)(opaque(&c_wide) % 8), out[3], a_pad, c_wide);
    return 0;
}
"""


def _readelf_clean(e, path, *opts):
    """readelf output; any warning or error on stderr is a malformed file."""
    r = _ok(e.cmd(["readelf", "-W", *opts, path]), f"readelf {' '.join(opts)} {path}")
    err = r.stderr.decode(errors="replace").strip()
    if err:
        raise Fail(f"readelf {' '.join(opts)} {os.path.basename(path)}: {err[:400]}")
    return r.stdout.decode()


def _sections(e, path):
    """{name: (index, type, flags)} from readelf -S."""
    out = {}
    for m in re.finditer(r"^\s*\[\s*(\d+)\]\s+(\S+)\s+(\S+)\s+[0-9a-f]+\s+[0-9a-f]+\s+"
                         r"[0-9a-f]+\s+[0-9a-f]+\s+(\w*)", _readelf_clean(e, path, "-S"), re.M):
        out[m.group(2)] = (int(m.group(1)), m.group(3), m.group(4))
    return out


def _nm_types(e, path, names):
    r = _ok(e.cmd(["nm", path]), f"nm {path}").stdout.decode()
    got = {}
    for ln in r.splitlines():
        f = ln.split()
        if len(f) == 3 and f[2] in names:
            got[f[2]] = f[1]
    return got


def case_section_headers_and_debug(e):
    """Executables and shared objects carry a complete section-header table,
    the DWARF of their inputs, and a .symtab, as GNU ld's do.

    lccc-ld used to write executables with no section headers at all (and
    shared objects with no debug sections), so gdb, addr2line, nm and perf
    saw nothing: a `-g` build could not be debugged.  The same `gcc -g`
    objects are linked by GNU ld and lccc-ld and must agree on: the set of
    .debug_* sections; each user symbol's nm class (so st_shndx names the
    right kind of section); the run output.  readelf must read every table
    without a warning, `strip` and `objcopy` (which rebuild the file from
    its section headers) must produce images that still run identically,
    `-S` must drop the DWARF but keep .symtab, and `-s` both.  An exported
    definition's .dynsym st_shndx must name its real section.
    """
    _w(e.td, "dl.c", DBG_LIB)
    _w(e.td, "dm.c", DBG_MAIN)
    _ok(e.gcc("-g", "-O0", "-fPIC", "-c", "dl.c", "-o", "dl.o", lccc=False), "compile dl.c")
    _ok(e.gcc("-g", "-O0", "-fno-pic", "-c", "dm.c", "-o", "dm.o", lccc=False), "compile dm.c")
    os.mkdir(e.path("ref"))
    os.mkdir(e.path("new"))
    for lccc, d in ((False, "ref"), (True, "new")):
        _ok(e.gcc("-shared", "dl.o", "-o", e.path(d, "libdl1.so"), lccc=lccc),
            f"{d}: link libdl1.so")
        _ok(e.gcc("-no-pie", "dm.o", "-o", e.path(d, "dm"), e.path(d, "libdl1.so"),
                  lccc=lccc), f"{d}: link dm")
    want = _expect_same_runs(e, e.path("ref", "dm"), e.path("new", "dm"),
                             e.path("ref"), e.path("new"), "debug link")
    user = {"main", "helper", "static_fn", "g_init", "g_zero", "g_const", "t_var"}
    for f in ("dm", "libdl1.so"):
        ref, new = e.path("ref", f), e.path("new", f)
        dbg_ref = {n for n in _sections(e, ref) if n.startswith(".debug_")}
        secs = _sections(e, new)
        dbg_new = {n for n in secs if n.startswith(".debug_")}
        if not dbg_ref or dbg_new != dbg_ref:
            raise Fail(f"{f}: debug sections {sorted(dbg_new)}, GNU ld {sorted(dbg_ref)}")
        for need in (".symtab", ".strtab", ".dynsym", ".dynamic", ".text"):
            if need not in secs:
                raise Fail(f"{f}: no {need} header")
        _readelf_clean(e, new, "-l", "-s", "--dyn-syms", "-r", "-d")
        _readelf_clean(e, new, "--debug-dump=info,line,aranges,str")
    got, ref_types = (_nm_types(e, e.path(d, "dm"), user) for d in ("new", "ref"))
    if got != ref_types or len(got) != len(user):
        raise Fail(f"nm classes {got}, GNU ld {ref_types}")
    # .dynsym st_shndx of the library's exports.
    secs = _sections(e, e.path("new", "libdl1.so"))
    by_index = {i: n for n, (i, _, _) in secs.items()}
    dyn = _readelf_clean(e, e.path("new", "libdl1.so"), "--dyn-syms")
    for sym, want_sec in (("lib_fn", ".text"), ("lib_counter", ".data"), ("lib_tls", ".tdata")):
        m = re.search(r"^\s*\d+:\s+[0-9a-f]+\s+\d+\s+\w+\s+\w+\s+\w+\s+(\d+)\s+" + sym + r"$",
                      dyn, re.M)
        if not m or by_index.get(int(m.group(1))) != want_sec:
            raise Fail(f"libdl1.so: {sym} st_shndx -> "
                       f"{by_index.get(int(m.group(1))) if m else None}, want {want_sec}")
    # Tools that rebuild the file from its section headers.
    for tool, argv in (("strip", ["strip", "-o"]), ("objcopy", ["objcopy", "-R", ".comment"])):
        for f in ("dm", "libdl1.so"):
            os.makedirs(e.path(tool), exist_ok=True)
            src, dst = e.path("new", f), e.path(tool, f)
            cmd = argv + [dst, src] if tool == "strip" else argv + [src, dst]
            _ok(e.cmd(cmd), f"{tool} {f}")
        for now in (False, True):
            r = e.run(e.path(tool, "dm"), now, e.path(tool))
            if r.returncode != 0 or r.stdout != want:
                raise Fail(f"{tool}ped image: rc={r.returncode} out={r.stdout!r} want {want!r}")
    # -S keeps .symtab, -s drops it; both drop the DWARF.
    for flag, keeps_symtab in (("-Wl,-S", True), ("-Wl,--strip-debug", True), ("-Wl,-s", False)):
        exe = e.path("new", "dm" + flag.replace(",", "_").replace("-", "_"))
        _ok(e.gcc("-no-pie", "dm.o", "-o", exe, e.path("new", "libdl1.so"), flag), f"link {flag}")
        secs = _sections(e, exe)
        if any(n.startswith(".debug_") for n in secs) or (".symtab" in secs) != keeps_symtab:
            raise Fail(f"{flag}: sections {sorted(secs)}")
        r = e.run(exe, False, e.path("new"))
        if r.returncode != 0 or r.stdout != want:
            raise Fail(f"{flag}: rc={r.returncode} out={r.stdout!r}")


def case_provided_symbols_yield_to_inputs(e):
    """GNU ld only PROVIDEs `end`, `etext`, `edata`, `__data_start` and
    friends: an input definition wins.  lccc-ld's executable emitter used to
    overwrite them unconditionally, so a program's own `int end` was
    redirected to the end of .bss (its stores clobbering whatever lay
    there), and crt1.o's `__data_start` pointed into the RELRO padding
    instead of at .data.
    """
    _w(e.td, "ua.c", USER_ANCHORS)
    for lccc in (False, True):
        who = "lccc" if lccc else "GNU"
        _ok(e.gcc("-O1", "-no-pie", "-fno-pic", "ua.c", "-o", f"ua_{who}", lccc=lccc),
            f"{who}: link")
    _expect_same_runs(e, e.path("ua_GNU"), e.path("ua_lccc"), None, None, "user anchors")
    secs = _sections(e, e.path("ua_lccc"))
    sh = _readelf_clean(e, e.path("ua_lccc"), "-S")
    m = re.search(r"\]\s+\.data\s+PROGBITS\s+([0-9a-f]+)\s+[0-9a-f]+\s+([0-9a-f]+)", sh)
    if ".data" not in secs or not m:
        raise Fail("no .data header")
    lo, size = int(m.group(1), 16), int(m.group(2), 16)
    nm = _ok(e.cmd(["nm", e.path("ua_lccc")]), "nm").stdout.decode()
    vals = {f[2]: int(f[0], 16) for f in (ln.split() for ln in nm.splitlines()) if len(f) == 3}
    for sym in ("__data_start", "end", "etext", "edata"):
        if not lo <= vals.get(sym, -1) < lo + size:
            raise Fail(f"{sym} = {vals.get(sym)!r}, outside .data [{lo:#x}, {lo + size:#x})")


def case_copy_relocation_alignment(e):
    """The executable's copy of a copy-relocated object keeps the alignment
    the library gave it.  lccc-ld aligned every COPY slot to 4: after a
    1-byte object, a 16-byte-aligned vector landed 4 mod 16 and the
    program's `movaps` faulted; an 8-byte object straddling a cache line
    loses the atomicity of its 8-byte accesses.
    """
    _w(e.td, "al.c", ALIGN_LIB)
    _w(e.td, "am.c", ALIGN_MAIN)
    _ok(e.gcc("-O1", "-fPIC", "-shared", "al.c", "-o", "libal.so", lccc=False), "build libal.so")
    _ok(e.gcc("-O1", "-msse", "-fno-pic", "-c", "am.c", "-o", "am.o", lccc=False), "compile am.c")
    for lccc in (False, True):
        who = "lccc" if lccc else "GNU"
        _ok(e.gcc("-no-pie", "am.o", "-o", f"am_{who}", e.path("libal.so"), lccc=lccc),
            f"{who}: link")
    out = _expect_same_runs(e, e.path("am_GNU"), e.path("am_lccc"), e.td, e.td, "copy alignment")
    if out != b"0 0 8 1 5\n":
        raise Fail(f"reference output {out!r}")
    rel = _ok(e.cmd(["readelf", "-rW", e.path("am_lccc")]), "readelf -r").stdout.decode()
    copies = {ln.split()[4].split("@")[0] for ln in rel.splitlines()
              if len(ln.split()) >= 5 and ln.split()[2] == "R_386_COPY"}
    if copies != {"a_pad", "b_vec", "c_wide"}:
        raise Fail(f"R_386_COPY for {sorted(copies)}: the case no longer tests COPY slots")


LOCAL_HELPER_DEF = "static int helper(void) { return 41; }\nint a(void) { return helper(); }\n"
LOCAL_HELPER_USE = "int helper(void);\nint a(void);\nint main(void) { return helper() + a(); }\n"
LOCAL_ATOI_DEF = "static int atoi(const char *s) { (void)s; return 99; }\nint a(void) { return atoi(\"1\"); }\n"
LOCAL_ATOI_USE = r"""#include <stdio.h>
#include <stdlib.h>
int a(void);
int main(void) { printf("%d %d\n", atoi("5"), a()); return 0; }
"""
LOCAL_IFUNC_C = r"""#include <stdio.h>
static int impl(void) { return 42; }
static void *resolve_local(void) { return (void *)impl; }
static int local_fn(void) __attribute__((ifunc("resolve_local")));
static void *resolve_global(void) { return (void *)impl; }
int global_fn(void) __attribute__((ifunc("resolve_global")));
int (*volatile fp)(void) = local_fn;
int main(void) { printf("%d %d %d\n", local_fn(), global_fn(), fp()); return 0; }
"""
# R_386_GOT32 (assembled with -mrelax-relocations=no) against an STB_LOCAL
# object whose name another object defines globally, and against an
# STB_LOCAL IFUNC (whose slot must hold the IPLT stub, the address every
# other reference to it resolves to).
LOCAL_GOT_S = """\t.text
\t.globl\tvia_got
\t.type\tvia_got, @function
via_got:
\tcall\t1f
1:\tpopl\t%ecx
\taddl\t$_GLOBAL_OFFSET_TABLE_+[.-1b], %ecx
\tmovl\tlval@GOT(%ecx), %eax
\tmovl\t(%eax), %eax
\tret
\t.globl\tifunc_via_got
\t.type\tifunc_via_got, @function
ifunc_via_got:
\tcall\t2f
2:\tpopl\t%ecx
\taddl\t$_GLOBAL_OFFSET_TABLE_+[.-2b], %ecx
\tmovl\tpick@GOT(%ecx), %eax
\tret
\t.globl\tifunc_direct
\t.type\tifunc_direct, @function
ifunc_direct:
\tmovl\t$pick, %eax
\tret
impl:
\tmovl\t$7, %eax
\tret
\t.type\tpick, @gnu_indirect_function
pick:
\tmovl\t$impl, %eax
\tret
\t.data
\t.type\tlval, @object
lval:\t.long\t123
\t.section\t.note.GNU-stack,"",@progbits
"""
LOCAL_GOT_MAIN = r"""#include <stdio.h>
int lval = 456;
int via_got(void);
int (*ifunc_via_got(void))(void);
int (*ifunc_direct(void))(void);
int main(void) {
    int (*a)(void) = ifunc_via_got(), (*b)(void) = ifunc_direct();
    printf("%d %d %d %d %d\n", via_got(), lval, a(), b(), a == b);
    return 0;
}
"""


def case_local_symbols_have_file_scope(e):
    """An STB_LOCAL symbol is visible only inside its own object.  lccc-ld
    used to enter locals into the global table as fallback definitions:
    another file's `static helper` silently satisfied an undefined
    `helper`, a `static atoi` captured every object's call to libc's, and a
    local's name could route other references through a same-named
    global's PLT.  GOT slots were keyed by name, so `local@GOT` collided
    with a global of that name (a hard error), and STB_LOCAL IFUNCs were
    called -- and had their address taken -- as their resolver in dynamic
    links and as their resolver's address through pointers in static ones.
    """
    for fn, body in (("lh.c", LOCAL_HELPER_DEF), ("lu.c", LOCAL_HELPER_USE),
                     ("la.c", LOCAL_ATOI_DEF), ("lm.c", LOCAL_ATOI_USE),
                     ("if.c", LOCAL_IFUNC_C), ("lg.s", LOCAL_GOT_S), ("lgm.c", LOCAL_GOT_MAIN)):
        _w(e.td, fn, body)
    _ok(e.gcc("-O0", "-fno-builtin", "-c", "lh.c", "lu.c", "la.c", "lm.c", lccc=False), "compile")
    _ok(e.gcc("-O1", "-c", "if.c", "lgm.c", lccc=False), "compile")
    _ok(e.gcc("-c", "-Wa,-mrelax-relocations=no", "lg.s", lccc=False), "assemble")
    rel = _ok(e.cmd(["readelf", "-rW", e.path("lg.o")]), "readelf -r").stdout.decode()
    if sum(" R_386_GOT32 " in ln for ln in rel.splitlines()) != 2:
        raise Fail(f"lg.o lost its R_386_GOT32 relocations: the case tests nothing\n{rel}")

    # The undefined reference stays undefined.
    for lccc in (False, True):
        who = "lccc" if lccc else "GNU"
        r = e.gcc("-no-pie", "lh.o", "lu.o", "-o", f"lh_{who}", lccc=lccc)
        if r.returncode == 0 or "helper" not in r.stderr.decode(errors="replace"):
            raise Fail(f"{who}: undefined `helper' linked (rc={r.returncode})")

    want = {"la": b"5 99\n", "if": b"42 42 42\n", "lg": b"123 456 7 7 1\n"}
    objs = {"la": ["la.o", "lm.o"], "if": ["if.o"], "lg": ["lg.o", "lgm.o"]}
    for link in ("-no-pie", "-static"):
        for key, inputs in objs.items():
            for lccc in (False, True):
                who = "lccc" if lccc else "GNU"
                _ok(e.gcc(link, *inputs, "-o", f"{key}{link}_{who}", lccc=lccc),
                    f"{who} {link}: link {key}")
            out = _expect_same_runs(e, e.path(f"{key}{link}_GNU"), e.path(f"{key}{link}_lccc"),
                                    None, None, f"{key} {link}")
            if out != want[key]:
                raise Fail(f"{key} {link}: reference output {out!r}, want {want[key]!r}")

    # A shared library's undefined `helper' is an import, never bound to
    # another object's local of that name.
    _ok(e.gcc("-fPIC", "-O0", "-c", "lh.c", "-o", "plh.o", lccc=False), "compile -fPIC")
    _ok(e.gcc("-fPIC", "-O0", "-c", "lu.c", "-o", "plu.o", lccc=False), "compile -fPIC")
    _ok(e.gcc("-shared", "plh.o", "plu.o", "-o", "libl.so"), "lccc: link -shared")
    dyn = _readelf_clean(e, e.path("libl.so"), "--dyn-syms")
    if not re.search(r"\bUND\s+helper\b", dyn):
        raise Fail(f"libl.so does not import `helper':\n{dyn}")


CASES = [
    ("i386_tls_matrix", case_tls_matrix),
    ("i386_exe_tls_transitions", case_exe_tls_transitions),
    ("i386_dso_chain_archive", case_dso_chain_and_archive),
    ("i386_archive_semantics", case_archive_semantics),
    ("i386_options_and_tags", case_options_and_tags),
    ("i386_property_notes", case_property_notes),
    ("i386_text_relocations", case_text_relocations),
    ("i386_copy_relocation_aliases", case_copy_relocation_aliases),
    ("i386_copy_only_when_addressed", case_copy_only_when_addressed),
    ("i386_section_headers_and_debug", case_section_headers_and_debug),
    ("i386_provided_symbols_yield_to_inputs", case_provided_symbols_yield_to_inputs),
    ("i386_copy_relocation_alignment", case_copy_relocation_alignment),
    ("i386_local_symbols_have_file_scope", case_local_symbols_have_file_scope),
]


def run_all(args, cc, result_cls):
    """Run the selected cases; returns Result objects."""
    selected = [(n, f) for n, f in CASES if not args.filter or args.filter in n]
    if not selected:
        return []
    probe_env = Env(args, cc)
    try:
        reason = _probe(probe_env)
    finally:
        probe_env.close(False)
    if reason:
        status = "FAIL" if os.environ.get("LCCC_REQUIRE_I386") == "1" else "SKIP"
        return [result_cls(n, status, reason) for n, _ in selected]
    results = []
    for name, fn in selected:
        e = Env(args, cc)
        try:
            fn(e)
            results.append(result_cls(name, "PASS"))
        except Fail as err:
            results.append(result_cls(name, "FAIL", f"{err} (scratch: {e.td})"))
            e.close(True)
            continue
        except Exception as err:  # noqa: BLE001 - report harness faults as failures
            results.append(result_cls(name, "FAIL",
                                      f"harness exception: {err!r} (scratch: {e.td})"))
            e.close(True)
            continue
        e.close(getattr(args, "keep", False))
    return results
