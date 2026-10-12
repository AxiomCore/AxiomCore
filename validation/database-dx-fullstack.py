#!/usr/bin/env python3
"""Native HTTP and actual browser acceptance for the installed minimal example."""
import argparse
import json
import os
from pathlib import Path
import shutil
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
import urllib.parse

WEB_TEST = r"""
import {chromium} from '@playwright/test';
const browser=await chromium.launch({channel:'chrome',headless:true});
const page=await browser.newPage();
const errors=[];page.on('pageerror',error=>errors.push(String(error)));

try {
 await page.goto(process.env.DX_WEB_URL);
 await page.getByPlaceholder('Local development API key',{exact:true}).fill(process.env.DX_API_KEY);
 await page.getByRole('button',{name:'Connect',exact:true}).click();
 await page.getByRole('textbox',{name:'Task title',exact:true}).fill('Browser persistence');
 await page.getByRole('button',{name:'Add task',exact:true}).click();
 await page.getByText('Browser persistence',{exact:true}).waitFor();
 await page.getByRole('textbox',{name:'Task title',exact:true}).fill('Browser persistence');
 await page.getByRole('button',{name:'Add task',exact:true}).click();
 await page.getByText('That task already exists',{exact:true}).first().waitFor();
 await page.reload();
 await page.getByPlaceholder('Local development API key',{exact:true}).fill(process.env.DX_API_KEY);
 await page.getByRole('button',{name:'Connect',exact:true}).click();
 await page.getByText('Browser persistence',{exact:true}).waitFor();
 await page.getByRole('textbox',{name:'Task title',exact:true}).fill('Browser deletion');
 await page.getByRole('button',{name:'Add task',exact:true}).click();
 const row=page.locator('.task').filter({hasText:'Browser deletion'});
 await row.getByRole('button',{name:'Delete',exact:true}).click();
 await page.getByText('Browser deletion',{exact:true}).waitFor({state:'hidden'});
 await page.reload();
 await page.getByPlaceholder('Local development API key',{exact:true}).fill(process.env.DX_API_KEY);
 await page.getByRole('button',{name:'Connect',exact:true}).click();
 await page.getByText('Browser persistence',{exact:true}).waitFor();
 if(await page.getByText('Browser deletion',{exact:true}).count())throw Error('Deleted row returned after reload');
 if(errors.length)throw new Error(errors.join('\n'));
 console.log('Browser create, declared duplicate failure and reload persistence passed');
} catch(error) {
 console.error(JSON.stringify({errors,body:(await page.locator('body').innerText()).slice(0,1000),inputs:await page.locator('input').evaluateAll(es=>es.map(e=>({type:e.type,placeholder:e.placeholder}))),frames:page.frames().map(f=>f.url())}));
 await page.screenshot({path:process.env.DX_SCREENSHOT});throw error;
} finally {await browser.close();}
"""


