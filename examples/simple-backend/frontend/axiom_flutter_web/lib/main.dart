import 'package:axiom_flutter/axiom_flutter.dart';
import 'package:axiom_flutter_web/axiom_generated/py-example/axiom_sdk.dart';
import 'package:axiom_flutter_web/axiom_generated/py-example/models.dart';
import 'package:flutter/material.dart';

late AxiomSdk sdk;

Future<void> main() async {
  sdk = await AxiomSdk.create();
  runApp(const MyApp());
}

class MyApp extends StatelessWidget {
  const MyApp({super.key});

  // This widget is the root of your application.
  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Flutter Demo',
      theme: ThemeData(
        // This is the theme of your application.
        //
        // TRY THIS: Try running your application with "flutter run". You'll see
        // the application has a purple toolbar. Then, without quitting the app,
        // try changing the seedColor in the colorScheme below to Colors.green
        // and then invoke "hot reload" (save your changes or press the "hot
        // reload" button in a Flutter-supported IDE, or press "r" if you used
        // the command line to start the app).
        //
        // Notice that the counter didn't reset back to zero; the application
        // state is not lost during the reload. To reset the state, use hot
        // restart instead.
        //
        // This works for code too, not just values: Most code changes can be
        // tested with just a hot reload.
        colorScheme: ColorScheme.fromSeed(seedColor: Colors.deepPurple),
      ),
      home: const MyHomePage(title: 'Flutter Demo Home Page'),
    );
  }
}

class MyHomePage extends StatefulWidget {
  const MyHomePage({super.key, required this.title});

  // This widget is the home page of your application. It is stateful, meaning
  // that it has a State object (defined below) that contains fields that affect
  // how it looks.

  // This class is the configuration for the state. It holds the values (in this
  // case the title) provided by the parent (in this case the App widget) and
  // used by the build method of the State. Fields in a Widget subclass are
  // always marked "final".

  final String title;

  @override
  State<MyHomePage> createState() => _MyHomePageState();
}

class _MyHomePageState extends State<MyHomePage> {
  String? token;
  Item? createdItem;

  final emailController = TextEditingController(text: "test@test.com");
  final passwordController = TextEditingController(text: "123456");

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text("Axiom Full Test")),
      body: SingleChildScrollView(
        padding: const EdgeInsets.all(16),
        child: Column(
          children: [
            TextField(controller: emailController),
            TextField(controller: passwordController),

            const SizedBox(height: 20),

            /// REGISTER
            ElevatedButton(
              onPressed: () async {
                final q = sdk.pyExample.register(
                  user: UserCreate(
                    email: emailController.text,
                    password: passwordController.text,
                  ),
                );

                final result = await q.stream.first;

                if (result.hasData) {
                  print("Registered: ${result.data!.id}");
                } else {
                  print(result.error);
                }
              },
              child: const Text("Register"),
            ),

            /// LOGIN
            ElevatedButton(
              onPressed: () async {
                final q =
                    sdk.pyExample.login(
                      body: {
                        "username": emailController.text,
                        "password": passwordController.text,
                      },
                    )..setHeader(
                      'Content-Type',
                      'application/x-www-form-urlencoded',
                    );

                final result = await q.stream.first;

                if (result.hasData) {
                  token = result.data!.accessToken;
                  sdk.pyExample.setAuthToken('Authorization', token!);
                  print("TOKEN: $token");
                  setState(() {});
                }
              },
              child: const Text("Login"),
            ),

            /// CREATE ITEM
            ElevatedButton(
              onPressed: token == null
                  ? null
                  : () async {
                      final q = sdk.pyExample.createItem(
                        item: ItemCreate(
                          title: "My Item",
                          description: "hello",
                        ),
                      );

                      final result = await q.stream.first;

                      if (result.hasData) {
                        createdItem = result.data!;
                        print("Created: ${createdItem!.id}");
                        setState(() {});
                      }
                    },
              child: const Text("Create Item"),
            ),

            const SizedBox(height: 20),

            /// LIST ITEMS (QUERY)
            AxiomBuilder<List<Item>, List<Item>>(
              query: sdk.pyExample.listItems(),
              builder: (context, state, _) {
                if (state.isLoading) return const CircularProgressIndicator();

                if (state.hasError) {
                  return Text("Error: ${state.error}");
                }

                final items = state.data ?? [];

                return Column(
                  children: items
                      .map(
                        (i) => ListTile(
                          title: Text(i.title),
                          subtitle: Text(i.id),
                        ),
                      )
                      .toList(),
                );
              },
            ),

            const SizedBox(height: 20),

            /// GET SINGLE ITEM
            if (createdItem != null)
              AxiomBuilder<Item, Item>(
                query: sdk.pyExample.getItem(itemId: createdItem!.id),
                builder: (context, state, _) {
                  if (!state.hasData) return const SizedBox();

                  return Text("Fetched: ${state.data!.title}");
                },
              ),

            const SizedBox(height: 20),

            /// DELETE ITEM
            ElevatedButton(
              onPressed: createdItem == null
                  ? null
                  : () async {
                      final q = sdk.pyExample.deleteItem(
                        itemId: createdItem!.id,
                        body: {},
                      );

                      final result = await q.stream.first;

                      if (result.hasData) {
                        print("Deleted");
                        createdItem = null;

                        /// refresh list manually
                        sdk.pyExample.listItems().refresh();

                        setState(() {});
                      }
                    },
              child: const Text("Delete Item"),
            ),
          ],
        ),
      ),
    );
  }
}
