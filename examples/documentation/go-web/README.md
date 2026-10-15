# go-web

Call a real Go API from an ATMX browser client. Verified with published
CLI 0.149.0 on macOS arm64, atmx-cli 0.146.7 and atmx-web 0.147.5.
Use Go 1.25.7 (the module requires 1.23+), Node.js 22.12+, npm, and two terminals.

From this directory, prepare and start the backend:

```bash
axiom install axiom-go-extractor --module
cd backend
go mod download
axiom build axiom.acore --out backend.axiom
axiom inspect backend.axiom
go run .
```

The installable package is axiom-go-extractor; axiom init uses the source module alias axiom-go.

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

Open http://127.0.0.1:5176 and select **Say hello**. It displays **Hello, Ada!**.
The client awaits runtime startup, subscribes to query state, and disposes old
subscriptions. `npm run build` checks TypeScript and creates a production bundle.

Verify the native API with `curl -fsS http://127.0.0.1:8084/greet/Ada`.
A name longer than 40 characters returns 422. The API is public and stateless;
no login or database is configured. Local CORS permits only the documented
frontend origins. Add application authentication separately.

Stop both servers with Ctrl-C. Keep or remove the disposable directory,
including its generated SDK, package cache and build output.
See the [guide](https://docs.axiomcore.dev/guides/go-to-web/)
for ownership, validation, errors, and release requirements.
