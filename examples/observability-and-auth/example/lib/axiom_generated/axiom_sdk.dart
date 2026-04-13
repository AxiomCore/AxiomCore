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
      'authObsTest': AxiomContractConfig(
        baseUrl: 'http://localhost:8000',
        assetPath: '/Users/yashmakan/AxiomCore/_axiom/AxiomCore/examples/observability-and-auth/.axiom',
      ),
    },
  );
}

class AxiomSdk {
  final AxiomRuntime runtime;

  late final AuthObsTestModule authObsTest;

  AxiomSdk._(this.runtime) {
    authObsTest = AuthObsTestModule(runtime, 'authObsTest');
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

class AuthObsTestModule {
  final AxiomRuntime _runtime;
  final String _namespace;

  AuthObsTestModule(this._runtime, this._namespace);

  void setAuthToken(String methodName, String token) {
    _runtime.setAuthToken(namespace: _namespace, methodName: methodName, token: token);
  }
  void clearAuthToken(String methodName) {
    _runtime.clearAuthToken(namespace: _namespace, methodName: methodName);
  }

  AxiomMutation<dynamic, void> login() {
    return AxiomMutation((args) {
      final argsMap = <String, dynamic>{
      };
      return _runtime.send<dynamic>(
        namespace: _namespace,
        endpointId: 0,
        method: 'POST',
        path: '/login',
        args: argsMap,
        decoder: (json) => json as dynamic,
      );
    });
  }

  AxiomQuery<dynamic> protectedApiKeyHeader({String? xApiKey, }) {
      final argsMap = <String, dynamic>{
        'x_api_key': xApiKey,
      };
      return _runtime.send<dynamic>(
        namespace: _namespace,
        endpointId: 2,
        method: 'GET',
        path: '/protected/api-key-header',
        args: argsMap,
        decoder: (json) => json as dynamic,
      );
  }

  AxiomQuery<dynamic> protectedApiKeyQuery({String? apiKey, }) {
      final argsMap = <String, dynamic>{
        'api_key': apiKey,
      };
      final queryParams = <String, dynamic>{
        'api_key': apiKey,
      };
      return _runtime.send<dynamic>(
        namespace: _namespace,
        endpointId: 3,
        method: 'GET',
        path: '/protected/api-key-query',
        args: argsMap,
        queryParams: queryParams,
        decoder: (json) => json as dynamic,
      );
  }

  AxiomQuery<dynamic> protectedJwt({String? authorization, }) {
      final argsMap = <String, dynamic>{
        'authorization': authorization,
      };
      return _runtime.send<dynamic>(
        namespace: _namespace,
        endpointId: 1,
        method: 'GET',
        path: '/protected/jwt',
        args: argsMap,
        decoder: (json) => json as dynamic,
      );
  }

  AxiomQuery<dynamic> protectedMulti({String? authorization, String? xApiKey, }) {
      final argsMap = <String, dynamic>{
        'authorization': authorization,
        'x_api_key': xApiKey,
      };
      return _runtime.send<dynamic>(
        namespace: _namespace,
        endpointId: 4,
        method: 'GET',
        path: '/protected/multi',
        args: argsMap,
        decoder: (json) => json as dynamic,
      );
  }

}

