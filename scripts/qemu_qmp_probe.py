#!/usr/bin/env python3
"""Boot a bzImage under QEMU with a QMP monitor and dump the guest state.

Why this exists
---------------
When an lccc-built kernel does not boot, the failure is silent: the compressed
stub's ``error()`` writes to the VGA text buffer (invisible with ``-nographic``)
and the kernel proper has no console until ``console_init``. This probe reads
the state straight out of the VM and maps it back to symbols:

* registers (RIP/RSP/CR2/CR3, R14 often holds the trap number on the early-IDT
  path), the code bytes at RIP, the physical stack (``xp`` is *physical*, so
  kernel virtual addresses must be translated by hand:
  ``phys = virt - 0xffffffff81000000 + 0x1000000``) and the VGA text buffer at
  0xb8000 (where the stub's error message lands);
* RIP/backtrace resolution against ``arch/x86/boot/compressed/vmlinux``
  (decompressor) and, on request, ``vmlinux`` (kernel proper). The decompressor
  is a PIE linked at 0 and relocated at run time, so the load base is derived
  by locating the code bytes at RIP inside the ELF file.

Usage
-----
    qemu_qmp_probe.py <bzImage> [wait-seconds]
    qemu_qmp_probe.py <bzImage> 10 --vmlinux /path/to/vmlinux

This is how the preboot-ZSTD miscompile was diagnosed (``error+0x23`` reached
from ``decompress_kernel → zstd_decompress_dctx → handle_zstd_error``) and how
the follow-on early page fault in the kernel proper was found
(``early_fixup_exception`` spinning with ``CR2=0xffffffff82400000``).
"""

import bisect
import json
import os
import re
import signal
import socket
import subprocess
import sys
import time


def readelf_load_segments(elf):
    out = subprocess.run(["readelf", "-lW", elf], capture_output=True, text=True).stdout
    segs = []
    for line in out.splitlines():
        m = re.match(
            r"\s*LOAD\s+0x([0-9a-f]+)\s+0x([0-9a-f]+)\s+0x([0-9a-f]+)\s+"
            r"0x([0-9a-f]+)\s+0x([0-9a-f]+)",
            line,
        )
        if m:
            segs.append(
                (int(m.group(1), 16), int(m.group(2), 16), int(m.group(4), 16))
            )
    return segs


def symbol_table(elf):
    out = subprocess.run(["nm", "-n", elf], capture_output=True, text=True).stdout
    syms = []
    for line in out.splitlines():
        parts = line.split()
        if len(parts) == 3 and parts[1] in "TtRrDdBbAaWw":
            try:
                syms.append((int(parts[0], 16), parts[2]))
            except ValueError:
                pass
    syms.sort()
    return syms


class Resolver:
    def __init__(self, elf):
        self.elf = elf
        self.syms = symbol_table(elf)
        self.addrs = [s[0] for s in self.syms]
        self.segs = readelf_load_segments(elf)
        try:
            self.data = open(elf, "rb").read()
        except OSError:
            self.data = b""

    def resolve(self, vaddr):
        i = bisect.bisect_right(self.addrs, vaddr) - 1
        if i < 0:
            return "?0x%x" % vaddr
        return "%s+0x%x" % (self.syms[i][1], vaddr - self.addrs[i])

    def file_offset_to_vaddr(self, off):
        for o, va, fsz in self.segs:
            if o <= off < o + fsz:
                return va + (off - o)
        return None

    def base_from_code(self, code_bytes, runtime_addr):
        """Derive the PIE load base by locating `code_bytes` inside the ELF."""
        idx = self.data.find(code_bytes[:12])
        if idx < 0:
            return None
        va = self.file_offset_to_vaddr(idx)
        return None if va is None else runtime_addr - va


def qmp_connect(path):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(path)
    s.settimeout(5)
    return s.makefile("rw", buffering=1, encoding="utf-8", newline="\n"), s


