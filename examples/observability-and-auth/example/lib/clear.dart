// Add this to your main() BEFORE AxiomSdk.create() to clear any stale
// auth tokens that old code wrote with empty bytes or ttl=0.
//
// You only need this once during the transition to the fixed store.rs.
// After all clients have the fix deployed you can remove this block.

import 'dart:js_interop';
import 'package:web/web.dart' as web;

void clearStaleAxiomAuthTokens() {
  // Only runs in the browser — no-op on native.
  try {
    final storage = web.window.localStorage;
    final keysToRemove = <String>[];

    for (var i = 0; i < storage.length; i++) {
      final key = storage.key(i);
      if (key != null && key.startsWith('axiom_cache_auth_')) {
        keysToRemove.add(key);
      }
    }

    for (final key in keysToRemove) {
      storage.removeItem(key);
    }

    if (keysToRemove.isNotEmpty) {
      // ignore: avoid_print
      print(
        '[Axiom] Cleared ${keysToRemove.length} stale auth token(s) from localStorage',
      );
    }
  } catch (_) {
    // Not in a browser context — safe to ignore.
  }
}
