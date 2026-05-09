import 'package:flutter/material.dart';
import 'package:axiom_flutter/axiom_flutter.dart';
import 'axiom_generated/axiom_sdk.dart';
import 'axiom_generated/models.dart';

void main() async {
  final sdk = await AxiomSdk.create(config: AxiomDefaultConfig.config);
  runApp(ChatApp(sdk: sdk));
}

class ChatApp extends StatelessWidget {
  final AxiomSdk sdk;
  const ChatApp({super.key, required this.sdk});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        scaffoldBackgroundColor: const Color(0xFFF1F5F9), // slate-100
        fontFamily: 'Segoe UI',
      ),
      home: ChatScreen(sdk: sdk),
    );
  }
}

class ChatScreen extends StatefulWidget {
  final AxiomSdk sdk;
  const ChatScreen({super.key, required this.sdk});

  @override
  State<ChatScreen> createState() => _ChatScreenState();
}

class _ChatScreenState extends State<ChatScreen> {
  final List<ChatMessage> _messages = [];
  final TextEditingController _senderCtrl = TextEditingController(
    text: "FlutterUser",
  );
  final TextEditingController _msgCtrl = TextEditingController();
  final ScrollController _scrollCtrl = ScrollController();

  void _sendMessage() {
    if (_msgCtrl.text.trim().isEmpty) return;

    // ✨ Send upstream through the active socket natively!
    widget.sdk.realtimeChat.axiom.send("handleConnections", {
      "sender": _senderCtrl.text,
      "content": _msgCtrl.text,
    });

    _msgCtrl.clear();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 672), // max-w-2xl
          child: Padding(
            padding: const EdgeInsets.all(32.0),
            child: Column(
              children: [
                // Navigation Bar
                Container(
                  padding: const EdgeInsets.all(16),
                  margin: const EdgeInsets.only(bottom: 24),
                  decoration: BoxDecoration(
                    color: Colors.white,
                    borderRadius: BorderRadius.circular(12),
                    border: Border.all(
                      color: const Color(0xFFE2E8F0),
                    ), // slate-200
                    boxShadow: const [
                      BoxShadow(color: Colors.black12, blurRadius: 4),
                    ],
                  ),
                  child: Row(
                    mainAxisAlignment: MainAxisAlignment.spaceBetween,
                    children: [
                      const Text(
                        "Axiom Flutter Chat",
                        style: TextStyle(
                          fontWeight: FontWeight.bold,
                          fontSize: 20,
                          color: Color(0xFF2563EB),
                        ),
                      ),
                      // Reactive Engine Status
                      AxiomBuilder<ChatMessage, ChatMessage>(
                        query: widget.sdk.realtimeChat.handleConnections(),
                        loading: (_) => const Text(
                          "Connecting...",
                          style: TextStyle(
                            color: Color(0xFFF59E0B),
                            fontSize: 12,
                            fontWeight: FontWeight.bold,
                          ),
                        ),
                        builder: (ctx, state, data) => const Text(
                          "✅ Engine Online",
                          style: TextStyle(
                            color: Color(0xFF16A34A),
                            fontSize: 12,
                            fontWeight: FontWeight.bold,
                          ),
                        ),
                      ),
                    ],
                  ),
                ),

                // Chat Container
                Expanded(
                  child: Container(
                    decoration: BoxDecoration(
                      color: Colors.white,
                      borderRadius: BorderRadius.circular(12),
                      border: Border.all(color: const Color(0xFFE2E8F0)),
                      boxShadow: const [
                        BoxShadow(color: Colors.black12, blurRadius: 4),
                      ],
                    ),
                    child: Column(
                      children: [
                        // Header
                        Container(
                          width: double.infinity,
                          padding: const EdgeInsets.all(16),
                          decoration: const BoxDecoration(
                            color: Color(0xFF1E293B), // slate-800
                            borderRadius: BorderRadius.vertical(
                              top: Radius.circular(11),
                            ),
                          ),
                          child: const Text(
                            "Live Global Chat Room",
                            style: TextStyle(
                              color: Colors.white,
                              fontWeight: FontWeight.bold,
                              fontSize: 14,
                            ),
                          ),
                        ),

                        // Messages Area
                        Expanded(
                          child: AxiomBuilder<ChatMessage, ChatMessage>(
                            query: widget.sdk.realtimeChat.handleConnections(),
                            loading: (_) => const Center(
                              child: Text(
                                "Connecting to WebSocket via Rust WASM...",
                                style: TextStyle(
                                  color: Color(0xFF94A3B8),
                                  fontSize: 12,
                                ),
                              ),
                            ),
                            builder: (context, state, data) {
                              // AxiomBuilder triggers on new stream data. Append it!
                              WidgetsBinding.instance.addPostFrameCallback((_) {
                                if (!_messages.contains(data)) {
                                  setState(() => _messages.add(data));
                                  _scrollCtrl.animateTo(
                                    _scrollCtrl.position.maxScrollExtent + 100,
                                    duration: const Duration(milliseconds: 300),
                                    curve: Curves.easeOut,
                                  );
                                }
                              });

                              return ListView.builder(
                                controller: _scrollCtrl,
                                padding: const EdgeInsets.all(16),
                                itemCount: _messages.length,
                                itemBuilder: (context, index) {
                                  final msg = _messages[index];
                                  final isMe = msg.sender == _senderCtrl.text;
                                  final isSystem = msg.sender == "System";

                                  return Container(
                                    margin: const EdgeInsets.only(bottom: 12),
                                    alignment: isSystem
                                        ? Alignment.center
                                        : isMe
                                        ? Alignment.centerRight
                                        : Alignment.centerLeft,
                                    child: Container(
                                      constraints: const BoxConstraints(
                                        maxWidth: 400,
                                      ),
                                      padding: const EdgeInsets.all(12),
                                      decoration: BoxDecoration(
                                        color: isSystem
                                            ? const Color(0xFFF1F5F9)
                                            : isMe
                                            ? const Color(0xFF2563EB)
                                            : const Color(0xFFF1F5F9),
                                        borderRadius: BorderRadius.circular(12),
                                        border: isSystem || !isMe
                                            ? Border.all(
                                                color: const Color(0xFFE2E8F0),
                                              )
                                            : null,
                                      ),
                                      child: Column(
                                        crossAxisAlignment:
                                            CrossAxisAlignment.start,
                                        children: [
                                          if (!isSystem)
                                            Text(
                                              msg.sender.toUpperCase(),
                                              style: TextStyle(
                                                fontSize: 10,
                                                fontWeight: FontWeight.bold,
                                                color: isMe
                                                    ? const Color(0xFFDBEAFE)
                                                    : const Color(0xFF94A3B8),
                                              ),
                                            ),
                                          if (!isSystem)
                                            const SizedBox(height: 4),
                                          Text(
                                            msg.content,
                                            style: TextStyle(
                                              fontSize: 14,
                                              fontStyle: isSystem
                                                  ? FontStyle.italic
                                                  : null,
                                              color: isSystem
                                                  ? const Color(0xFF64748B)
                                                  : isMe
                                                  ? Colors.white
                                                  : const Color(0xFF1E293B),
                                            ),
                                          ),
                                        ],
                                      ),
                                    ),
                                  );
                                },
                              );
                            },
                          ),
                        ),

                        // Input Area
                        Container(
                          padding: const EdgeInsets.all(16),
                          decoration: const BoxDecoration(
                            color: Color(0xFFF8FAFC), // slate-50
                            border: Border(
                              top: BorderSide(color: Color(0xFFE2E8F0)),
                            ),
                            borderRadius: BorderRadius.vertical(
                              bottom: Radius.circular(11),
                            ),
                          ),
                          child: Row(
                            children: [
                              SizedBox(
                                width: 100,
                                child: TextField(
                                  controller: _senderCtrl,
                                  decoration: InputDecoration(
                                    filled: true,
                                    fillColor: Colors.white,
                                    hintText: "Name",
                                    contentPadding: const EdgeInsets.all(12),
                                    border: OutlineInputBorder(
                                      borderRadius: BorderRadius.circular(8),
                                      borderSide: const BorderSide(
                                        color: Color(0xFFCBD5E1),
                                      ),
                                    ),
                                  ),
                                ),
                              ),
                              const SizedBox(width: 8),
                              Expanded(
                                child: TextField(
                                  controller: _msgCtrl,
                                  onSubmitted: (_) => _sendMessage(),
                                  decoration: InputDecoration(
                                    filled: true,
                                    fillColor: Colors.white,
                                    hintText: "Type a message...",
                                    contentPadding: const EdgeInsets.all(12),
                                    border: OutlineInputBorder(
                                      borderRadius: BorderRadius.circular(8),
                                      borderSide: const BorderSide(
                                        color: Color(0xFFCBD5E1),
                                      ),
                                    ),
                                  ),
                                ),
                              ),
                              const SizedBox(width: 8),
                              ElevatedButton(
                                onPressed: _sendMessage,
                                style: ElevatedButton.styleFrom(
                                  backgroundColor: const Color(0xFF2563EB),
                                  foregroundColor: Colors.white,
                                  padding: const EdgeInsets.symmetric(
                                    horizontal: 24,
                                    vertical: 20,
                                  ),
                                  shape: RoundedRectangleBorder(
                                    borderRadius: BorderRadius.circular(8),
                                  ),
                                ),
                                child: const Text(
                                  "Send",
                                  style: TextStyle(fontWeight: FontWeight.bold),
                                ),
                              ),
                            ],
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
