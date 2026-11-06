# i386 multilib without destroying the machine

**Status:** enforced structurally (jail + offline provisioning + static CI gate).
**Audience:** every agent and human who needs 32-bit `-m32` coverage in this repo.

---

## 1. The failure, precisely

The sandbox root is *usr-merged*:

```
/lib   -> usr/lib
/lib64 -> usr/lib64
```

`/lib64/ld-linux-x86-64.so.2` is therefore reached **through a symlink**. Debian's
`libc6-i386` payload contains real directories `lib32/`, `lib/` and `lib64/`
(the package predates usr-merge and ships `/lib32` plus compatibility links).
Extracting it onto `/` — `dpkg-deb -x libc6-i386_*.deb /`, `dpkg -i`, or any
`tar`/`cp` equivalent — asks the extractor to create `/lib` and `/lib64` as
directories. The symlinks are replaced, the x86-64 ELF interpreter disappears,
and:

* every new process fails with `No such file or directory` from `execve`,
* that includes `bash`, `python3`, `git`, and the agent's own tool runner,
* so the session cannot repair itself with normal tools.

This has now happened in **three separate sessions**, each time through a
different script, and one of them was `tools/linker/recover_env.sh` — the
"environment recovery" script an agent runs *first* after a harness wipe. Prose
warnings in three documents did not prevent the third occurrence.

**Measured cost of the incident:** total loss of the session (all shell tools
dead), full re-clone and toolchain re-download.

## 2. Why the usual advice is not enough

| Approach | Verdict |
| --- | --- |
| "Just don't do it" in `RULES.md` | Failed three times. Prose is not a control. |
| `apt-get -s` simulation before installing | Necessary but insufficient: it does not model usr-merge symlink replacement, and the fatal command was `dpkg-deb -x`, which has no simulation mode. |
| `dpkg --force-overwrite` / `--force-unsafe-io` | Makes it *worse*. |
| `fakeroot` / `proot` / `bwrap` | Not installed in the sandbox; `proot` also slows every gate. |
| Read-only bind mount over `/` | Needs `CAP_SYS_ADMIN`; `mount --bind` inside a user namespace returns `EINVAL`/`EPERM` here (measured, rc 32). |
| Drop 32-bit coverage | Unacceptable: i686 is a first-class backend and linker target. |

## 3. The solution: three independent layers

### Layer 1 — a jail that cannot be turned off

`scripts/rootfs_jail.sh` runs a command inside a user namespace that maps **only
the caller's uid**. Even a process that believes it is uid 0 has no capability
over files owned by the real root, so the live root is effectively read-only,
while caller-owned trees (`/var/tmp`, `$HOME`, `$RUNNER_TEMP`) stay writable.
`sudo` cannot escape because there is no unmapped uid to escalate to.

```bash
bash scripts/rootfs_jail.sh --self-test     # 9 proofs, exit 0
bash scripts/rootfs_jail.sh --available     # exit 0 = usable, 3 = unavailable
bash scripts/rootfs_jail.sh -- exec dpkg-deb -x "$deb" "$sysroot"
```

Contract:

* **exit 3 = jail unavailable or ineffective.** The payload is **not run**.
  Fail-closed is the whole point: a "best effort" jail is how the third
  incident happened.
* No environment variable disables it. `LCCC_JAIL_CANARY_PARENT` only relocates
  the preflight canary.
* The preflight refuses to proceed if the canary parent is writable by the
  jailed process — i.e. it verifies its own protection before trusting it.

**Measured:** the exact historical command, run through the jail, returns
**rc 2** (`tar: …: Cannot open: Permission denied` for every member) and the host
still has `/lib -> usr/lib`, `/lib64 -> usr/lib64`, no `/lib32`, and a working
`/bin/sh`.

### Layer 2 — remove the *reason* anyone ever reached for `/`

Agents extracted onto `/` because `gcc -m32` did not work and the sysroot recipe
they tried was broken. `scripts/ensure_i386_multilib_safe.sh --offline-debs DIR`
now stages a **working** isolated sysroot from verified local `.deb`s:

```bash
bash scripts/ensure_i386_multilib_safe.sh \
     --offline-debs ~/.cache/lccc/agent-toolchains/debs
source /var/tmp/lccc-i686-sysroot/env.sh
m32-cc  prog.c   -o prog32 && ./prog32     # real 32-bit compile, link, run
m32-c++ prog.cpp -o prog32 && ./prog32
```

