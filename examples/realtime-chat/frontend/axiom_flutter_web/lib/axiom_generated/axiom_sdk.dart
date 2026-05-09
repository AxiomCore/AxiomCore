// GENERATED CODE – DO NOT EDIT.
// ignore_for_file: unused_import
// ignore_for_file: invalid_null_aware_operator

import 'dart:convert';
import 'dart:typed_data';
import 'package:flutter/services.dart' show rootBundle;
import 'package:flutter/widgets.dart' show WidgetsFlutterBinding;
import 'package:axiom_flutter/axiom_flutter.dart';
import 'package:axiom_flutter/src/internal/query_key.dart';
import 'package:axiom_flutter/src/internal/axiom_codec.dart';
import 'package:axiom_flutter/src/query_manager.dart';
import 'package:axiom_flutter_web/axiom_generated/models.dart' as models;

class AxiomDefaultConfig {
  static AxiomConfig get config => const AxiomConfig(
    contracts: {
      'realtime-chat': AxiomContractConfig(
        baseUrl: 'http://localhost:8080',
        assetPath: 'assets/axiom/realtime-chat.axiom',
      ),
    },
  );
}

class AxiomSdk {
  final AxiomRuntime runtime;

  late final RealtimeChatModule realtimeChat;

  AxiomSdk._(this.runtime) {
    realtimeChat = RealtimeChatModule(runtime, 'realtime-chat');
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

class RealtimeChatModule {
  final AxiomRuntime _runtime;
  final String _namespace;
  late final RealtimeChatModuleAxiomNamespace axiom;

  RealtimeChatModule(this._runtime, this._namespace) {
    axiom = RealtimeChatModuleAxiomNamespace(this);
  }

  AxiomQuery<models.ChatMessage> handleConnections() {
      final argsMap = <String, dynamic>{
      };
      return _runtime.send<models.ChatMessage>(
        namespace: _namespace,
        endpointId: 0,
        method: 'WS',
        path: '/ws',
        args: argsMap,
        decoder: (json) => models.ChatMessage.fromJson(json),
      );
  }
}

class RealtimeChatModuleAxiomNamespace {
  final RealtimeChatModule _parent;
  RealtimeChatModuleAxiomNamespace(this._parent);

  void setAuthToken(String methodName, String token) {
    _parent._runtime.setAuthToken(namespace: _parent._namespace, methodName: methodName, token: token);
  }

  void clearAuthToken(String methodName) {
    _parent._runtime.clearAuthToken(namespace: _parent._namespace, methodName: methodName);
  }

  void connect(String methodName, {Map<String, dynamic> args = const {}}) {
    print('Warning: .connect() in Dart requires active UI subscription. Use the endpoint stream directly.');
  }

  void disconnect(String methodName, {Map<String, dynamic> args = const {}}) {
    int? endpointId;
    switch (methodName) {
      case 'handleConnections': endpointId = 0; break;
    }
    if (endpointId != null) {
      final key = AxiomQueryKey.build(endpoint: '${_parent._namespace}_$endpointId', args: args);
      AxiomQueryManager().remove(key);
    }
  }

  void send(String methodName, dynamic payload, {Map<String, dynamic> args = const {}}) {
    int? endpointId;
    switch (methodName) {
      case 'handleConnections': endpointId = 0; break;
    }
    if (endpointId == null) return;
    final key = AxiomQueryKey.build(endpoint: '${_parent._namespace}_$endpointId', args: args);
    final bytes = AxiomCodec.encodeBody(payload, null);
    AxiomQueryManager().send(key, bytes, _parent._runtime);
  }
}

