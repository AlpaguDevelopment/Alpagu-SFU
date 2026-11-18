"""Collect notices from locked Rust dependencies for a release archive."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
from urllib.request import urlopen

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', required=True, type=Path)
args = parser.parse_args()
repo = Path(__file__).resolve().parent.parent
metadata = json.loads(subprocess.check_output([
    'cargo', 'metadata', '--manifest-path', str(repo / 'core/Cargo.toml'),
    '--locked', '--format-version', '1', '--filter-platform', 'x86_64-pc-windows-msvc',
], encoding='utf-8'))
args.output.mkdir(parents=True, exist_ok=True)
rows = ['# Rust dependency licenses', '', '| Package | License expression | Files |', '|---|---|---|']
missing = []
# Some published crates omit their license files. Pin fallbacks to their recorded VCS commits.
fallbacks = {
    'asn1-rs-impl-0.2.0': ('rusticata/asn1-rs', 'a20e5f7319c896737ad0f2557037817b91ad854f', ['LICENSE-MIT', 'LICENSE-APACHE']),
    'crc-catalog-2.5.0': ('akhilles/crc-catalog', 'ed4ad631f22b05055c21a3a4127eb7cf6d75bb62', ['LICENSES/MIT.txt', 'LICENSES/Apache-2.0.txt']),
}
for key in ['is-0.12.0', 'str0m-proto-0.8.0', 'str0m-rust-crypto-0.7.0']:
    fallbacks[key] = ('algesten/str0m', '5f2183ce7c67117cd1960dfeb11bed11fc188b66', ['LICENSE-MIT.txt'])
for package in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
    if not package['source']:
        continue
    root = Path(package['manifest_path']).parent
    key = package['name'] + '-' + package['version']
    copied = []
    for path in sorted(root.rglob('*')):
        if path.is_symlink() or not path.is_file():
            continue
        name = path.name.lower()
        if not (name.startswith(('license', 'licence', 'copying', 'notice'))):
            continue
        relative = path.relative_to(root)
        target = args.output / key / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, target)
        copied.append(relative.as_posix())
    if not copied and key in fallbacks:
        repository, revision, files = fallbacks[key]
        provenance = []
        for filename in files:
            url = f'https://raw.githubusercontent.com/{repository}/{revision}/{filename}'
            target = args.output / key / filename
            target.parent.mkdir(parents=True, exist_ok=True)
            with urlopen(url, timeout=30) as response:
                target.write_bytes(response.read())
            copied.append(filename)
            provenance.append(url)
        (args.output / key / 'SOURCE.txt').write_text('\n'.join(provenance) + '\n', encoding='utf-8')
    if not copied:
        missing.append(key)
    expression = (package['license'] or 'See package license file').replace('|', '\\|')
    rows.append(f'| {key} | {expression} | [{len(copied)} files]({key}/) |')
(args.output / 'INDEX.md').write_text('\n'.join(rows) + '\n', encoding='utf-8')
if missing:
    raise SystemExit('Missing notices: ' + ', '.join(missing))
print(f'Collected notices for {len(rows) - 4} packages')
