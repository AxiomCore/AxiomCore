import 'package:axiom_flutter/axiom_flutter.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import 'axiom_generated/axiom_sdk.dart';
import 'axiom_generated/models.dart' as models;

/// Backend address. Override with `--dart-define=AXIOM_BASE_URL=https://...`.
/// The Android emulator reaches the host machine's localhost as 10.0.2.2.
String backendBaseUrl() {
  const defined = String.fromEnvironment('AXIOM_BASE_URL');
  if (defined.isNotEmpty) return defined;
  if (!kIsWeb && defaultTargetPlatform == TargetPlatform.android) {
    return 'http://10.0.2.2:{{port}}';
  }
  return 'http://localhost:{{port}}';
}

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  final sdk = await AxiomSdk.create(
    config: AxiomConfig(
      contracts: {
        '{{namespace}}': AxiomContractConfig(
          baseUrl: backendBaseUrl(),
          assetPath: 'assets/axiom/{{namespace}}.axiom',
        ),
      },
    ),
  );
  runApp(App(sdk: sdk));
}

class App extends StatelessWidget {
  const App({super.key, required this.sdk});

  final AxiomSdk sdk;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: '{{project}}',
      theme: ThemeData(colorSchemeSeed: Colors.indigo, useMaterial3: true),
      home: Scaffold(
        appBar: AppBar(
          title: AxiomBuilder<models.Greeting, models.Greeting>(
            query: sdk.{{namespace}}.hello(name: 'Flutter'),
            loading: (context) => const Text('{{project}}'),
            error: (context, error) => const Text('{{project}}'),
            builder: (context, state, greeting) => Text(greeting.message),
          ),
        ),
        body: AxiomBuilder<models.Talks, models.Talks>(
          query: sdk.{{namespace}}.listTalks(),
          loading: (context) => const Center(child: CircularProgressIndicator()),
          error: (context, error) => Center(
            child: Padding(
              padding: const EdgeInsets.all(24),
              child: Text(
                'Could not reach the backend at ${backendBaseUrl()}.\n'
                'Start it with backend/run-backend.sh.\n\n$error',
                textAlign: TextAlign.center,
              ),
            ),
          ),
          builder: (context, state, talks) => ListView(
            children: [
              for (final talk in talks.items)
                ListTile(title: Text(talk.title), subtitle: Text(talk.speaker)),
            ],
          ),
        ),
      ),
    );
  }
}
