// GENERATED CODE – DO NOT EDIT.
// ignore_for_file: unused_import
// ignore_for_file: invalid_null_aware_operator

import 'dart:typed_data';

class CollectionRequest {
  final int maxResults;
  final String personId;

  const CollectionRequest({
    required this.maxResults,
    required this.personId,
  });

  factory CollectionRequest.fromJson(Map<String, dynamic> json) {
    return CollectionRequest(
      maxResults: json['max_results'] as int,
      personId: json['person_id'] as String,
    );
  }

  Map<String, dynamic> toJson() {
    return {
      'max_results': maxResults,
      'person_id': personId,
    };
  }
}

class Person {
  final String id;
  final String name;

  const Person({
    required this.id,
    required this.name,
  });

  factory Person.fromJson(Map<String, dynamic> json) {
    return Person(
      id: json['id'] as String,
      name: json['name'] as String,
    );
  }

  Map<String, dynamic> toJson() {
    return {
      'id': id,
      'name': name,
    };
  }
}

