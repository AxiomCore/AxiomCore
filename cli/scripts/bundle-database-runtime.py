#!/usr/bin/env python3
"""Build a relocatable CLI distribution with private PostgreSQL validators.

Release maintainers supply qualified PostgreSQL prefixes; application developers
only run the resulting axiom executable. This never provisions their target DB.
"""
import argparse, hashlib, json, os, platform, shutil, subprocess, urllib.request
from pathlib import Path


def run(*args):
    return subprocess.check_output(args, text=True).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cli', type=Path, required=True)
    parser.add_argument('--postgres-prefix', type=Path, action='append', required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--ui-cache', type=Path, help='qualified matching web host cache')
    args = parser.parse_args()
    if args.out.exists():
        parser.error('distribution destination already exists; choose a new immutable destination')
    args.out.mkdir(parents=True)
    shutil.copy2(args.cli, args.out / 'axiom')
    if args.ui_cache:
        cache=args.out/'ui-cache'
        shutil.copytree(args.ui_cache/'ui-hosts',cache/'ui-hosts')
        for receipt in (cache/'ui-hosts').glob('*/host.json'):
            installation=json.loads(receipt.read_text())
            artifact=installation.get('artifact')
            if not artifact:
                parser.error('bundled UI host needs an immutable artifact receipt')
            matches=[p for p in receipt.parent.rglob(artifact['file'])
                     if hashlib.sha256(p.read_bytes()).hexdigest()==artifact['sha256']]
            if len(matches)!=1:
                parser.error('bundled UI host artifact is missing or ambiguous')
            installation['hostRoot']=str(matches[0].parent.resolve())
            receipt.write_text(json.dumps(installation,sort_keys=True)+'\n')
    for prefix in args.postgres_prefix:
        version = run(str(prefix / 'bin/postgres'), '--version').split()[-1]
        major = version.split('.')[0]
        if major not in ['17', '18']:
            parser.error('only PostgreSQL 17/18 validators are supported')
        engine = args.out / 'engines/postgres' / major
        for folder in ['bin', 'lib', 'share']:
            shutil.copytree(prefix / folder, engine / folder, symlinks=False)
            for asset in (engine/folder).rglob('*'):
                if asset.is_file():asset.chmod(asset.stat().st_mode|0o200)
        licenses = engine / 'licenses'; licenses.mkdir()
        if (prefix/'licenses').is_dir(): shutil.copytree(prefix/'licenses', licenses, dirs_exist_ok=True)
        if not (licenses/'PostgreSQL.txt').exists():
            licenses.joinpath('PostgreSQL.txt').write_bytes(urllib.request.urlopen(
                f'https://raw.githubusercontent.com/postgres/postgres/REL_{major}_STABLE/COPYRIGHT').read())
        if platform.system() == 'Darwin':
            # Copy the exact linked OpenSSL runtime and remove Homebrew paths.
            binaries = [p for folder in ['bin','lib'] for p in (engine/folder).rglob('*') if p.is_file()]
            dependencies = set()
            for binary in binaries:
                info = subprocess.run(['otool','-L',str(binary)], text=True, capture_output=True)
                for line in info.stdout.splitlines()[1:]:
                    name = line.strip().split(' (')[0]
                    if name.startswith('/opt/homebrew/') and Path(name).is_file(): dependencies.add(name)
            for name in list(dependencies):
                library = Path(name)
                destination=engine / 'lib' / library.name
                if destination.exists():destination.unlink()
                shutil.copy2(library,destination);destination.chmod(destination.stat().st_mode|0o200)
                if 'openssl' in str(library):
                    license_file = library.parent.parent / 'LICENSE.txt'
                    if license_file.is_file(): shutil.copy2(license_file, licenses / 'OpenSSL.txt')
            for binary in binaries + [engine/'lib'/Path(name).name for name in dependencies]:
                info = subprocess.run(['otool','-L',str(binary)], text=True, capture_output=True)
                for line in info.stdout.splitlines()[1:]:
                    name=line.strip().split(' (')[0]
                    if name in dependencies:
                        relative='@executable_path/../lib/' if binary.parent.name=='bin' else '@loader_path/'
                        subprocess.check_call(['install_name_tool','-change',name,relative+Path(name).name,str(binary)], stdout=subprocess.DEVNULL)
                if binary.suffix=='.dylib':subprocess.check_call(['install_name_tool','-id','@rpath/'+binary.name,str(binary)],stdout=subprocess.DEVNULL)
                # Modified arm64 executables need a fresh ad-hoc signature.
                if info.returncode==0: subprocess.run(['codesign','--force','--sign','-',str(binary)],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        files={str(p.relative_to(engine)):hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(engine.rglob('*')) if p.is_file()}
        (engine/'runtime.axiom').write_text(json.dumps({'format':'axiom-postgres-runtime/v1','version':version,'os':platform.system().lower(),'arch':platform.machine(),'files':files},sort_keys=True))
        assert run(str(engine/'bin/postgres'),'--version').endswith(version)
    print(args.out)

if __name__ == '__main__': main()
