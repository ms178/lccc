#!/usr/bin/env python3
"""Boot a bzImage under QEMU, wait, then dump guest PHYSICAL memory ranges.

Why this exists
---------------
`qemu_qmp_probe.py` samples 64-byte windows through `xp`, which is enough to
resolve RIP but not to answer "how far did the in-place decompressor get, and
what does the output look like at the point of failure?".  QMP's `pmemsave`
writes an arbitrary physical range straight to a host file, so the whole
decompression output buffer (16 MiB) can be diffed byte-exactly against
`arch/x86/boot/compressed/vmlinux.bin` in one shot.

Usage
-----
    qemu_pmemsave.py <bzImage> <wait-seconds> <out-file> <phys-addr> <size> [<out-file> <addr> <size> ...]

Addresses/sizes accept 0x-prefixed hex.  The VM is torn down afterwards.
"""

import json
import os
import signal
import socket
import subprocess
import sys
import time


def read_msg(f):
    line = f.readline()
    return json.loads(line) if line else None


def main():
    if len(sys.argv) < 6 or (len(sys.argv) - 3) % 3 != 0:
        print(__doc__)
        return 2
    bz = sys.argv[1]
    wait = float(sys.argv[2])
    dumps = []
    args = sys.argv[3:]
    for i in range(0, len(args), 3):
        dumps.append((os.path.abspath(args[i]), int(args[i + 1], 0), int(args[i + 2], 0)))

    qmp_path = "/tmp/lccc-qmp-pmem.sock"
    try:
        os.unlink(qmp_path)
    except FileNotFoundError:
        pass
    append = os.environ.get("QEMU_APPEND", "console=ttyS0,115200 nokaslr panic=-1")
    cmd = [
        os.environ.get("QEMU", "qemu-system-x86_64"), "-m", os.environ.get("QEMU_MEM", "512"),
        "-smp", "2", "-kernel", bz, "-nographic", "-no-reboot",
        "-append", append,
        "-qmp", "unix:%s,server,nowait" % qmp_path,
    ]
    p = subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        time.sleep(wait)
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.connect(qmp_path)
        f = s.makefile("rw")
        read_msg(f)
        f.write(json.dumps({"execute": "qmp_capabilities"}) + "\n"); f.flush()
        read_msg(f)
        def cmd_sync(obj):
            # QMP interleaves asynchronous events (STOP, RESUME, ...) with
            # command replies; skip events until the reply arrives.
            f.write(json.dumps(obj) + "\n"); f.flush()
            while True:
                r = read_msg(f)
                if r is None or "return" in r or "error" in r:
                    return r

        cmd_sync({"execute": "stop"})
        regs = (cmd_sync({"execute": "human-monitor-command",
                          "arguments": {"command-line": "info registers"}}) or {}).get("return", "")
        if not isinstance(regs, str):
            regs = ""
        for line in regs.splitlines():
            if line.startswith(("RIP", "RAX", "RCX", "RSP", "R12", "R14")):
                print("  " + line.rstrip())
        for path, addr, size in dumps:
            r = cmd_sync({"execute": "pmemsave",
                          "arguments": {"val": addr, "size": size,
                                        "filename": path}})
            print("pmemsave 0x%x +0x%x -> %s : %s" % (addr, size, path,
                  "ok" if r and "return" in r else r))
    finally:
        p.send_signal(signal.SIGTERM)
        time.sleep(1)
        p.kill()
    return 0


if __name__ == "__main__":
    sys.exit(main())
