// GENERATED CODE – DO NOT EDIT.
// ignore_for_file: unused_import
// ignore_for_file: invalid_null_aware_operator

import 'dart:typed_data';

class Item {
  final String? description;
  final String id;
  final String ownerId;
  final String title;

  const Item({
    this.description,
    required this.id,
    required this.ownerId,
    required this.title,
  });

  factory Item.fromJson(Map<String, dynamic> json) {
    return Item(
      description: (json['description'] == null ? null : json['description'] as String),
      id: json['id'] as String,
      ownerId: json['owner_id'] as String,
      title: json['title'] as String,
    );
  }

  Map<String, dynamic> toJson() {
    return {
      'description': description,
      'id': id,
      'owner_id': ownerId,
      'title': title,
    };
  }
}

class ItemCreate {
  final String? description;
  final String title;

  const ItemCreate({
    this.description,
    required this.title,
  });

  factory ItemCreate.fromJson(Map<String, dynamic> json) {
    return ItemCreate(
      description: (json['description'] == null ? null : json['description'] as String),
      title: json['title'] as String,
    );
  }

  Map<String, dynamic> toJson() {
    return {
      'description': description,
      'title': title,
    };
  }
}

class Token {
  final String accessToken;
  final String tokenType;

  const Token({
    required this.accessToken,
    required this.tokenType,
  });

  factory Token.fromJson(Map<String, dynamic> json) {
    return Token(
      accessToken: json['access_token'] as String,
      tokenType: json['token_type'] as String,
    );
  }

  Map<String, dynamic> toJson() {
    return {
      'access_token': accessToken,
      'token_type': tokenType,
    };
  }
}

class User {
  final dynamic email;
  final String id;
  final String role;

  const User({
    required this.email,
    required this.id,
    required this.role,
  });

  factory User.fromJson(Map<String, dynamic> json) {
    return User(
      email: json['email'],
      id: json['id'] as String,
      role: json['role'] as String,
    );
  }

  Map<String, dynamic> toJson() {
    return {
      'email': email,
      'id': id,
      'role': role,
    };
  }
}

class UserCreate {
  final dynamic email;
  final String password;

  const UserCreate({
    required this.email,
    required this.password,
  });

  factory UserCreate.fromJson(Map<String, dynamic> json) {
    return UserCreate(
      email: json['email'],
      password: json['password'] as String,
    );
  }

  Map<String, dynamic> toJson() {
    return {
      'email': email,
      'password': password,
    };
  }
}

