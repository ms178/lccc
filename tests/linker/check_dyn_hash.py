#!/usr/bin/env python3
"""Independently resolve every exported symbol through BOTH ELF hash tables.

A synthesised .hash/.gnu.hash can be structurally plausible and still be
unusable: glibc's ld.so walks the bucket/chain arrays itself, so the only
meaningful test is to perform that walk. This reimplements the lookups from
the ELF gABI (.hash) and glibc's dl-lookup.c (.gnu.hash) against the raw
bytes of the produced image.
"""
import struct
import subprocess
import sys


def sections(path):
    out = subprocess.run(["readelf", "-SW", path], capture_output=True, text=True).stdout
    secs = {}
    for line in out.splitlines():
        line = line.strip()
        if not line.startswith("["):
            continue
        body = line[line.index("]") + 1:].split()
        if len(body) < 5:
            continue
        name, typ, addr, off, size = body[0], body[1], body[2], body[3], body[4]
        try:
            secs[name] = (int(addr, 16), int(off, 16), int(size, 16))
        except ValueError:
            continue
    return secs


def elf_hash(name):
    h = 0
    for c in name:
        h = (h << 4) + c
        g = h & 0xF0000000
        if g:
            h ^= g >> 24
        h &= ~g & 0xFFFFFFFF
    return h


def gnu_hash(name):
    h = 5381
    for c in name:
        h = (h * 33 + c) & 0xFFFFFFFF
    return h


def dynsym_names(blob, dynstr):
    """Yield (index, name) for every entry in .dynsym."""
    for i in range(len(blob) // 24):
        st_name = struct.unpack_from("<I", blob, i * 24)[0]
        end = dynstr.index(b"\0", st_name)
        yield i, dynstr[st_name:end].decode()


def lookup_sysv(hash_blob, dynsym, dynstr, want):
    nbucket, nchain = struct.unpack_from("<II", hash_blob, 0)
    buckets = struct.unpack_from(f"<{nbucket}I", hash_blob, 8)
    chains = struct.unpack_from(f"<{nchain}I", hash_blob, 8 + 4 * nbucket)
    idx = buckets[elf_hash(want.encode()) % nbucket]
    seen = 0
    while idx != 0:
        st_name = struct.unpack_from("<I", dynsym, idx * 24)[0]
        end = dynstr.index(b"\0", st_name)
        if dynstr[st_name:end].decode() == want:
            return idx
        if idx >= len(chains):
            return None
        idx = chains[idx]
        seen += 1
        if seen > nchain:
            return None  # cycle
    return None


def lookup_gnu(gh, dynsym, dynstr, want):
    nbuckets, symoffset, bloom_size, bloom_shift = struct.unpack_from("<IIII", gh, 0)
    bloom = struct.unpack_from(f"<{bloom_size}Q", gh, 16)
    bo = 16 + 8 * bloom_size
    buckets = struct.unpack_from(f"<{nbuckets}I", gh, bo)
    co = bo + 4 * nbuckets
    h = gnu_hash(want.encode())

    word = bloom[(h // 64) % bloom_size]
    if not (word >> (h % 64)) & 1:
        return None, "bloom rejected a present symbol"
    if not (word >> ((h >> bloom_shift) % 64)) & 1:
        return None, "bloom rejected a present symbol"

    idx = buckets[h % nbuckets]
    if idx == 0:
        return None, "empty bucket for a present symbol"
    while True:
        off = co + 4 * (idx - symoffset)
        if off + 4 > len(gh):
            return None, "chain ran past end of .gnu.hash"
        chain = struct.unpack_from("<I", gh, off)[0]
        if (chain & ~1) == (h & ~1):
            st_name = struct.unpack_from("<I", dynsym, idx * 24)[0]
            end = dynstr.index(b"\0", st_name)
            if dynstr[st_name:end].decode() == want:
                return idx, None
        if chain & 1:
            return None, "chain terminated without a match"
        idx += 1


def main(path):
    secs = sections(path)
    blob = open(path, "rb").read()

    def read(name):
        if name not in secs:
            return None
        _, off, size = secs[name]
        return blob[off:off + size]

    dynsym, dynstr = read(".dynsym"), read(".dynstr")
    if dynsym is None or dynstr is None:
        print(f"{path}: no .dynsym/.dynstr")
        return 1

    names = [n for i, n in dynsym_names(dynsym, dynstr) if i != 0 and n]
    print(f"{path}: {len(names)} dynamic symbols")

    rc = 0
    sysv = read(".hash")
    if sysv:
        for n in names:
            if lookup_sysv(sysv, dynsym, dynstr, n) is None:
                print(f"  SYSV-HASH FAIL: {n} not findable")
                rc = 1
        if rc == 0:
            print("  .hash      : all symbols resolve")
    else:
        print("  .hash      : absent")

    gh = read(".gnu.hash")
    if gh:
        bad = 0
        for n in names:
            idx, err = lookup_gnu(gh, dynsym, dynstr, n)
            if idx is None:
                print(f"  GNU-HASH FAIL: {n}: {err}")
                bad = rc = 1
        if not bad:
            print("  .gnu.hash  : all symbols resolve")
        # A symbol that is absent must NOT resolve.
        idx, _ = lookup_gnu(gh, dynsym, dynstr, "definitely_not_here_xyz")
        if idx is not None:
            print("  GNU-HASH FAIL: absent symbol resolved")
            rc = 1
    else:
        print("  .gnu.hash  : absent")
    return rc


if __name__ == "__main__":
    sys.exit(max(main(p) for p in sys.argv[1:]))
