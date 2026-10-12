"""Owned Linux native migration qualification; run in database-dx.Dockerfile."""
import os,pathlib,shutil,subprocess,tempfile,socket,time,json
cli='/work/database-dx-linux-target/debug/axiom-cli'
for major in [17,18]:
    work=pathlib.Path(tempfile.mkdtemp(prefix=f'axiom-dx-pg{major}-'))
    project=work/'project';shutil.copytree('/workspace/examples/simple-database',project,ignore=shutil.ignore_patterns('.axiom','.local','__pycache__'))
    subprocess.run(['chown','-R','axiom:axiom',str(work)],check=True)
    tools=pathlib.Path(f'/usr/lib/postgresql/{major}/bin');data=work/'data'
    subprocess.run(['runuser','-u','axiom','--',str(tools/'initdb'),'-D',str(data),'--auth=trust','--encoding=UTF8','--locale=C','--username=dx_admin'],check=True,stdout=subprocess.DEVNULL)
    sock=socket.socket();sock.bind(('127.0.0.1',0));port=sock.getsockname()[1];sock.close()
    server=subprocess.Popen(['runuser','-u','axiom','--',str(tools/'postgres'),'-D',str(data),'-h','127.0.0.1','-p',str(port),'-c','unix_socket_directories='],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    endpoint=f'postgresql://dx_admin@127.0.0.1:{port}/postgres?sslmode=disable'
    env=os.environ.copy();env['DATABASE_URL']=endpoint;env['AXIOM_DATABASE_KEY_DIR']=str(work/'keys')
    for key in ['PG_BIN','AXIOM_DATABASE_POSTGRES_BIN','AXIOM_DATABASE_ENGINE_DIR']:env.pop(key,None)
    def call(*args,expected=0):
        result=subprocess.run(['runuser','-u','axiom','--',cli,*args],cwd=project,env=env,capture_output=True,text=True,timeout=90)
        print(major,' '.join(args),result.returncode,result.stdout.strip(),result.stderr.strip(),flush=True)
        assert result.returncode==expected
        return result
    try:
        for _ in range(100):
            try:
                with socket.create_connection(('127.0.0.1',port),timeout=.2):break
            except OSError:time.sleep(.1)
        call('database','migrate','database/schema.acore','--non-interactive')
        call('run','database/schema.acore','--no-shell','--execute',"INSERT INTO app.tasks(tenant,id,title) VALUES('demo','one','retained');")
        source=project/'database/schema.acore';source.write_text(source.read_text().replace('title: Pg.VarChar(length: 120)','title: Pg.VarChar(length: 120) note: Pg.Text(nullable: true)'))
        call('database','migrate','database/schema.acore','--non-interactive',expected=1)
        call('database','migrate','database/schema.acore','--auto-version','--non-interactive','--export','release.axiom')
        call('database','rollback','database/schema.acore','--steps','1','--auto-version','--non-interactive','--allow-destructive')
        call('run','database/schema.acore','--auto-version','--non-interactive','--no-shell','--execute','SELECT title FROM app.tasks;')
        call('database','status','database/schema.acore','--check')
        call('database','history','database/schema.acore')
        # Real crashes in qualification builds: before commit leaves the old
        # schema/head; after commit is safely recognized on the next invocation.
        for fault in ['before-commit','after-commit']:
            source.write_text(source.read_text().replace('note: Pg.Text(nullable: true)',
                'note: Pg.Text(nullable: true) '+fault.replace('-','')+': Pg.Text(nullable: true)'))
            env['AXIOM_DATABASE_DX_TEST_FAULT']=fault
            call('database','migrate','database/schema.acore','--auto-version','--non-interactive',expected=93)
            env.pop('AXIOM_DATABASE_DX_TEST_FAULT')
            snapshot=json.loads(call('database','status','database/schema.acore','--json').stdout)
            assert not snapshot['drift']
            assert snapshot['pending']==(fault=='before-commit')
            call('database','migrate','database/schema.acore','--non-interactive')
            call('database','status','database/schema.acore','--check')
        print(f'PostgreSQL {major} Linux native qualification and commit crash recovery passed',flush=True)
    finally:
        server.terminate();server.wait(timeout=20)
