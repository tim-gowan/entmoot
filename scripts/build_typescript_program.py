#!/usr/bin/env python3
"""Install the pinned in-repo TypeScript Program adapter. Explicit setup only."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--version-tools', type=Path, help='also install the pinned historical compiler controls')
    args = parser.parse_args()
    out = args.out.resolve()
    out.mkdir(parents=True, exist_ok=True)
    (out / 'package.json').write_text(json.dumps({'private': True, 'dependencies': {
        'typescript': '6.0.3', '@noble/hashes': '1.8.0'}}), encoding='utf-8')
    shutil.copyfile(Path(__file__).with_name('typescript_program.package-lock.json'), out / 'package-lock.json')
    subprocess.run([shutil.which('npm.cmd' if os.name == 'nt' else 'npm'), 'ci',
                    '--ignore-scripts', '--no-audit', '--no-fund'], cwd=out, check=True)
    source = Path(__file__).resolve().parents[1] / 'crates/entmoot-native/src/native_resolver/producers/typescript_program.cjs'
    shutil.copyfile(source, out / 'entmoot-typescript.cjs')
    # Explicit reference producer shares only the pinned hashing runtime.
    shutil.copyfile(Path(__file__).resolve().parents[1] / 'tools/lock-instance-providers/npm.cjs', out / 'npm-lock-provider.cjs')
    if args.version_tools:
        versions = args.version_tools.resolve()
        if versions == out:
            parser.error('--version-tools must differ from --out')
        versions.mkdir(parents=True, exist_ok=True)
        lock = Path(__file__).with_name('native_language_versions.package-lock.json')
        document = json.loads(lock.read_text(encoding='utf-8'))
        (versions / 'package.json').write_text(json.dumps(document['packages']['']), encoding='utf-8')
        shutil.copyfile(lock, versions / 'package-lock.json')
        subprocess.run([shutil.which('npm.cmd' if os.name == 'nt' else 'npm'), 'ci',
                        '--ignore-scripts', '--no-audit', '--no-fund'], cwd=versions, check=True)
    print(out / 'entmoot-typescript.cjs')


if __name__ == '__main__':
    main()
