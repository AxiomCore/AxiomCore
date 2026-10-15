# fastapi-web

Call a real Python API from an ATMX browser client. Verified with published
CLI 0.149.0 on macOS arm64, atmx-cli 0.146.7 and atmx-web 0.147.5.
Use Python 3.12, Node.js 22.12+, npm, and two terminals.

From this directory, prepare and start the backend:

```bash
python3 -m venv .venv
.venv/bin/python -m pip install -r backend/requirements.txt
axiom install axiom-fastapi --module
cd backend
axiom build axiom.acore --out backend.axiom
axiom inspect backend.axiom
../.venv/bin/python -m uvicorn main:app --host 127.0.0.1 --port 8083
```

Extraction imports the app. Keep import-time migrations and production side effects out of that path.

In another terminal, from this directory:

```bash
cd frontend
npm ci
axiom pull ../backend/backend.axiom --framework atmx-web --name greetings --out generated
npm run dev
```

Install the pinned local generator **before** `axiom pull`. CLI 0.149.0's
matching automatic npm companion is unavailable. This explicit combination is
verified for this example; qualify your own contract before upgrading.

Open http://127.0.0.1:5175 and select **Say hello**. It displays **Hello, Ada!**.
The client awaits runtime startup, subscribes to query state, and disposes old
subscriptions. `npm run build` checks TypeScript and creates a production bundle.

Verify the native API with `curl -fsS http://127.0.0.1:8083/greet/Ada`.
A name longer than 40 characters returns 422. The API is public and stateless;
no login or database is configured. Local CORS permits only the documented
frontend origins. Add application authentication separately.

Stop both servers with Ctrl-C. Keep or remove the disposable directory,
including its generated SDK, package cache and venv.
See the [guide](https://docs.axiomcore.dev/guides/fastapi-to-web/)
for ownership, validation, errors, and release requirements.
