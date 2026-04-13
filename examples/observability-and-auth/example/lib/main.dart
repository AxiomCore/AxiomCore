import 'dart:async';
import 'package:example/clear.dart';
import 'package:flutter/material.dart';
import 'package:axiom_flutter/axiom_flutter.dart';
import 'axiom_generated/axiom_sdk.dart';

late final AxiomSdk sdk;

void main() async {
  WidgetsFlutterBinding.ensureInitialized();
  clearStaleAxiomAuthTokens();
  // 1. Create using the auto-generated config!
  // You can optionally pass `config: AxiomDefaultConfig.config.copyWith(...)` if you want to override something.
  sdk = await AxiomSdk.create();

  // Enable FFI debugging
  sdk.runtime.debug = true;

  runApp(const MyApp());
}

class MyApp extends StatelessWidget {
  const MyApp({super.key});

  @override
  Widget build(BuildContext context) {
    return const MaterialApp(home: AuthTesterScreen());
  }
}

class AuthTesterScreen extends StatefulWidget {
  const AuthTesterScreen({super.key});

  @override
  State<AuthTesterScreen> createState() => _AuthTesterScreenState();
}

class _AuthTesterScreenState extends State<AuthTesterScreen> {
  String log = "Awaiting execution...";
  StreamSubscription? _sub;

  void _log(String message) {
    setState(() => log += '\n$message');
  }

  void _clearLog() {
    setState(() => log = "Executing...");
  }

  /// Helper to listen to ALL stream events (Loading, Network, Cache, Error)
  void _listenToQuery(AxiomQuery<dynamic> query, String actionName) {
    _clearLog();
    _sub?.cancel();

    _sub = query.stream.listen((state) {
      if (state.isLoading) {
        _log("[$actionName] ⏳ Loading...");
      } else if (state.hasError) {
        _log("[$actionName] ❌ Error: ${state.error?.message}");
        _log("   Details: ${state.error?.details}");
      } else if (state.hasData) {
        final source = state.source == AxiomSource.cache
            ? "⚡ CACHED"
            : "🌐 NETWORK";
        _log("[$actionName] ✅ Success ($source): ${state.data}");
      }
    });
  }

  void _loginAndSetToken() {
    _clearLog();
    _sub?.cancel();

    // Mutations require execution via `.mutationFn()`
    final mutationQuery = sdk.authObsTest.login().mutationFn({});

    _sub = mutationQuery.stream.listen((state) {
      if (state.isLoading) {
        _log("[Login] ⏳ Authenticating...");
      } else if (state.hasError) {
        _log("[Login] ❌ Error: ${state.error}");
      } else if (state.hasData) {
        final data = state.data as Map<String, dynamic>?;
        if (data != null && data['token'] != null) {
          // 2. Set Token in Rust Engine
          sdk.authObsTest.setAuthToken('Authorization', data['token']);
          _log("[Login] ✅ Success! Token secured in Rust engine.");
        }
      }
    });
  }

  void _setApiKeyHeader() {
    sdk.authObsTest.setAuthToken('x-api-key', 'secret-key-123');
    _clearLog();
    _log("API Key Header set in Rust engine.");
  }

  void _setApiKeyQuery() {
    sdk.authObsTest.setAuthToken('api_key', 'secret-key-123');
    _clearLog();
    _log("API Key Query set in Rust engine.");
  }

  @override
  void dispose() {
    _sub?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text("Phase 1 & 2 Streaming Tester")),
      body: Padding(
        padding: const EdgeInsets.all(16.0),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Container(
              height: 200,
              padding: const EdgeInsets.all(16),
              color: Colors.black87,
              child: SingleChildScrollView(
                child: Text(
                  log,
                  style: const TextStyle(
                    color: Colors.greenAccent,
                    fontFamily: 'monospace',
                  ),
                ),
              ),
            ),
            const SizedBox(height: 16),
            ElevatedButton(
              onPressed: _loginAndSetToken,
              child: const Text("1. Stream Login & Set JWT"),
            ),
            ElevatedButton(
              onPressed: _setApiKeyHeader,
              child: const Text("2. Set API Key (Header)"),
            ),
            ElevatedButton(
              onPressed: _setApiKeyQuery,
              child: const Text("3. Set API Key (Query)"),
            ),
            const Divider(height: 32),
            ElevatedButton(
              style: ElevatedButton.styleFrom(
                backgroundColor: Colors.purple.shade200,
              ),
              onPressed: () => _listenToQuery(
                sdk.authObsTest.protectedJwt(),
                "JWT Scope Test",
              ),
              child: const Text("Test: Protected JWT (Scope validation)"),
            ),
            ElevatedButton(
              style: ElevatedButton.styleFrom(
                backgroundColor: Colors.purple.shade200,
              ),
              onPressed: () => _listenToQuery(
                sdk.authObsTest.protectedApiKeyHeader(),
                "API Header Test",
              ),
              child: const Text("Test: Protected API Key (Header)"),
            ),
            ElevatedButton(
              style: ElevatedButton.styleFrom(
                backgroundColor: Colors.purple.shade200,
              ),
              onPressed: () => _listenToQuery(
                sdk.authObsTest.protectedApiKeyQuery(),
                "API Query Test",
              ),
              child: const Text("Test: Protected API Key (Query)"),
            ),
            ElevatedButton(
              style: ElevatedButton.styleFrom(
                backgroundColor: Colors.purple.shade200,
              ),
              onPressed: () => _listenToQuery(
                sdk.authObsTest.protectedMulti(),
                "Multi-Auth Test",
              ),
              child: const Text("Test: Multi Auth (OR Condition)"),
            ),
          ],
        ),
      ),
    );
  }
}
