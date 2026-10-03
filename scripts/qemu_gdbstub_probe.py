#!/usr/bin/env python3
"""Break in a running QEMU guest over the gdbstub and dump what you need.

Debugging a miscompile that only shows up inside a booting kernel needs three
things at the moment of the fault: the register holding the bad value, the
return-address chain, and the memory it points at. Installing gdb is not always
possible (no root), and even with it the round trip is slow. QEMU's `-s`
gdbstub speaks the GDB remote serial protocol, which is ~40 lines of framing —
so this script talks to it directly and answers one question:

    "the next time execution reaches SYMBOL, and register REG holds a value in
     RANGE, show me REG, the other registers, and the return addresses on the
     stack."

That is exactly the shape of "kfree() was handed a pointer into .text — what
was the pointer and who called?", which is what motivated it.

Usage (two terminals, or background the QEMU side):

    qemu-system-x86_64 ... -s -S -accel tcg &      # -s = gdbstub on :1234
    scripts/qemu_gdbstub_probe.py --port 1234 \\
        --system-map /opt/kwork/linux-6.18.52/System.map \\
        --break free_large_kmalloc \\
        --reg rsi --until-range 0xffffffff81000000-0xffffffff82300000 \\
        --stack-scan --max-hits 4000

Notes that cost time to rediscover:
  * Use single-threaded TCG (`-accel tcg`, not `thread=multi`). With
    multi-threaded TCG the guest keeps running other vCPUs between packets and
    a `c`/stop sequence can miss the hit.
  * `-S` (stop at reset) is required: without it the guest is already past the
    boot-time break site by the time this script connects.
  * Software breakpoints (`Z0`) patch an int3 through QEMU's
    cpu_memory_rw_debug, so kernel text write-protection is not an obstacle.
"""
from __future__ import annotations

import argparse
import socket
import sys
import time

# QEMU's x86-64 gdbstub register order for the `g` packet. Everything after
# `gs` (segment internals, FPU, vector state) is ignored here.
REG_ORDER = [
    "rax", "rbx", "rcx", "rdx", "rsi", "rdi", "rbp", "rsp",
    "r8", "r9", "r10", "r11", "r12", "r13", "r14", "r15",
    "rip", "eflags", "cs", "ss", "ds", "es", "fs", "gs",
]


class RspError(RuntimeError):
    pass


