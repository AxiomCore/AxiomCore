#!/usr/bin/env python3
"""Qualify actual CLI authoring, approved interfaces and hidden state offline."""
import argparse, hashlib, json, shutil, subprocess, tempfile
from pathlib import Path

parser=argparse.ArgumentParser(description=__doc__)
parser.add_argument('--cli',type=Path,required=True)
parser.add_argument('--report',type=Path,required=True)
args=parser.parse_args();cli=args.cli.resolve();workspace=Path(__file__).resolve().parents[2]
args.report.parent.mkdir(parents=True,exist_ok=True)
checks=[]
def digest(path):return hashlib.sha256(path.read_bytes()).hexdigest()
def run(root,*tokens,ok=True):
 result=subprocess.run([str(cli),*map(str,tokens)],cwd=root,capture_output=True,text=True,timeout=120)
 assert (result.returncode==0)==ok,(tokens,result.stdout[-1000:],result.stderr[-2000:])
 return result
with tempfile.TemporaryDirectory(prefix='ship-interface-',dir=args.report.parent) as temporary:
 root=Path(temporary)
 for name,source in [('before','simple-database'),('after','ship-ergonomics')]:
  shutil.copytree(workspace/'examples'/source,root/name,ignore=shutil.ignore_patterns('.axiom','.local','__pycache__'))
  run(root/name,'check','backend/main.acore');run(root/name,'build','backend/main.acore')
 before=json.loads((root/'before/backend/.axiom/contracts/backend.axiom').read_text())
 after=json.loads((root/'after/backend/.axiom/contracts/backend.axiom').read_text())
 for key in ['ir','endpoints','failure_contracts','language','ui_contract_metadata','domain_manifest']:
  assert before.get(key)==after.get(key),f'Explicit/inferred public contract differs at {key}'
 checks.append('Explicit and concise forms preserve complete public IR, identities, failures, codecs and UI descriptors')
 project=root/'after';artifact=project/'backend/.axiom/contracts/backend.axiom'
 approved=root/'approved.axiom';shutil.copy2(artifact,approved)
 accepted_artifact_sha256=digest(approved)
 run(project,'build','backend/main.acore');assert digest(artifact)==digest(approved)
 checks.append('Repeated native compilation is byte deterministic')
 run(project,'check','backend/main.acore','--frozen-interface',approved)
 baseline={str(p.relative_to(project)):digest(p) for p in project.rglob('*') if p.is_file() and '.axiom' in p.parts}
 application=project/'backend/application.acore';original=application.read_text()
 for label,old,new in [
  ('new closed failure','failure unavailable(payload: Models.Problem, status: 503)','failure unavailable(payload: Models.Problem, status: 503)\nfailure changed(payload: Models.Problem,status: 422)'),
  ('failure status','status: 409','status: 410'),
  ('public projection','fields: [.id, .title]','fields: [.id]'),
  ('response field','maxLength: 120','maxLength: 119'),
 ]:
  changed=original.replace(old,new)
  if label=='new closed failure':changed=changed.replace('errors: [Failures.duplicate]','errors: [Failures.duplicate, Failures.changed]')
  assert changed!=original
  application.write_text(changed)
  for command in ['check','build']:
   result=run(project,command,'backend/main.acore','--frozen-interface',approved,ok=False)
   assert 'AXINTERFACE001' in result.stdout+result.stderr,(label,result.stdout,result.stderr)
  now={str(p.relative_to(project)):digest(p) for p in project.rglob('*') if p.is_file() and '.axiom' in p.parts}
  assert baseline==now,'Rejected frozen operation modified accepted state'
  application.write_text(original)
  checks.append(f'Approved interface rejects {label} before replacing any retained output')
 schema=project/'database/schema.acore';schema.write_text(schema.read_text().replace('title: Pg.VarChar(length: 120)','title: Pg.VarChar(length: 119)'))
 result=run(project,'build','backend/main.acore','--frozen-interface',approved,ok=False)
 assert 'AXINTERFACE001' in result.stdout+result.stderr
 schema.write_text(schema.read_text().replace('length: 119','length: 120'))
 checks.append('Selected schema drift participates in frozen public checking')
 schema.write_text(schema.read_text().replace('title: Pg.VarChar(length: 120)','title: Pg.VarChar(length: 120)\n      privateNote: Pg.Text(nullable: true)'))
 run(project,'check','backend/main.acore','--frozen-interface',approved)
 assert 'privateNote' not in artifact.read_text()
 checks.append('Unselected private schema changes do not widen the public API')
 # Source checks never migrate a legacy file; builds preserve exact bytes.
 legacy=project/'backend/main.acore.identities.json';legacy.write_bytes(b'{"format":"acore-backend-identities/v1","endpoints":{}}\n')
 preserved=project/'backend/data.sqlite';preserved.write_bytes(b'persistent application data')
 policy=project/'backend/authored.json';policy.write_text('{"authored":true}\n')
 run(project,'build','backend/main.acore')
 assert not legacy.exists()
 assert (project/'backend/.axiom/legacy/main.acore.identities.json').read_bytes()==b'{"format":"acore-backend-identities/v1","endpoints":{}}\n'
 assert preserved.read_bytes()==b'persistent application data' and policy.read_text()=='{"authored":true}\n'
 run(project,'build','backend/main.acore')
 checks.append('Legacy relocation is preserving and idempotent; authored JSON and persistent data remain intact')
 visible=[str(p.relative_to(project)) for p in project.rglob('*') if p.is_file() and '.axiom' not in p.relative_to(project).parts and p.suffix in ['.json','.lock','.lockfile'] and p!=policy]
 assert not visible,visible
 assert not (project/'backend/Task.acore').exists()
 checks.append('Default compilation emits no visible generated JSON or derived source snapshot')
 # Fresh checkout retains only source and required hidden authority.
 fresh=root/'fresh';shutil.copytree(project,fresh,ignore=shutil.ignore_patterns('.axiom','data.sqlite','authored.json'))
 identities=project/'backend/.axiom/identities'
 if identities.exists():shutil.copytree(identities,fresh/'backend/.axiom/identities')
 run(fresh,'check','backend/main.acore','--frozen-interface',approved)
 checks.append('Fresh checkout with retained identity authority checks offline against the approved contract')

