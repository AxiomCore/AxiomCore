import 'package:flutter/material.dart';
import 'package:axiom_flutter/axiom_flutter.dart';
import 'axiom_generated/axiom_sdk.dart';
import 'axiom_generated/models.dart' as models;

late final AxiomSdk sdk;

void main() async {
  WidgetsFlutterBinding.ensureInitialized();

  // 1. Initialize the SDK
  sdk = await AxiomSdk.create();

  runApp(const MaterialApp(home: RpcDemoScreen()));
}

class RpcDemoScreen extends StatefulWidget {
  const RpcDemoScreen({super.key});

  @override
  State<RpcDemoScreen> createState() => _RpcDemoScreenState();
}

class _RpcDemoScreenState extends State<RpcDemoScreen> {
  // 2. Instantiate a local model (this would usually come from a previous query)
  final person = const models.Person(id: "USR-9921", name: "Yash Makan");

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text("RPC Chaining Demo")),
      body: Center(
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            Text("Active User: ${person.name}"),
            const SizedBox(height: 20),

            // 3. Use the Mutation Builder for the RPC call
            // Since 'create_collection' is a POST, the generator mapped it to AxiomMutation
            AxiomMutationBuilder<dynamic, int>(
              // 4. Calling the RPC Extension Method!
              // Note how we pass 'sdk' (the module) and the typed 'limit' argument.
              mutation: AxiomMutation(
                (limit) => person.getContacts(sdk.rpc, limit: limit),
              ),
              builder: (context, state, execute) {
                return Column(
                  children: [
                    ElevatedButton(
                      onPressed: state.isMutating ? null : () => execute(10),
                      child: Text(
                        state.isMutating ? "Fetching..." : "Get Contacts (RPC)",
                      ),
                    ),

                    const SizedBox(height: 20),

                    // Result Display
                    if (state.hasData)
                      Container(
                        padding: const EdgeInsets.all(12),
                        color: Colors.green.shade50,
                        child: Text(
                          "Server Response: ${state.data['message']}",
                        ),
                      ),

                    if (state.hasError)
                      Text(
                        "Error: ${state.error?.message}",
                        style: const TextStyle(color: Colors.red),
                      ),
                  ],
                );
              },
            ),
          ],
        ),
      ),
    );
  }
}
