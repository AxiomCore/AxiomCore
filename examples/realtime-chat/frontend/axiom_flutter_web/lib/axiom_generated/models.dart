// GENERATED CODE – DO NOT EDIT.
// ignore_for_file: unused_import
// ignore_for_file: invalid_null_aware_operator

import 'dart:typed_data';

class ChatMessage {
  final String content;
  final String sender;
  final String timestamp;

  const ChatMessage({
    required this.content,
    required this.sender,
    required this.timestamp,
  });

  factory ChatMessage.fromJson(Map<String, dynamic> json) {
    return ChatMessage(
      content: json['content'] as String,
      sender: json['sender'] as String,
      timestamp: json['timestamp'] as String,
    );
  }

  Map<String, dynamic> toJson() {
    return {
      'content': content,
      'sender': sender,
      'timestamp': timestamp,
    };
  }
}