**Measured: 12 packages staged in 1.4 s, zero live-root writes, C and C++ both
compile + link + execute.** No network, no root, no `LD_LIBRARY_PATH`.

Three real defects made naive sysroot recipes fail; all three are now encoded in
one canonical helper (`m32_sysroot_flags`), which the probe *and* the generated
wrappers share, so they cannot drift:

1. **Header order.** `-idirafter $SR/usr/include` loses to the host headers and
   dies with `fatal error: bits/libc-header-start.h`. The working order is
   `-I$SR/usr/include -idirafter /usr/include/$(cc -print-multiarch)`. Bookworm's
   `libc6-dev-i386` ships only 25 real files (`gnu/stubs-32.h`,
   `gnu/lib-names-32.h`, …); `bits/*` correctly comes from the host amd64
   `libc6-dev`, whose `bits/wordsize.h` keys off `__x86_64__`. Do **not** try to
   make the sysroot header-complete.
2. **Linker scripts.** `$SR/usr/lib32/libc.so` is a GNU ld script referencing
   absolute `/lib32/libc.so.6`, `/usr/lib32/libc_nonshared.a` and
   `/lib/ld-linux.so.2`. Without `-Wl,--sysroot=$SR` the linker resolves those
   against the **live root**.
3. **Absolute intra-sysroot symlinks.** Nine links in the biarch set are absolute
   (`lib/ld-linux.so.2 -> /lib32/ld-linux.so.2`, eight `usr/lib32/*.so`). They
   must be re-rooted to relative targets, or `ld`/`ld.so` again resolve against
   the live root. `reroot_absolute_symlinks` does exactly that and leaves
   genuinely dangling links alone.

The produced binary has
`PT_INTERP = <sysroot>/lib32/ld-linux.so.2` and an rpath of
`<sysroot>/lib32:<sysroot>/usr/lib32`, so it runs in any environment — which
matters because gates execute the binaries in their own env.

Verification is **compile + link + execute**, for **C and C++ independently**.
A link-only check would have hidden defects 2 and 3; a C-only check would hide a
broken 32-bit libstdc++.

### Layer 3 — a static gate so it cannot come back

`scripts/check_rootfs_write_policy.py` scans every tracked `.sh`, `.py`, `.yml`
and `.yaml` file and fails on:

* `dpkg-deb`/`dpkg` extraction targeting `/` or a system directory,
* live `dpkg -i/--install/--unpack/--configure`,
* `dpkg --add-architecture i386`,
* `apt-get install` of i386/multilib packages outside the audited provisioner,
* `tar -C /`, `cp -a … /usr/`, `rsync … /lib`, destructive system `rm -rf`.

It strips privilege wrappers **including their options** — `sudo -n dpkg-deb -x
… /` is the exact historical command, and a naive stripper that only removed the
word `sudo` left `-n` as the program name and missed it. That gap was found by
the regression test, not by review.

Exemptions must be explicit and are printed as a complete inventory:

```bash
# rootfs-write-policy: allow(disposable GitHub runner, destroyed after the job)
sudo apt-get install -y libc6-dev-i386
```

Both CI paths run the detector self-test **before** the scan (a scanner that
silently stopped matching would be worse than no scanner):

* `scripts/ci_local.sh`: gates `rootfs-write-policy-self-test`, `rootfs-write-policy`
* `.github/workflows/ci.yml`: same two commands in the static-gate step
* `.github/ci-lane-plan.json`: both invocations are registered in the
  backend-contracts-a shard with measured cost, so the lane-plan audit cannot
  mistake them for unreviewed hand additions

**Measured state:** 1320 tracked source files scanned, 32 detector vectors
(19 positive / 10 negative) plus 3 exemption-mechanism vectors, 4 inventoried
exemptions (all disposable CI containers), 0 violations.

## 4. Context matters: disposable runner vs persistent sandbox

Live-root package installs are perfectly reasonable on a GitHub runner that is
destroyed after the job, and catastrophic in a persistent sandbox. The gate
cannot infer context, so the scripts state it:

* `scripts/ci_prepare_research_tools.sh` installs the i386 payload **only** when
  `GITHUB_ACTIONS=true` and `RUNNER_TEMP` is set; otherwise it installs the
  host-side tools and routes 32-bit coverage through the audited provisioner.
  Coverage is identical in both branches; only the blast radius differs.
