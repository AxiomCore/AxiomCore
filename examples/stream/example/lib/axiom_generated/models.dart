// GENERATED CODE – DO NOT EDIT.
// ignore_for_file: unused_import
// ignore_for_file: invalid_null_aware_operator

import 'dart:typed_data';

class ChatMessage {
  final String text;
  final String user;

  const ChatMessage({
    required this.text,
    required this.user,
  });

  factory ChatMessage.fromJson(Map<String, dynamic> json) {
    return ChatMessage(
      text: json['text'] as String,
      user: json['user'] as String,
    );
  }

  Map<String, dynamic> toJson() {
    return {
      'text': text,
      'user': user,
    };
  }
}

