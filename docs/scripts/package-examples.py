"""Create deterministic, source-only documentation downloads; --check rejects drift."""
import argparse
import re
from io import BytesIO
from pathlib import Path
from zipfile import ZipFile, ZipInfo, ZIP_DEFLATED

DOCS = Path(__file__).resolve().parents[1]
SOURCE = DOCS.parent / 'examples' / 'documentation'
OUTPUT = DOCS / 'public' / 'examples'
NAMES = ('task-app', 'exam-app', 'hello-api', 'managed-title', 'fastapi-web', 'go-web')
SUFFIXES = {'.acore', '.toml', '.md', '.py', '.txt', '.go', '.mod', '.sum', '.json', '.ts', '.mjs', '.html'}
FORBIDDEN = {'node_modules', '.axiom', '.venv', 'generated', 'public', 'dist', '__pycache__', '.git'}


def archive(name):
    root = SOURCE / name
    output = BytesIO()
    with ZipFile(output, 'w', compression=ZIP_DEFLATED, compresslevel=9) as bundle:
        for path in sorted(root.rglob('*')):
            if not path.is_file():
                continue
            relative = path.relative_to(root)
            if any(part in FORBIDDEN for part in relative.parts):
                raise ValueError(f'Remove generated or private files from example source: {path}')
            if path.is_symlink() or (path.name != '.gitignore' and path.suffix not in SUFFIXES):
                raise ValueError(f'Unrecognized example source: {path}')
            if path.name.startswith('.env') or 'deployment' in path.name or path.suffix == '.axiom':
                raise ValueError(f'Runtime or deployment material is not source: {path}')
            content = path.read_text()
            if re.search(r'/Users/|/home/|\.task-validation|\u2014', content):
                raise ValueError(f'Personal path, task state, or em dash in public example: {path}')
            info = ZipInfo(f'{name}/{relative.as_posix()}', date_time=(2026, 1, 1, 0, 0, 0))
            info.compress_type = ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            bundle.writestr(info, path.read_bytes(), compresslevel=9)
    return output.getvalue()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    OUTPUT.mkdir(parents=True, exist_ok=True)
    for name in NAMES:
        destination = OUTPUT / f'{name}.zip'
        expected = archive(name)
        if args.check:
            if not destination.exists() or destination.read_bytes() != expected:
                raise SystemExit(f'Stale download: {destination}. Run pnpm examples:package.')
        else:
            destination.write_bytes(expected)
        print(f'{name}: {len(expected)} bytes, source-only archive verified')
