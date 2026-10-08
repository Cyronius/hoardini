# RUNTIME: Pinned downloadable runtimes

Canonical spec for `crates/core/src/runtime/`. Design reference: `hoardini-design.md` sections 5.6 (rclone) and 5.12 (llama-server and models).

### RUNTIME-PINNED-HASH: Downloaded binaries and models run only if they match their pinned SHA-256
**Applies to:** hoardini-core
**Verification:** test

rclone, llama.cpp and the built-in model files are downloaded on demand from a URL pinned in the build, together with a SHA-256 and a size. A download whose hash differs shall be deleted and never run. Before every launch `verify_before_run` shall confirm the file against its install record (size and mtime) and the pinned hash, re-hashing when the record is missing or stale. Executables are never resolved from `PATH`. Zip pins extract only the listed members.

**Acceptance criteria:**
- Serving bytes whose SHA-256 equals the pin: `ensure` returns the installed path, the file holds the served bytes, progress reports `(5000, 5000)` at the end, and `verify_before_run` is Ok.
- Serving bytes whose SHA-256 differs: `ensure` errors with a SHA-256 message and leaves nothing in `models\` or any `.part` file in the root.
- Appending one byte to an installed file makes `verify_before_run` error (and, for a `File` pin, deletes the stale record).
- Deleting the `.verified` record of an intact `File` install makes `verify_before_run` re-hash, succeed and write the record again.
- A zip pin with `extract: ["pkg/tool.exe"]` over an archive holding three members leaves exactly `tool.exe` and `.verified` in `runtime\tool-1.0\`.
- A set cancel flag aborts the download and leaves nothing behind.
- Every entry of `runtime::PINS` has a 64-character lowercase hex sha256, an `https://` url and license url, and a non-zero size.

**Pins (as of 2026-10-08):**

| id | version | size | hash source |
|---|---|---|---|
| rclone | 1.75.1 (windows-amd64 zip) | 31,488,864 | publisher `SHA256SUMS`, confirmed by a local download |
| llama.cpp | b11503 (win-cpu-x64 zip) | 19,483,533 | GitHub release asset digest, confirmed by a local download |
| lfm2.5-1.2b-instruct Q4_K_M | HF commit 8ed2880 | 730,895,168 | Hugging Face tree API LFS oid, matches `X-Linked-ETag` |
| lfm2.5-2.6b Q4_K_M | HF commit e7caca5 | 1,674,455,040 | same |
| qwen3-4b-instruct-2507 Q4_K_M (Apache-2.0) | HF commit a06e946 | 2,497,281,120 | same |
| granite-4.0-micro Q4_K_M (Apache-2.0) | HF commit ec48475 | 2,099,502,528 | same |

Bumping a pin means re-reading the publisher checksum, never copying a hash from a mirror.