* `tools/linker/recover_env.sh` never installs anything: it calls the audited
  provisioner and `scripts/ci_enable_swap.sh` (4 GiB, per the standing rule —
  it used to open-code an 8 GiB `/swapfile` through `sudo`).

## 5. If a machine is already broken

`execve` of dynamically linked binaries fails, so use the **static** loader that
is still on disk, or the loader by absolute path, to restore the symlinks:

```bash
# The x86-64 loader still exists under its usr-merged path:
/usr/lib64/ld-linux-x86-64.so.2 /usr/bin/rm -rf /lib /lib64
/usr/lib64/ld-linux-x86-64.so.2 /usr/bin/ln -s usr/lib /lib
/usr/lib64/ld-linux-x86-64.so.2 /usr/bin/ln -s usr/lib64 /lib64
```

If the payload overwrote files *inside* `usr/lib`, the machine is not repairable
in place — start a fresh sandbox. This is a recovery recipe of last resort, not
a licence to try the extraction.

## 6. Regression coverage

| Test | What it proves |
| --- | --- |
| `tests/corpus/test_rootfs_jail.py::test_jail_self_test_passes` | The jail's nine properties hold on this host (or it correctly reports itself unavailable). |
| `…::test_jail_blocks_a_live_root_write_and_host_is_unchanged` | A real `.deb` extraction aimed at `/` fails, no canary reaches the live root, the identity (symlink target / inode / mode) of `lib`, `lib64`, `lib32`, `libx32`, `bin`, `sbin`, `usr` is unchanged, and `execve` still works. The payload contains only a **benign canary**, never `lib32/`, so a jail regression yields a red test instead of a dead sandbox. |
| `…::test_jail_fails_closed_without_unshare` | With `unshare` removed from `PATH`, exit is 3 and the payload marker is **never created**. |
| `…::test_jail_keeps_caller_owned_trees_writable` | The jail does not break legitimate staging writes. |
| `…::test_gate_rejects_the_exact_historical_catastrophe` | The literal command from the incident is classified `live-root-extraction`. |
| `…::test_repository_has_no_unguarded_live_root_writes` | Repo-wide scan is clean. |
| `tests/corpus/test_i386_multilib_safety.py::test_sourcing_exposes_helpers_without_running_the_cli` | Sourcing the provisioner defines helpers without running `main`, consulting apt, or requiring a staged `env.sh`; removing the source/execute guard fails the test. |
| `…::test_generated_wrappers_embed_an_absolute_driver_path` | A bare driver name is resolved through `type -P` to an executable file (not a shell function/builtin), embedded absolutely, and the wrapper still runs with the generating toolchain removed from `PATH`. |
| `…::RecoveryScriptTests` | The post-wipe recovery script has no unaudited multilib transaction and uses the 4 GiB helper. |
| `tests/corpus/test_i386_multilib_safety.py` | Sysroot path validation, apt-simulation audit, restore-script constraints, and the provisioner's own 9-proof self-test. |
| `tests/corpus/test_valgrind_provisioning.py::test_bootstrap_installs_i386_kernel_headers_only_on_disposable_runners` | The runner/sandbox split is real, annotated, and the sandbox branch *must* call the audited provisioner. |

## 7. Alternatives considered and why they lost

* **`bwrap`/`proot`/`fakeroot`** — not present in the sandbox, and adding a
  download dependency to a *safety* mechanism is backwards. `unshare` is in
  util-linux, already installed, and the kernel supports unprivileged user
  namespaces here (`max_user_namespaces = 15734`).
* **A wrapper that rewrites `/` to a staging dir in the command line** — clever,
  but it silently changes what the caller asked for and cannot cover `tar`,
  `cp`, maintainer scripts, or anything spawned indirectly. The jail enforces
  the property instead of guessing the intent.
* **Refusing all extraction outright** — would lose i686 coverage, which is a
  hard requirement.
* **Only documenting the safe recipe** — that is exactly what failed three
  times. Documentation is layer 4, not layer 1.

The chosen design is the only one that is simultaneously *structural*
(kernel-enforced), *fail-closed*, *coverage-preserving*, *offline*, and *fast*
(1.4 s).