def read_msg(f):
    while True:
        line = f.readline()
        if not line:
            return None
        try:
            msg = json.loads(line)
        except Exception:
            continue
        if "event" in msg:
            continue
        return msg


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    bz = sys.argv[1]
    wait = float(sys.argv[2]) if len(sys.argv) > 2 else 8.0
    vmlinux = None
    if "--vmlinux" in sys.argv:
        vmlinux = sys.argv[sys.argv.index("--vmlinux") + 1]
    kdir = os.environ.get(
        "KERNEL_DIR", os.path.dirname(os.path.dirname(os.path.abspath(bz))) + "/../.."
    )
    comp_elf = os.path.normpath(os.path.join(kdir, "arch/x86/boot/compressed/vmlinux"))

    qmp_path = "/tmp/lccc-qmp.sock"
    try:
        os.unlink(qmp_path)
    except FileNotFoundError:
        pass

    cmd = [
        "qemu-system-x86_64", "-m", "512", "-smp", "2", "-kernel", bz,
        "-nographic", "-no-reboot",
        "-append", "console=ttyS0,115200 nokaslr panic=-1",
        "-qmp", "unix:%s,server,nowait" % qmp_path,
    ]
    p = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        time.sleep(wait)
        f, sock = qmp_connect(qmp_path)

        def hmc(c):
            read_msg(f) if False else None
            f.write(json.dumps({"execute": "human-monitor-command",
                                "arguments": {"command-line": c}}) + "\n")
            return (read_msg(f) or {}).get("return", "")

        read_msg(f)  # greeting
        f.write(json.dumps({"execute": "qmp_capabilities"}) + "\n")
        read_msg(f)

        regs = hmc("info registers")
        print("=== registers ===")
        for line in regs.splitlines():
            if line.startswith(("RAX", "RIP", "RSP", "RBP", "R12", "R13", "R14",
                                "R15", "CR0", "CR2", "CR3", "CR4", "EFER")):
                print("  " + line.rstrip())

        m = re.search(r"RIP=([0-9a-f]+)", regs)
        rip = int(m.group(1), 16) if m else 0
        rsp = int(re.search(r"RSP=([0-9a-f]+)", regs).group(1), 16) \
            if re.search(r"RSP=([0-9a-f]+)", regs) else 0

        code = hmc("xp /32xb 0x%x" % rip)
        code_bytes = bytes(int(x, 16) for x in re.findall(r"0x([0-9a-f]{2})", code))
        print("=== code at RIP ===")
        print("  %s" % code_bytes[:16].hex())

        print("=== VGA text buffer (stub error messages land here) ===")
        vga = hmc("xp /80xb 0xb8000")
        text = "".join(chr(int(x, 16)) for x in re.findall(r"0x([0-9a-f]{2})", vga)[::2])
        print("  %r" % text[:160])

        print("=== stack (kernel windows need phys = virt - 0xffffffff81000000 + 0x1000000) ===")
        phys = rip - 0xFFFFFFFF81000000 + 0x1000000 if rip > 0xFFFFFFFF80000000 else rsp
        st = hmc("xp /32xg 0x%x" % phys)
        words = [int(x, 16) for x in re.findall(r"0x([0-9a-f]{16})", st)][:32]
        for i, w in enumerate(words):
            print("  [%2d] 0x%016x" % (i, w))

        for label, elf in (("decompressor", comp_elf), ("kernel", vmlinux)):
            if not elf or not os.path.exists(elf):
                continue
            r = Resolver(elf)
            base = r.base_from_code(code_bytes, rip)
            print("=== %s (%s) ===" % (label, elf))
            if base is None:
                print("  RIP not found in this ELF")
                continue
            print("  load base: 0x%x" % base)
            print("  RIP        -> %s" % r.resolve(rip - base))
            for w in words:
                if base < w < base + 0x1000000:
                    print("  stack 0x%x -> %s" % (w, r.resolve(w - base)))
    finally:
        p.send_signal(signal.SIGTERM)
        time.sleep(1)
        p.kill()
    return 0


if __name__ == "__main__":
    sys.exit(main())
