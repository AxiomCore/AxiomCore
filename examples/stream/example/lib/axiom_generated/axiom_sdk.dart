// GENERATED CODE – DO NOT EDIT.
// ignore_for_file: unused_import
// ignore_for_file: invalid_null_aware_operator

import 'dart:convert';
import 'dart:typed_data';
import 'package:flutter/services.dart' show rootBundle;
import 'package:flutter/widgets.dart' show WidgetsFlutterBinding;
import 'package:axiom_flutter/axiom_flutter.dart';
import 'package:example/axiom_generated/models.dart' as models;

class AxiomDefaultConfig {
  static AxiomConfig get config => const AxiomConfig(
    contracts: {
      'streamTest': AxiomContractConfig(
        baseUrl: 'http://localhost:8000',
        assetPath: '/Users/yashmakan/AxiomCore/_axiom/AxiomCore/examples/stream/.axiom',
      ),
    },
  );
}

class AxiomSdk {
  final AxiomRuntime runtime;

  late final StreamTestModule streamTest;

  AxiomSdk._(this.runtime) {
    streamTest = StreamTestModule(runtime, 'streamTest');
  }

  static Future<AxiomSdk> create({AxiomConfig? config}) async {
    WidgetsFlutterBinding.ensureInitialized();
    final runtime = AxiomRuntime();
    final cfg = config ?? AxiomDefaultConfig.config;
    runtime.debug = cfg.debug;
    await runtime.init(cfg.dbPath);

    for (final entry in cfg.contracts.entries) {
      final c = entry.value;
      final contractData = await rootBundle.load(c.assetPath);
      final contractBytes = contractData.buffer.asUint8List();
      runtime.loadContract(
        namespace: entry.key,
        baseUrl: c.baseUrl,
        contractBytes: contractBytes,
      );
    }
    return AxiomSdk._(runtime);
  }

}

class StreamTestModule {
  final AxiomRuntime _runtime;
  final String _namespace;

  StreamTestModule(this._runtime, this._namespace);

  void setAuthToken(String methodName, String token) {
    _runtime.setAuthToken(namespace: _namespace, methodName: methodName, token: token);
  }
  void clearAuthToken(String methodName) {
    _runtime.clearAuthToken(namespace: _namespace, methodName: methodName);
  }

  AxiomStreamQuery<models.ChatMessage> streamChunks() {
      final argsMap = <String, dynamic>{
      };
    final res = _runtime.callStream(
      namespace: _namespace,
      endpointId: 0,
      method: 'GET',
      path: '/stream/chunks',
      requestBytes: Uint8List(0),
    );
    final mapped = res.stream.map((state) {
      if (state.hasError) return state.map<models.ChatMessage>((_) => throw '');
      if (state.data != null) {
         final decodeFn = (bytes) {
          final str = utf8.decode(bytes).trim();
          if (str.isEmpty) return null;

          // Split chunk sequences explicitly (Newline JSON/SSE events fallback handler)
          final lines = str.split('\n').map((l) => l.trim()).where((l) => l.isNotEmpty).toList();
          if (lines.isEmpty) return null;

          for (var line in lines) {
            if (line.startsWith('data:')) {
              line = line.substring(5).trim();
            }
            try {
              final json = jsonDecode(line);
              return models.ChatMessage.fromJson(json);
            } catch (_) {}
          }

          if (str is models.ChatMessage) return str as models.ChatMessage;
          return null;
        };
         final decodedData = decodeFn(state.data!);
         if (decodedData == null) return AxiomState<models.ChatMessage>.loading();
         return AxiomState<models.ChatMessage>.success(decodedData, state.source, isStreaming: state.isStreaming);
      }
      return AxiomState<models.ChatMessage>.loading();
    });
    return AxiomStreamQuery<models.ChatMessage>(mapped);
  }

  AxiomStreamQuery<models.ChatMessage> streamSse() {
      final argsMap = <String, dynamic>{
      };
    final res = _runtime.callStream(
      namespace: _namespace,
      endpointId: 1,
      method: 'GET',
      path: '/stream/sse',
      requestBytes: Uint8List(0),
    );
    final mapped = res.stream.map((state) {
      if (state.hasError) return state.map<models.ChatMessage>((_) => throw '');
      if (state.data != null) {
         final decodeFn = (bytes) {
          final str = utf8.decode(bytes).trim();
          if (str.isEmpty) return null;

          // Split chunk sequences explicitly (Newline JSON/SSE events fallback handler)
          final lines = str.split('\n').map((l) => l.trim()).where((l) => l.isNotEmpty).toList();
          if (lines.isEmpty) return null;

          for (var line in lines) {
            if (line.startsWith('data:')) {
              line = line.substring(5).trim();
            }
            try {
              final json = jsonDecode(line);
              return models.ChatMessage.fromJson(json);
            } catch (_) {}
          }

          if (str is models.ChatMessage) return str as models.ChatMessage;
          return null;
        };
         final decodedData = decodeFn(state.data!);
         if (decodedData == null) return AxiomState<models.ChatMessage>.loading();
         return AxiomState<models.ChatMessage>.success(decodedData, state.source, isStreaming: state.isStreaming);
      }
      return AxiomState<models.ChatMessage>.loading();
    });
    return AxiomStreamQuery<models.ChatMessage>(mapped);
  }

  AxiomChannel<models.ChatMessage, String> websocketEndpoint() {
      final argsMap = <String, dynamic>{
      };
    final res = _runtime.callStream(
      namespace: _namespace,
      endpointId: 2,
      method: 'WS',
      path: '/ws/chat',
      requestBytes: Uint8List(0),
    );
    final mapped = res.stream.map((state) {
      if (state.hasError) return state.map<models.ChatMessage>((_) => throw '');
      if (state.data != null) {
        try {
          return AxiomState<models.ChatMessage>.success(models.ChatMessage.fromJson(jsonDecode(utf8.decode(state.data!))), state.source, isStreaming: true);
        } catch (e) { print("WS Decode Error: $e"); }
      }
      return AxiomState<models.ChatMessage>.loading();
    });
    return AxiomChannel<models.ChatMessage, String>(res.requestId, mapped, _runtime);
  }

}

