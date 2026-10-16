#!/usr/bin/env python3
"""Rootless ICX 2026.2 provisioning from Intel's authenticated APT repository.

The first install selects the newest 2026.2.x build, then freezes its five
package versions, SHA-256 digests and compiler identity in a private manifest.
An existing prefix is a reproducible snapshot, not a moving 'latest' symlink.
Use a new prefix to explicitly refresh that snapshot. No host APT state changes.
"""
from __future__ import annotations

import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import urllib.parse
import urllib.request

RELEASE = "2026.2"
REPOSITORY = "https://apt.repos.intel.com/oneapi"
KEY_URL = "https://apt.repos.intel.com/intel-gpg-keys/GPG-PUB-KEY-INTEL-SW-PRODUCTS.PUB"
# Compiler + CPU-only C/C++ support, not the entire oneAPI toolkit.
PACKAGES = tuple(f"intel-oneapi-{stem}-{RELEASE}" for stem in (
    "dpcpp-cpp", "compiler-shared-runtime", "compiler-shared",
    "compiler-dpcpp-cpp-common", "compiler-shared-common",
))
MANIFEST = ".lccc-icx.json"


class ProvisionError(RuntimeError):
    pass


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run(args: list[str], *, env: dict[str, str] | None = None,
        cwd: Path | None = None, timeout: int = 120) -> str:
    result = subprocess.run(args, env=env, cwd=cwd, text=True,
                            capture_output=True, timeout=timeout)
    if result.returncode:
        raise ProvisionError(f"{' '.join(args)} failed: {result.stderr.strip()}")
    return result.stdout


def semver(version: str) -> str:
    match = re.fullmatch(r"(2026\.2\.[0-9]+)(?:-[A-Za-z0-9.+~]+)?", version)
    if not match or int(match[1].rsplit('.', 1)[1]) > 99:
        raise ProvisionError(f"not a supported ICX {RELEASE} package version: {version!r}")
    return match[1]


def macro_for(version: str) -> str:
    major, minor, patch = (int(n) for n in semver(version).split('.'))
    return str(major * 10000 + minor * 100 + patch)


def package_record(text: str, name: str, version: str) -> dict[str, str]:
    # Debian control fields can have continuation lines; they are irrelevant to
    # the identity fields here, but must not be parsed as independent fields.
    records = []
    for stanza in text.strip().split('\n\n'):
        fields = dict(line.split(': ', 1) for line in stanza.splitlines()
                      if line and not line[0].isspace() and ': ' in line)
        if fields.get('Package') == name and fields.get('Version') == version:
            records.append(fields)
    if len(records) != 1:
        raise ProvisionError(f"expected one authenticated package record for {name}={version}")
    fields = records[0]
    if fields.get('Architecture') not in ('amd64', 'all'):
        raise ProvisionError(f"unsupported package architecture for {name}")
    if not re.fullmatch(r'[0-9a-f]{64}', fields.get('SHA256', '')):
        raise ProvisionError(f"missing SHA-256 for {name}")
    filename = fields.get('Filename', '')
    if not filename.startswith('pool/') or '..' in filename.split('/'):
        raise ProvisionError(f"unsafe package filename for {name}")
    return {key: fields[key] for key in
            ('Package', 'Version', 'Architecture', 'Filename', 'SHA256')}


def apt_environment(work: Path, repository: str) -> dict[str, str]:
    parsed = urllib.parse.urlsplit(repository)
    if (parsed.scheme != 'https' or not parsed.netloc or parsed.query or parsed.fragment
            or any(c.isspace() or c in '\"\\[]' for c in repository)):
        raise ProvisionError('LCCC_ICX_MIRROR must be a plain HTTPS repository URL')
    for sub in ('etc/empty', 'state/lists/partial', 'cache/archives/partial', 'log', 'downloads'):
        (work / sub).mkdir(parents=True)
    key = work / 'etc/intel.asc'
    # Bootstrap trust from Intel's HTTPS-published key, as Intel's installation
    # instructions do. APT then verifies signed metadata and package checksums.
    # We do NOT claim a separately pinned signing-key fingerprint.
    with urllib.request.urlopen(KEY_URL, timeout=30) as response:
        key_bytes = response.read(1 << 20)
    if (not key_bytes.startswith(b'-----BEGIN PGP PUBLIC KEY BLOCK-----')
            or b'-----END PGP PUBLIC KEY BLOCK-----' not in key_bytes):
        raise ProvisionError('Intel signing-key response is not an armored public key')
    key.write_bytes(key_bytes)
    (work / 'etc/sources.list').write_text(
        f'deb [arch=amd64 signed-by={key}] {repository} all main\n')
    config = work / 'apt.conf'
    config.write_text(f'''
Dir::Etc "{work / 'etc'}";
Dir::Etc::parts "empty";
Dir::Etc::main "absent";
Dir::Etc::sourcelist "sources.list";
Dir::Etc::sourceparts "empty";
Dir::Etc::trusted "absent.gpg";
Dir::Etc::trustedparts "empty";
Dir::State "{work / 'state'}";
Dir::State::status "/dev/null";
Dir::Cache "{work / 'cache'}";
Dir::Log "{work / 'log'}";
APT::Architecture "amd64";
APT::Architectures {{ "amd64"; }};
Acquire::Languages "none";
Acquire::Retries "2";
Acquire::https::Timeout "30";
Acquire::AllowInsecureRepositories "false";
Acquire::AllowDowngradeToInsecureRepositories "false";
APT::Get::AllowUnauthenticated "false";
''')
    return {**os.environ, 'APT_CONFIG': str(config), 'LC_ALL': 'C'}