class GdbStub:
    """Minimal GDB remote serial protocol client (no-ack mode)."""

    def __init__(self, host: str, port: int, timeout: float = 30.0):
        self.sock = socket.create_connection((host, port), timeout=timeout)
        self.sock.settimeout(timeout)
        self.buf = b""
        # Stay in ack mode. `QStartNoAckMode+` is the nicer protocol — one
        # $...# packet per reply, no +/- traffic — but the QEMU in this
        # environment answers it with an empty packet ($#00 = unsupported),
        # so the client has to work the documented way: QEMU prefixes every
        # reply with `+` and expects a `+` back for every packet we consume.
        reply = self.cmd("qSupported:multiprocess+")
        if reply[:1] == b"E":
            raise RspError(f"qSupported failed: {reply!r}")

    @staticmethod
    def _checksum(data: bytes) -> str:
        return f"{sum(data) & 0xFF:02x}"

    def _send(self, data: str) -> None:
        raw = data.encode()
        self.sock.sendall(b"$" + raw + b"#" + self._checksum(raw).encode())

    def _recv(self) -> bytes:
        while True:
            start = self.buf.find(b"$")
            if start >= 0:
                end = self.buf.find(b"#", start)
                if end >= 0 and len(self.buf) >= end + 3:
                    payload = self.buf[start + 1:end]
                    self.buf = self.buf[end + 3:]
                    return payload
            chunk = self.sock.recv(65536)
            if not chunk:
                raise RspError("gdbstub closed the connection")
            self.buf += chunk

    def _ack(self) -> None:
        # Ack-mode only: tell the stub the packet arrived intact, otherwise it
        # retransmits and the stream desynchronises.
        self.sock.sendall(b"+")

    def cmd(self, data: str) -> bytes:
        self._send(data)
        reply = self._recv()
        self._ack()
        return reply

    # ---- the few packets this probe needs ---------------------------------
    def select_thread(self, stop_reply: bytes) -> None:
        """Point `g`/`m` at the CPU that actually stopped.

        With -smp N the stub reports `T05thread:01;` and then answers `g` for
        whichever thread is currently selected — which, unselected, is a stale
        snapshot rather than the stopped CPU. That shows up as every hit
        printing byte-identical registers while the guest keeps booting.
        """
        text = stop_reply.decode(errors="replace")
        tid = "01"
        if "thread:" in text:
            tid = text.split("thread:", 1)[1].split(";", 1)[0] or "01"
        # QEMU reports multiprocess form `p<pid>.<tid>` (e.g. `T05thread:p01.02;`).
        # `Hp`/`Hg` here want the bare tid — passing `p01.02` back is rejected
        # with E22, and the `p` must not be doubled into `Hpp01.02`.
        if tid.startswith("p") and "." in tid:
            tid = tid.rsplit(".", 1)[1]
        # NOTE: selecting the thread is *not* safe here. `Hg<tid>` is accepted
        # but on this target it leaves the guest unable to make progress (the
        # serial log stays empty and the breakpoint keeps re-reporting the same
        # state), and `Hp<tid>` is rejected with E22 outright. The tid is
        # therefore only reported, never selected.
        return tid

    def registers(self) -> dict[str, int]:
        raw = self.cmd("g")
        if raw[:1] == b"E":
            raise RspError(f"register read failed: {raw!r}")
        out = {}
        for i, name in enumerate(REG_ORDER):
            # Each register is 8 bytes, little-endian, hex-encoded.
            field = raw[i * 16:(i + 1) * 16]
            out[name] = int.from_bytes(bytes.fromhex(field.decode()), "little")
        return out

    def read_mem(self, addr: int, length: int) -> bytes:
        raw = self.cmd(f"m{addr:x},{length:x}")
        if raw[:1] in (b"E", b"X"):
            return b""
        try:
            return bytes.fromhex(raw.decode())
        except ValueError:
            return b""

    def set_break(self, addr: int) -> None:
        reply = self.cmd(f"Z0,{addr:x},1")
        if reply != b"OK":
            raise RspError(f"breakpoint at {addr:#x} refused: {reply!r}")

    def clear_break(self, addr: int) -> None:
        self.cmd(f"z0,{addr:x},1")

    def continue_(self) -> bytes:
        # `vCont;c` rather than bare `c`: with -smp N a bare `c` resumes only
        # the selected vCPU, so the guest can stall with the other CPU parked
        # and the breakpoint keeps firing on the same state.
        reply = self.cmd("vCont;c")
        if reply[:1] in (b"E", b"X"):
            reply = self.cmd("c")
        return reply

    def detach(self) -> None:
        try:
            self.cmd("D")
        except (RspError, OSError):
            pass
        self.sock.close()


def load_system_map(path: str) -> dict[str, int]:
    syms: dict[str, int] = {}
    with open(path) as fh:
        for line in fh:
            parts = line.split()
            if len(parts) >= 3:
                try:
                    syms[parts[2]] = int(parts[0], 16)
                except ValueError:
                    continue
    return syms


