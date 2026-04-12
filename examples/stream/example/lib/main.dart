import 'dart:async';
import 'package:flutter/material.dart';
import 'package:axiom_flutter/axiom_flutter.dart';
import 'axiom_generated/axiom_sdk.dart';
import 'axiom_generated/models.dart' as models;

late final AxiomSdk sdk;

void main() async {
  WidgetsFlutterBinding.ensureInitialized();

  sdk = await AxiomSdk.create();
  sdk.runtime.debug = true;

  runApp(const MaterialApp(home: StreamTester()));
}

class StreamTester extends StatefulWidget {
  const StreamTester({super.key});

  @override
  State<StreamTester> createState() => _StreamTesterState();
}

class _StreamTesterState extends State<StreamTester> {
  final List<String> _logs = [];
  StreamSubscription? _subscription;
  AxiomChannel<models.ChatMessage, String>? _activeChannel;
  final TextEditingController _wsController = TextEditingController();

  void _log(String msg) => setState(() => _logs.insert(0, msg));

  void _clear() {
    _subscription?.cancel();
    _activeChannel = null;
    setState(() => _logs.clear());
  }

  /// Test Case 1: HTTP Chunked NDJSON
  void _testHttpStream() {
    _clear();
    _log("🚀 Starting HTTP Chunked Stream...");

    final streamQuery = sdk.streamTest.streamChunks();
    _subscription = streamQuery.stream.listen((state) {
      if (state.isStreaming && state.hasData) {
        final msg = state.data;
        _log("[CHUNK] ${msg?.user}: ${msg?.text}");
      } else if (state.isSuccess && !state.isStreaming) {
        _log("✅ Stream Complete");
      }
    });
  }

  /// Test Case 2: Server-Sent Events (SSE)
  void _testSseStream() {
    _clear();
    _log("🚀 Starting SSE Stream...");

    final streamQuery = sdk.streamTest.streamSse();
    _subscription = streamQuery.stream.listen((state) {
      if (state.isStreaming && state.hasData) {
        final msg = state.data;
        _log("[SSE] ${msg?.user}: ${msg?.text}");
      }
    });
  }

  /// Test Case 3: Bidirectional WebSockets
  void _testWebSocket() {
    _clear();

    // Now returns AxiomChannel<ChatMessage, String>
    final channel = sdk.streamTest.websocketEndpoint();
    _activeChannel = channel;

    _subscription = channel.stream.listen((state) {
      if (state.isStreaming && state.hasData) {
        // state.data is now automatically cast to models.ChatMessage!
        final models.ChatMessage msg = state.data!;
        _log("[WS RECV] ${msg.user}: ${msg.text}");
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text("Axiom Streaming & WS")),
      body: Padding(
        padding: const EdgeInsets.all(16.0),
        child: Column(
          children: [
            Wrap(
              spacing: 8,
              children: [
                ElevatedButton(
                  onPressed: _testHttpStream,
                  child: const Text("HTTP Stream"),
                ),
                ElevatedButton(
                  onPressed: _testSseStream,
                  child: const Text("SSE Stream"),
                ),
                ElevatedButton(
                  onPressed: _testWebSocket,
                  child: const Text("Connect WS"),
                ),
              ],
            ),
            const Divider(),
            if (_activeChannel != null)
              Row(
                children: [
                  Expanded(
                    child: TextField(
                      controller: _wsController,
                      decoration: const InputDecoration(
                        hintText: "Send message to WS...",
                      ),
                    ),
                  ),
                  IconButton(
                    icon: const Icon(Icons.send),
                    onPressed: () {
                      _activeChannel!.send(_wsController.text);
                      _log("[WS SENT] ${_wsController.text}");
                      _wsController.clear();
                    },
                  ),
                ],
              ),
            const SizedBox(height: 16),
            Expanded(
              child: Container(
                padding: const EdgeInsets.all(8),
                color: Colors.black,
                width: double.infinity,
                child: ListView.builder(
                  itemCount: _logs.length,
                  itemBuilder: (context, i) => Text(
                    _logs[i],
                    style: const TextStyle(
                      color: Colors.greenAccent,
                      fontFamily: 'monospace',
                      fontSize: 12,
                    ),
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
