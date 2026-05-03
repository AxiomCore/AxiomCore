// GENERATED CODE – DO NOT EDIT.
// ignore_for_file: unused_import
// ignore_for_file: invalid_null_aware_operator

import 'dart:convert';
import 'dart:typed_data';
import 'package:flutter/services.dart' show rootBundle;
import 'package:flutter/widgets.dart' show WidgetsFlutterBinding;
import 'package:axiom_flutter/axiom_flutter.dart';
import 'package:axiom_flutter_web/axiom_generated/py-example/models.dart' as models;

class AxiomDefaultConfig {
  static AxiomConfig get config => const AxiomConfig(
    contracts: {
      'py-example': AxiomContractConfig(
        baseUrl: 'http://localhost:8000',
        assetPath: 'assets/axiom/py-example.axiom',
      ),
    },
  );
}

class AxiomSdk {
  final AxiomRuntime runtime;

  late final PyExampleModule pyExample;

  AxiomSdk._(this.runtime) {
    pyExample = PyExampleModule(runtime, 'py-example');
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

class PyExampleModule {
  final AxiomRuntime _runtime;
  final String _namespace;

  PyExampleModule(this._runtime, this._namespace);

  void setAuthToken(String methodName, String token) {
    _runtime.setAuthToken(namespace: _namespace, methodName: methodName, token: token);
  }
  void clearAuthToken(String methodName) {
    _runtime.clearAuthToken(namespace: _namespace, methodName: methodName);
  }

  AxiomQuery<models.Item> createItem({required models.ItemCreate item}) {
      final argsMap = <String, dynamic>{
        'item': item.toJson(),
      };
      return _runtime.sendMutation<models.Item>(
        namespace: _namespace,
        endpointId: 0,
        method: 'POST',
        path: '/items',
        args: argsMap,
        body: item,
        decoder: (json) => models.Item.fromJson(json),
      );
  }
  AxiomQuery<dynamic> deleteItem({required String itemId, Map<String, dynamic>? body}) {
      final argsMap = <String, dynamic>{
        'item_id': itemId,
      };
      final pathParams = <String, dynamic>{
        'item_id': itemId,
      };
      return _runtime.sendMutation<dynamic>(
        namespace: _namespace,
        endpointId: 0,
        method: 'DELETE',
        path: '/items/{item_id}',
        args: argsMap,
        pathParams: pathParams,
        body: body,
        decoder: (json) => json as dynamic,
      );
  }
  AxiomQuery<models.Item> getItem({required String itemId}) {
      final argsMap = <String, dynamic>{
        'item_id': itemId,
      };
      final pathParams = <String, dynamic>{
        'item_id': itemId,
      };
      return _runtime.send<models.Item>(
        namespace: _namespace,
        endpointId: 0,
        method: 'GET',
        path: '/items/{item_id}',
        args: argsMap,
        pathParams: pathParams,
        decoder: (json) => models.Item.fromJson(json),
      );
  }
  AxiomQuery<List<models.Item>> listItems({int? skip, int? limit, String? search}) {
      final argsMap = <String, dynamic>{
        'skip': skip,
        'limit': limit,
        'search': search,
      };
      final queryParams = <String, dynamic>{
        'skip': skip,
        'limit': limit,
        'search': search,
      };
      return _runtime.send<List<models.Item>>(
        namespace: _namespace,
        endpointId: 0,
        method: 'GET',
        path: '/items',
        args: argsMap,
        queryParams: queryParams,
        decoder: (json) => (json as List).map((e) => models.Item.fromJson(e)).toList(),
      );
  }
  AxiomQuery<dynamic> listUsers() {
      final argsMap = <String, dynamic>{
      };
      return _runtime.send<dynamic>(
        namespace: _namespace,
        endpointId: 0,
        method: 'GET',
        path: '/admin/users',
        args: argsMap,
        decoder: (json) => json as dynamic,
      );
  }
  AxiomQuery<models.Token> login({Map<String, dynamic>? body}) {
      final argsMap = <String, dynamic>{
      };
      return _runtime.sendMutation<models.Token>(
        namespace: _namespace,
        endpointId: 0,
        method: 'POST',
        path: '/login',
        args: argsMap,
        body: body,
        decoder: (json) => models.Token.fromJson(json),
      );
  }
  AxiomQuery<models.User> register({required models.UserCreate user}) {
      final argsMap = <String, dynamic>{
        'user': user.toJson(),
      };
      return _runtime.sendMutation<models.User>(
        namespace: _namespace,
        endpointId: 0,
        method: 'POST',
        path: '/register',
        args: argsMap,
        body: user,
        decoder: (json) => models.User.fromJson(json),
      );
  }
  AxiomQuery<dynamic> sendEmail({required dynamic backgroundTasks, Map<String, dynamic>? body}) {
      final argsMap = <String, dynamic>{
        'background_tasks': backgroundTasks,
      };
      final queryParams = <String, dynamic>{
        'background_tasks': backgroundTasks,
      };
      return _runtime.sendMutation<dynamic>(
        namespace: _namespace,
        endpointId: 0,
        method: 'POST',
        path: '/send-email',
        args: argsMap,
        queryParams: queryParams,
        body: body,
        decoder: (json) => json as dynamic,
      );
  }
  AxiomQuery<void> websocketEndpoint({Map<String, dynamic>? body}) {
      final argsMap = <String, dynamic>{
      };
      return _runtime.sendMutation<void>(
        namespace: _namespace,
        endpointId: 0,
        method: 'WS',
        path: '/ws',
        args: argsMap,
        body: body,
        decoder: (json) => null,
      );
  }
}

