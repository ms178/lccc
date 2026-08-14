#!/usr/bin/env python3
"""Fetch assembly from godbolt.org's Compiler Explorer API for ICC/ICX/etc.

Usage: gb.py <compiler-id> <source-file> [--flags "..."] [--func name]
Prints the assembly (intel syntax, filtered) to stdout.
"""
import json
import subprocess
import sys
import urllib.request

def compile_on_godbolt(compiler_id: str, source: str, flags: str):
    body = {
        "source": source,
        "options": {
            "userArguments": flags,
            "filters": {
                "binary": False,
                "execute": False,
                "intel": True,
                "demangle": True,
                "directives": True,
                "labels": True,
                "commentOnly": True,
                "trim": True,
            },
            "compilerOptions": {"executorRequest": False},
        },
    }
    req = urllib.request.Request(
        f"https://godbolt.org/api/compiler/{compiler_id}/compile",
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json", "Accept": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=120) as resp:
        data = json.load(resp)
    if data.get("code") != 0:
        sys.stderr.write("compile failed: " + json.dumps(data.get("stderr", [])) + "\n")
        return None
    return data

def main():
    args = sys.argv[1:]
    if len(args) < 2:
        sys.stderr.write(__doc__)
        sys.exit(2)
    compiler_id = args[0]
    source_file = args[1]
    flags = "-O3 -march=x86-64-v3"
    func = None
    i = 2
    while i < len(args):
        if args[i] == "--flags":
            flags = args[i + 1]
            i += 2
        elif args[i] == "--func":
            func = args[i + 1]
            i += 2
        else:
            i += 1
    source = open(source_file).read()
    data = compile_on_godbolt(compiler_id, source, flags)
    if data is None:
        sys.exit(1)
    for block in data.get("asm", []):
        text = block.get("text", "")
        print(text)

if __name__ == "__main__":
    main()
