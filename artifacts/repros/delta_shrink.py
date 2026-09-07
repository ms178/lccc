"""Greedy line-deleter shrinker for a miscompile between split ON and gcc."""
import subprocess, sys

BIN = "/home/user/lccc/target/fastbuild/lccc"


def bad(src_path):
    c1 = subprocess.run("%s -O2 %s -o /tmp/sh_on" % (BIN, src_path), shell=True, capture_output=True)
    c2 = subprocess.run("gcc -O2 %s -o /tmp/sh_gcc" % src_path, shell=True, capture_output=True)
    if c1.returncode != 0 or c2.returncode != 0:
        return None  # does not compile: must keep the lines
    a = subprocess.run("/tmp/sh_on", shell=True, capture_output=True, text=True)
    b = subprocess.run("/tmp/sh_gcc", shell=True, capture_output=True, text=True)
    if a.returncode != 0 or b.returncode != 0:
        return None  # crash/hang: counts as "interesting", keep lines
    return a.stdout.strip() != b.stdout.strip()


def main(path):
    lines = open(path).read().split("\n")
    i = 0
    while i < len(lines):
        cand = lines[:i] + lines[i + 1:]
        open(path, "w").write("\n".join(cand))
        if open(path).read().count("\n") < 3:
            i += 1
            continue
        r = bad(path)
        if r is not True:  # correct again, or no longer valid -> keep the line
            open(path, "w").write("\n".join(lines))
            i += 1
        else:
            lines = cand
    print("\n".join(lines))


if __name__ == "__main__":
    main(sys.argv[1])
