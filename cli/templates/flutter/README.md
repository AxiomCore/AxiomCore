# {{project}}

A Flutter app with a typed Axiom backend, created by `axiom init --template flutter`.

```
backend/   Acore contract (main.acore) and AxiomDeps.toml
app/       Flutter app; generated client in app/lib/axiom_generated
```

## Run it

```sh
./backend/run-backend.sh                    # 1. build and serve the backend on :{{port}}
(cd app && axiom pull --watch)              # 2. regenerate the Dart client when the contract changes
(cd app && flutter run)                     # 3. run the app (pick a device: Chrome, macOS, iOS, Android)
```

Step 2 is only needed while you edit `backend/main.acore`; the client is already generated.
After editing the contract, run step 1 again; `axiom pull --watch` picks up the new
`backend/backend.axiom` and rewrites `app/lib/axiom_generated`.

## Notes

- Base URL: `http://localhost:{{port}}`, or `http://10.0.2.2:{{port}}` on the Android emulator
  (see `backendBaseUrl()` in `app/lib/main.dart`). Use `--dart-define=AXIOM_BASE_URL=...` for
  a physical device or a deployed backend.
- Already set for you: iOS 13.0 and macOS 11.0 deployment targets, the macOS
  `com.apple.security.network.client` entitlement, the Android INTERNET permission,
  Android minSdk 24 and cleartext http in Android debug builds only.
- `axiom doctor` checks your Flutter, Dart, CocoaPods and Android SDK setup.
- The CORS rule in `backend/main.acore` allows every origin for development; restrict it to your
  web origins before deploying. Keep its `headers` list: the SDK sends `traceparent` and
  `tracestate` on every call, and a missing header makes the browser refuse every request.