def port():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def wait_http(url, process, log, timeout=45):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise AssertionError("server exited: " + log.read_text(errors="replace")[-2000:])
        try:
            urllib.request.urlopen(url, timeout=1).close()
            return
        except urllib.error.HTTPError:
            return
        except (OSError, urllib.error.URLError):
            time.sleep(.15)
    raise AssertionError("server startup timeout: " + log.read_text(errors="replace")[-2000:])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    cli = args.cli.resolve()
    workspace = Path(__file__).resolve().parents[2]
    results = []
    for engine in ["sqlite", "postgres17", "postgres18"]:
        with tempfile.TemporaryDirectory(prefix="axiom-fullstack-" + engine + "-") as directory:
            work = Path(directory)
            project = work / "example"
            shutil.copytree(workspace / "examples/simple-database", project,
                            ignore=shutil.ignore_patterns(".axiom", ".local", "__pycache__"))
            env = os.environ.copy()
            for name in ["DATABASE_URL", "PGPASSWORD", "PG_BIN",
                         "AXIOM_DATABASE_POSTGRES_BIN", "AXIOM_DATABASE_ENGINE_DIR",
                         "AXIOM_UI_CACHE_ROOT", "AXIOM_DATABASE_DX_TEST_FAULT"]:
                env.pop(name, None)
            env["AXIOM_DATABASE_KEY_DIR"] = str(work / "keys")
            env["CI"] = "true"
            backend_port, frontend_port = port(), port()
            frontend_manifest = project / "frontend/AxiomDeps.toml"
            frontend_manifest.write_text(frontend_manifest.read_text().replace(
                "127.0.0.1:8093", "127.0.0.1:" + str(backend_port)))
            application = project / "backend/application.acore"
            application.write_text(application.read_text().replace("5173", str(frontend_port)))
            processes = []
            logs = []

            def run(*tokens, expected=0):
                result = subprocess.run([str(cli), *map(str, tokens)], cwd=project, env=env,
                                        capture_output=True, text=True, timeout=90)
                assert result.returncode == expected, (tokens, result.stdout[-2000:], result.stderr[-2000:])
                return result

            def spawn(name, tokens):
                path = work / (name + ".log")
                output = path.open("w")
                logs.append(output)
                process = subprocess.Popen(list(map(str, tokens)), cwd=project, env=env,
                                           stdout=output, stderr=subprocess.STDOUT)
                processes.append(process)
                return process, path

            try:
                if engine == "sqlite":
                    schema = "database/sqlite/schema.acore"
                    backend_source = "backend/sqlite.acore"
                else:
                    major = engine.removeprefix("postgres")
                    tools = cli.parent / "engines/postgres" / major / "bin"
                    data = work / "postgres"
                    subprocess.run([str(tools / "initdb"), "-D", str(data), "--auth=trust",
                                    "--encoding=UTF8", "--locale=C", "--username=dx_operator"],
                                   check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                    postgres_port = port()
                    database, log = spawn("postgres", [tools / "postgres", "-D", data,
                        "-h", "127.0.0.1", "-p", str(postgres_port),
                        "-c", "unix_socket_directories="])
                    env["DATABASE_URL"] = f"postgresql://dx_operator@127.0.0.1:{postgres_port}/postgres?sslmode=disable"
                    for _ in range(100):
                        try:
                            socket.create_connection(("127.0.0.1", postgres_port), timeout=.2).close()
                            break
                        except OSError:
                            time.sleep(.1)
                    schema = "database/schema.acore"
                    backend_source = "backend/main.acore"
                backend, backend_log = spawn("backend", [cli, "run", backend_source,
                    "--port", str(backend_port), "--non-interactive"])
                endpoint = "http://127.0.0.1:" + str(backend_port)
                wait_http(endpoint + "/tasks", backend, backend_log)
                key = (project / "backend/.axiom/database/development.key").read_text().strip()

                def api(method, body=None, path=""):
                    request = urllib.request.Request(endpoint + "/tasks" + path, method=method,
                        headers={"apikey": key, "Content-Type": "application/json"},
                        data=json.dumps(body).encode() if body is not None else None)
                    try:
                        response = urllib.request.urlopen(request, timeout=10)
                    except urllib.error.HTTPError as response:
                        return response.code, json.loads(response.read())
                    return response.status, json.loads(response.read())

                title = "Bound value '); DROP TABLE tasks; --"
                created = api("POST", {"title": title})
                assert created[0] in [200, 201], created
                assert api("POST", {"title": title})[0] == 409
                response = api("GET")
                assert response[0] == 200 and any(row["title"] == title for row in response[1])
                row_path = "/" + urllib.parse.quote(created[1]["id"], safe="")
                renamed = api("PATCH", "Updated through typed storage", row_path)
                assert renamed[0] == 200 and renamed[1]["title"] == "Updated through typed storage", renamed
                deleted = api("DELETE", path=row_path)
                assert deleted[0] == 200 and deleted[1]["id"] == created[1]["id"], deleted
                assert not any(row["id"] == created[1]["id"] for row in api("GET")[1])
                assert api("DELETE", path=row_path)[0] >= 400
                run("database", "status", schema, "--check")
                frontend, frontend_log = spawn("frontend", [cli, "run", "frontend/main.acore",
                    "--port", str(frontend_port)])
                url = "http://127.0.0.1:" + str(frontend_port)
                wait_http(url, frontend, frontend_log, timeout=75)
                tested = subprocess.run(["node", "--input-type=module", "-e", WEB_TEST],
                    cwd=workspace / "axiom-frontend/apps/playground",
                    env={**env, "DX_WEB_URL": url, "DX_API_KEY": key, "DX_SCREENSHOT": str(args.report.parent / ("database-dx-"+engine+".png"))},
                    capture_output=True, text=True, timeout=75)
                assert tested.returncode == 0, (tested.stdout[-1000:], tested.stderr[-2500:])
                backend.terminate()
                backend.wait(timeout=15)
                backend, backend_log = spawn("backend-restart", [cli, "run", backend_source,
                    "--port", str(backend_port), "--non-interactive"])
                wait_http(endpoint + "/tasks", backend, backend_log)
                assert any(row["title"] == "Browser persistence" for row in api("GET")[1])
                if engine != "sqlite":
                    # The backend uses the unelevated role; another tenant's
                    # operator-inserted row must never appear in this API.
                    run("run", schema, "--no-shell", "--execute",
                        "INSERT INTO app.tasks(tenant,id,title) VALUES('other','private','Another tenant');")
                    assert not any(row["title"] == "Another tenant" for row in api("GET")[1])
                visible = [str(p.relative_to(project)) for p in project.rglob("*")
                           if p.is_file() and ".axiom" not in p.relative_to(project).parts
                           and p.suffix in [".json", ".lockfile", ".lock"]]
                assert not visible, visible
                print(engine, "native HTTP/browser/restart/clean-tree acceptance passed", flush=True)
                results.append({"engine": engine, "result": "passed",
                                "checks": ["bound-parameters", "duplicate-409", "native-update", "native-delete", "delete-not-found", "browser-create", "browser-delete",
                                           "browser-failure", "reload", "backend-restart",
                                           "status", "hidden-state"]})
            finally:
                for process in reversed(processes):
                    if process.poll() is None:
                        process.terminate()
                        try:
                            process.wait(timeout=15)
                        except subprocess.TimeoutExpired:
                            process.kill()
                            process.wait()
                for log in logs:
                    log.close()
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps({"format": "axiom-database-fullstack-acceptance/v1",
                                      "results": results}, indent=2) + "\n")


if __name__ == "__main__":
    main()