def parse_range(text: str) -> tuple[int, int]:
    lo, _, hi = text.partition("-")
    return int(lo, 0), int(hi, 0)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=1234)
    ap.add_argument("--system-map", required=True)
    ap.add_argument("--break", dest="breaks", action="append", required=True,
                    metavar="SYMBOL_OR_ADDR",
                    help="symbol name from System.map, or a 0x address")
    ap.add_argument("--reg", default=None,
                    help="register to test against --until-range")
    ap.add_argument("--until-range", default=None, metavar="LO-HI",
                    help="stop reporting once --reg falls inside this range")
    ap.add_argument("--max-hits", type=int, default=100,
                    help="report and continue this many times before giving up")
    ap.add_argument("--stack-scan", action="store_true",
                    help="dump rsp..rsp+512 as candidate return addresses")
    ap.add_argument("--text-range", default=None, metavar="LO-HI",
                    help="with --stack-scan, only show words inside this range")
    ap.add_argument("--seconds", type=float, default=240.0,
                    help="give up after this long (the guest may never get there)")
    args = ap.parse_args()

    syms = load_system_map(args.system_map)
    stub = GdbStub(args.host, args.port)
    print(f"connected to gdbstub at {args.host}:{args.port}")

    addrs = []
    for spec in args.breaks:
        addr = syms.get(spec) if not spec.startswith("0x") else int(spec, 0)
        if addr is None:
            print(f"symbol not in System.map: {spec}", file=sys.stderr)
            return 2
        stub.set_break(addr)
        addrs.append(addr)
        print(f"breakpoint: {spec} = {addr:#x}")

    want = parse_range(args.until_range) if args.until_range else None
    text = parse_range(args.text_range) if args.text_range else None
    deadline = time.monotonic() + args.seconds
    hits = 0
    verdict = 1

    try:
        stub.cmd("?")  # consume the -S reset stop
        while hits < args.max_hits and time.monotonic() < deadline:
            reply = stub.continue_()
            if reply[:1] not in (b"S", b"T"):
                print(f"unexpected continue reply: {reply!r}", file=sys.stderr)
                return 2
            tid = stub.select_thread(reply)
            regs = stub.registers()
            hits += 1
            probe = regs.get(args.reg) if args.reg else None
            inside = probe is not None and want is not None and want[0] <= probe <= want[1]
            tag = "MATCH" if inside else f"hit {hits} tid={tid}"
            shown = {k: v for k, v in regs.items() if k in
                     ("rax", "rbx", "rcx", "rdx", "rsi", "rdi", "rbp", "rsp",
                      "r8", "r12", "r13", "r14", "r15", "rip")}
            detail = " ".join(f"{k}={v:#x}" for k, v in shown.items())
            print(f"[{tag}] rip={regs['rip']:#x} {detail}")
            if not inside:
                continue

            # This is the stop we came for.
            verdict = 0
            print(f"\n=== {args.reg}={probe:#x} is inside "
                  f"{want[0]:#x}-{want[1]:#x} ===")
            nearest = min(syms.items(), key=lambda kv: abs(kv[1] - probe))
            print(f"nearest symbol to {args.reg}: {nearest[0]} "
                  f"{nearest[1]:#x} ({probe - nearest[1]:+#d})")
            rip_sym = min(syms.items(), key=lambda kv: abs(kv[1] - regs["rip"]))
            print(f"rip is {rip_sym[0]}+{regs['rip'] - rip_sym[1]:#x}")
            mem = stub.read_mem(probe, 64)
            if mem:
                print(f"first 64 bytes at {args.reg}: {mem.hex()}")
                printable = "".join(chr(b) if 32 <= b < 127 else "." for b in mem)
                print(f"  as text: {printable}")
            if args.stack_scan:
                words = stub.read_mem(regs["rsp"], 512)
                print(f"\nreturn-address candidates from rsp={regs['rsp']:#x}:")
                for off in range(0, len(words) - 7, 8):
                    val = int.from_bytes(words[off:off + 8], "little")
                    if text and not (text[0] <= val <= text[1]):
                        continue
                    owner = min(syms.items(), key=lambda kv: abs(kv[1] - val))
                    delta = val - owner[1]
                    if 0 <= delta < 0x4000:
                        print(f"  rsp+{off:#05x}: {val:#x}  "
                              f"{owner[0]}+{delta:#x}")
            break
        else:
            print(f"\nno hit satisfying the condition after {hits} stops "
                  f"(or {args.seconds}s elapsed)")
    finally:
        for addr in addrs:
            stub.clear_break(addr)
        stub.detach()
    return verdict


if __name__ == "__main__":
    sys.exit(main())
