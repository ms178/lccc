# ICX 2026.2 in the agent sandbox: reproducibility record

Requirement: select technical solutions with three oracles — GCC 16.2, Clang
23.1.5 and ICX 2026.2.  This repository's CI has an authenticated ICX lane;
the agent sandbox does not.  This file records every attempted local
provisioning path and its **observed** outcome, so that no claim in the PR
rests on an untested assumption.

## What is actually available locally

`/home/user/.cache/lccc/agent-toolchains/env.sh` (cache tag
`d66c5ac881e2`, release asset `harness-cache-8`) installs:

| Tool | Version | Verified |
| --- | --- | --- |
| `rustc` / `cargo` | 1.99.1 | `rustc --version` |
| `gcc-16` | 16.2.0 | `gcc-16 --version` |
| `clang-23` | 23.1.5 | `clang-23 --version` |
| `gdb` | 18.1 | `gdb --version` |
| `binutils` | 2.47 | `as --version` |
| `mold` | 3.0.0 | `mold --version` |
| `qemu-user` | 11.1.3 x86_64 | extracted, not used for native x86_64 |

`which icx icpx` → **not found**.  The cache manifest has 12 archives and
none contains a compiler driver from Intel.  `dpcpp-cpp-rt` and
`intel-cmplr-lib-rt` wheels are runtime libraries only (see below).

## Provisioning attempts (all observed, all failed for a stated reason)

1. **Intel oneAPI apt repository** —
   `curl -fsSL --max-time 20 https://apt.repos.intel.com/oneapi/dists/deb/Release`
   → exit 35, `OpenSSL SSL_connect: SSL_ERROR_SYSCALL`, `HTTP=000`.
   The sandbox egress allowlist is limited to `github.com`,
   `codeload.github.com`, `api.github.com`, `registry.npmjs.org`,
   `pypi.org`, `files.pythonhosted.org`.  `apt.repos.intel.com` is not on it.
   A `HTTP=000` result is a *connection failure*, not an HTTP status, and is
   not evidence about the repository's contents.  (Note: `curl -w
   %{http_error}` is not a valid `curl` variable; use `%{errormsg}`.)

2. **GitHub release asset of `intel/llvm`** — the newest nightly at the time
   of the audit was `nightly-2026-11-04` (published 2026-11-05T04:48:38Z),
   asset `sycl_linux.tar.gz`, id `712104013`, 453,468,611 bytes; that bundle
   is the only candidate that actually contains `icx`/`icpx`.
   - `gh api -H 'Accept: application/octet-stream' -H 'Range: bytes=0-1023'`
     → 302 to `release-assets.githubusercontent.com`, then
     `Get "...": EOF`; zero bytes received.
   - `curl -I https://github.com/intel/llvm/releases/download/.../sycl_linux.tar.gz`
     → 302 to `objects.githubusercontent.com/...`.
   - `curl -L` and a direct request to `release-assets.githubusercontent.com`
     → exit 35 / `HTTP=000`.  Both redirect hosts are outside the allowlist.

3. **Repository release tag cache** —
   `gh api repos/ms178/lccc/releases/tags/harness-cache-8` → HTTP 404
   (`{"message":"Not Found"}`); the toolchain cache is *not* a GitHub release
   tag of this name.  `gh release list` only exposes `harness-cache-4`.
   Do not guess tag names; use `git ls-remote --tags` plus the cache lock.

4. **PyPI** — `https://pypi.org/simple/icx/` and
   `https://pypi.org/pypi/icx/json` return HTTP 200, but `icx 1.0.2` is a
   Streamlit individual-fairness dashboard, not a compiler.
   `intel-llvm`, `icx-compiler`, `oneapi-compiler`, `icpx`, `intel-oneapi`
   simple indexes → 404.  `dpcpp-cpp-rt` 2026.2.0 wheels are 28,739 B /
   58,376 B (runtime shared objects, no `icx` driver); `intel-cmplr-lib-rt`
   is likewise runtime-only.  Simply-index 404s are authoritative negative
   results for those distribution names.

5. **npm** — `https://registry.npmjs.org/icx/latest` → `0.0.1`, a stub
   package with no compiler; `icpx` and `@intel/oneapi-compilers` → 404.

6. **Code search for a vendored binary** —
   `gh api -X GET search/code -f q='icx filename:icx'` → 219,136 hits, all
   noise; no usable local path.

7. **Restore scripts** — `scripts/restore_agent_toolchain_cache.py` and
   `scripts/ensure_gcc_torture_git.py` were rerun after a harness wipe; both
   exit 0 and restore the 12 archives listed above, still with no ICX.

## Consequence for the PR's evidence

* All local *differential* measurements are two-way: GCC 16.2.0 and Clang
  23.1.5.  No local result is described as a three-way comparison, and no
  local result is attributed to ICX.
* Hosted CI does provide the third oracle.  The `icx-oracle` job in
  `.github/workflows/ci.yml` installs ICX 2026.2 through
  `scripts/ensure_icx_2026.sh` (authenticated secret `LCCC_ICX_BIN`) and runs
  the `**/*icx*`, `**/icx_runtime*`, `**/check_icx*` gates in the
  `host-prerequisites` shard.  Its status is verified per head with
  `gh pr checks <pr>` and `gh api repos/ms178/lccc/actions/runs/<id>/jobs`.
  A green hosted ICX lane is *hosted* evidence only; it is never reported as
  a local measurement.
* If a three-way local decision were required, the correct next step is to
  add ICX to the pinned toolchain cache archive (an upload to the release
  asset from a network that can reach Intel), not to substitute a different
  compiler and label it ICX.

## Verification commands

```sh
which icx icpx || echo 'ICX_MISSING'
curl -sS --max-time 20 -o /dev/null -w '%{http_code} %{errormsg}\n' \
  https://apt.repos.intel.com/oneapi/dists/deb/Release
gh api repos/intel/llvm/releases --jq '.[0] | {tag_name,published_at}'
gh api repos/ms178/lccc/actions/runs/<run-id>/jobs \
  --jq '.jobs[] | select(.name|test("icx";"i")) | {name,conclusion}'
```
