#!/usr/bin/env python3
"""Qualify the downloaded playground handlers against disposable native databases."""
import argparse, importlib.util, json, os, shutil, socket, subprocess, tempfile, time
import urllib.request, urllib.error, urllib.parse
from pathlib import Path

spec=importlib.util.spec_from_file_location('fullstack',Path(__file__).with_name('database-dx-fullstack.py'))
helper=importlib.util.module_from_spec(spec);spec.loader.exec_module(helper)

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cli',type=Path,required=True);parser.add_argument('--report',type=Path,required=True)
    args=parser.parse_args();cli=args.cli.resolve();workspace=Path(__file__).resolve().parents[2]
    results=[]
    for example,major,plural,field in [('database-inventory','17','products','quantity'),('database-tasks','18','tasks','completed'),('database-sqlite',None,'tasks',None)]:
        with tempfile.TemporaryDirectory(prefix='axiom-playground-native-') as folder:
            work=Path(folder);project=work/'project';shutil.copytree(workspace/'axiom-frontend/apps/playground/src/examples'/example,project)
            env=os.environ.copy()
            for key in ['DATABASE_URL','PGPASSWORD','PG_BIN','AXIOM_POSTGRES_BIN','AXIOM_POSTGRES_17_BIN','AXIOM_POSTGRES_18_BIN']:env.pop(key,None)
            processes=[];logs=[]
            def spawn(name,tokens):
                path=work/(name+'.log');output=path.open('w');logs.append(output)
                process=subprocess.Popen(list(map(str,tokens)),cwd=project,env=env,stdout=output,stderr=subprocess.STDOUT);processes.append(process)
                return process,path
            def run(*tokens):
                result=subprocess.run([str(cli),*tokens],cwd=project,env=env,text=True,capture_output=True,timeout=60)
                assert result.returncode==0,result.stderr[-1800:]
                return result.stdout
            try:
                if major:
                    tools=cli.parent/'engines/postgres'/major/'bin';data=work/'postgres';dbport=helper.port()
                    subprocess.run([str(tools/'initdb'),'-D',str(data),'--auth=trust','--encoding=UTF8','--locale=C','--username=dx_operator'],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
                    database,log=spawn('postgres',[tools/'postgres','-D',data,'-h','127.0.0.1','-p',str(dbport),'-c','unix_socket_directories='])
                    env['DATABASE_URL']=f'postgresql://dx_operator@127.0.0.1:{dbport}/postgres?sslmode=disable'
                    for _ in range(100):
                        try:socket.create_connection(('127.0.0.1',dbport),timeout=.2).close();break
                        except OSError:time.sleep(.1)
                apiport=helper.port();endpoint=f'http://127.0.0.1:{apiport}'
                backend,log=spawn('backend',[cli,'run','backend/backend.acore','--port',str(apiport),'--non-interactive'])
                helper.wait_http(endpoint+'/'+plural,backend,log)
                key=(project/'backend/.axiom/database/development.key').read_text().strip()
                if major:
                    table,parent,name=('categories','software','Software') if plural=='products' else ('users','yash','Yash')
                    run('run','database/schema.acore','--no-shell','--execute',f"INSERT INTO app.{table}(tenant,id,name) VALUES('demo','{parent}','{name}');")
                def api(method,body=None,suffix=''):
                    req=urllib.request.Request(endpoint+'/'+plural+suffix,method=method,headers={'apikey':key,'Content-Type':'application/json'},data=json.dumps(body).encode() if body is not None else None)
                    try:response=urllib.request.urlopen(req,timeout=10)
                    except urllib.error.HTTPError as response:return response.code,response.read().decode()
                    return response.status,json.loads(response.read())
                created=api('POST',{'title':'Native mapped record'});assert created[0] in [200,201],created
                identity=created[1]['id'];path='/'+urllib.parse.quote(identity,safe='')
                assert any(r['id']==identity for r in api('GET')[1])
                if field:
                    value=25 if field=='quantity' else True;updated=api('PATCH',{field:value},path);assert updated[0]==200 and updated[1][field]==value,updated
                deleted=api('DELETE',suffix=path);assert deleted[0]==200 and deleted[1]['id']==identity,deleted
                assert api('GET')==(200,[])
                backend.terminate();backend.wait(timeout=15)
                backend,log=spawn('backend-restart',[cli,'run','backend/backend.acore','--port',str(apiport),'--non-interactive']);helper.wait_http(endpoint+'/'+plural,backend,log)
                assert api('GET')==(200,[])
                manifest=project/'frontend/AxiomDeps.toml';manifest.write_text(manifest.read_text().replace('http://127.0.0.1:8080',endpoint))
                uiport=helper.port();frontend,log=spawn('frontend',[cli,'run','frontend/frontend.acore','--port',str(uiport)])
                helper.wait_http(f'http://127.0.0.1:{uiport}',frontend,log,timeout=75)
                visible=[str(p.relative_to(project)) for p in project.rglob('*') if p.is_file() and '.axiom' not in p.parts and p.suffix in ['.json','.lockfile','.lock']];assert not visible,visible
                results.append({'example':example,'engine':'postgres'+major if major else 'sqlite','result':'passed','checks':['native-create','native-delete','restart-deletion','frontend-boot','hidden-state']+(['native-update'] if field else [])})
                print(example,'native handlers/frontend/clean-tree passed',flush=True)
            finally:
                for process in reversed(processes):
                    if process.poll() is None:
                        process.terminate()
                        try:process.wait(timeout=15)
                        except subprocess.TimeoutExpired:process.kill();process.wait()
                for output in logs:output.close()
    args.report.write_text(json.dumps({'format':'axiom-playground-native-handlers-acceptance/v1','results':results},indent=2)+'\n')
if __name__=='__main__':main()
