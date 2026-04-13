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
      'rpc': AxiomContractConfig(
        baseUrl: 'http://localhost:8000',
        assetPath: '/Users/yashmakan/AxiomCore/_axiom/AxiomCore/examples/rpc/.axiom',
      ),
    },
  );
}

class AxiomSdk {
  final AxiomRuntime runtime;

  late final RpcModule rpc;

  AxiomSdk._(this.runtime) {
    rpc = RpcModule(runtime, 'rpc');
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

class RpcModule {
  final AxiomRuntime _runtime;
  final String _namespace;

  RpcModule(this._runtime, this._namespace);

  void setAuthToken(String methodName, String token) {
    _runtime.setAuthToken(namespace: _namespace, methodName: methodName, token: token);
  }
  void clearAuthToken(String methodName) {
    _runtime.clearAuthToken(namespace: _namespace, methodName: methodName);
  }

  AxiomMutation<dynamic, ({Map<String, dynamic>? body})> createCollection() {
    return AxiomMutation(($queryArgs) {
      final argsMap = <String, dynamic>{
      };
      return _runtime.sendMutation<dynamic>(
        namespace: _namespace,
        endpointId: 0,
        method: 'POST',
        path: '/collections',
        args: argsMap,
        body: $queryArgs.body,
        decoder: (json) => json as dynamic,
      );
    });
  }
}

// RPC Extension for Person
extension PersonRpc on models.Person {
  dynamic getContacts(RpcModule module, {required int limit, }) {
    return module.createCollection().mutationFn((
      body: {'max_results': limit, 'person_id': this.id, },
    ));
  }
}