def resolve(work: Path, repository: str) -> dict:
    env = apt_environment(work, repository)
    run(['apt-get', 'update', '--error-on=any'], env=env, timeout=300)
    policy = run(['apt-cache', 'policy', PACKAGES[0]], env=env)
    candidate = re.search(r'^\s*Candidate:\s*(\S+)\s*$', policy, re.MULTILINE)
    if candidate is None:
        raise ProvisionError(f'Intel repository has no {RELEASE} compiler candidate')
    version = candidate[1]
    release = semver(version)  # Refuse older/newer release lines, never silently fall back.
    packages = [package_record(run(['apt-cache', 'show', f'{name}={version}'], env=env),
                               name, version) for name in PACKAGES]
    downloads = work / 'downloads'
    run(['apt-get', 'download', *(f'{name}={version}' for name in PACKAGES)],
        env=env, cwd=downloads, timeout=1800)
    files = {sha256(p): p for p in downloads.glob('*.deb')}
    for package in packages:
        archive = files.get(package['SHA256'])
        if archive is None:
            raise ProvisionError(f"missing or corrupt archive for {package['Package']}")
        run(['dpkg-deb', '-x', str(archive), str(work / 'install')])
    return {
        'release_line': RELEASE, 'semver': release, 'repository': repository,
        'key_url': KEY_URL, 'key_sha256': sha256(work / 'etc/intel.asc'),
        'packages': packages,
        'apt_metadata_sha256': {p.name: sha256(p) for p in (work / 'state/lists').iterdir()
                                if p.is_file() and p.name != 'lock'},
    }


def probe(binary: Path) -> str:
    output = run([str(binary), '-E', '-dM', '-x', 'c', os.devnull], timeout=60)
    match = re.search(r'^#define __INTEL_LLVM_COMPILER ([0-9]+)$', output, re.MULTILINE)
    if match is None:
        raise ProvisionError(f'{binary} did not report __INTEL_LLVM_COMPILER')
    return match[1]


def validate(prefix: Path) -> Path:
    data = json.loads((prefix / MANIFEST).read_text())
    if data.get('release_line') != RELEASE:
        raise ProvisionError(f'{prefix} is not an ICX {RELEASE} snapshot')
    binary = (prefix / data['binary']).resolve()
    if not binary.is_relative_to(prefix.resolve()):
        raise ProvisionError('snapshot binary escapes its prefix')
    if sha256(binary) != data['binary_sha256']:
        raise ProvisionError('snapshot compiler digest changed')
    if probe(binary) != macro_for(data['semver']):
        raise ProvisionError('snapshot compiler version probe changed')
    return binary


def install(prefix: Path) -> Path:
    prefix = prefix.expanduser().resolve()
    if prefix == prefix.parent:
        raise ProvisionError('refusing a filesystem root as the install prefix')
    prefix.parent.mkdir(parents=True, exist_ok=True)
    # Cross-process lock, not just the Python oracle registry's thread lock.
    with prefix.with_name(prefix.name + '.lock').open('a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if prefix.exists():
            return validate(prefix)
        with tempfile.TemporaryDirectory(prefix='.lccc-icx-', dir=prefix.parent) as tmp:
            work = Path(tmp)
            data = resolve(work, os.environ.get('LCCC_ICX_MIRROR', REPOSITORY))
            staging = work / 'install'
            binaries = [p for p in staging.rglob('icx') if p.is_file() and p.parent.name == 'bin']
            if len(binaries) != 1:
                raise ProvisionError(f'expected exactly one icx binary, found {len(binaries)}')
            binary = binaries[0]
            if probe(binary) != macro_for(data['semver']):
                raise ProvisionError('extracted compiler disagrees with authenticated package version')
            data.update(binary=str(binary.relative_to(staging)), binary_sha256=sha256(binary))
            (staging / MANIFEST).write_text(json.dumps(data, indent=2) + '\n')
            # Do not publish a broken prefix. Validation and probing precede
            # the atomic rename on the same filesystem; failures clean staging.
            validate(staging)
            staging.rename(prefix)
            return prefix / data['binary']


def main(argv: list[str] | None = None) -> int:
    args = sys.argv[1:] if argv is None else argv
    if len(args) > 1 or (args and args[0].startswith('-')):
        print('usage: ensure_icx_2026.sh [PREFIX]', file=sys.stderr)
        return 2
    default = os.environ.get('LCCC_ICX_PREFIX', str(Path.home() / '.cache/intel-icx-2026.2'))
    try:
        print(install(Path(args[0] if args else default)))
    except (ProvisionError, OSError, ValueError, KeyError, subprocess.TimeoutExpired) as error:
        print(f'ICX {RELEASE} provisioning failed: {error}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
