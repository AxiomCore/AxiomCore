# Public history migration — 7 October 2026

Public branches and tags were sanitized to remove operational/editorial documents and generated source captures that were not intended for public distribution. Existing release asset versions and checksums are unchanged. Rewritten source tags have new commit/object IDs; an original tag's signature does not attest to its rewritten object.

Use a fresh clone. Preserve local uncommitted work separately and replay reviewed changes onto the current branch. Do not merge previous branch histories or push old tags back. The public-source CI check rejects reintroduction of selected internal/generated paths anywhere in reachable history.

Corrected macOS ARM64 compiler/editor downloads are Acore LSP 0.1.2 and CLI 0.148.1 (`cli-v0.148.1`). LSP 0.1.0 and 0.1.1 remain withdrawn. Previously downloaded copies and independent caches cannot be recalled. Public source and executable distribution retain their respective license terms.