def measures(name):
 files=list((workspace/'examples'/name/'backend').glob('*.acore'))+list((workspace/'examples'/name/'frontend').rglob('*.acore'))
 files+=list((workspace/'examples'/name/'backend').rglob('AxiomDeps.toml'))+list((workspace/'examples'/name/'frontend').rglob('AxiomDeps.toml'))
 text='\n'.join(p.read_text() for p in files)
 lines=[line for line in text.splitlines() if line.strip() and not line.strip().startswith(('//','#'))]
 return {'nonCommentLines':len(lines),'characters':sum(len(line) for line in lines),'explicitResponseTypes':text.count('response(type:'),'explicitTransportTypes':sum(text.count(k) for k in ['header("apiKey", type:','path("id", type:','body("task", type:','body("title", type:']),'explicitFailureLists':text.count('errors(Failures.'),'generatedModelSnapshots':int((workspace/'examples'/name/'backend/Task.acore').exists())}
report={'format':'axiom-ship-interface-acceptance/v1','cliSha256':digest(cli),'publicArtifactSha256':accepted_artifact_sha256,'checks':checks,'measurementScope':'Complete authored backend/frontend Acore and manifests, including policies, projections and shared-library overhead; unchanged database schemas excluded','measurement':{'before':measures('simple-database'),'after':measures('ship-ergonomics')}}
args.report.write_text(json.dumps(report,indent=2)+'\n')
print(f'{len(checks)} native interface/hidden-state acceptance checks passed')
