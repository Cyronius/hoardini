# Evidence engine (prefix EVID)

`inspect(path)` builds one deterministic `Evidence` record (no LLM). Canonical design: `.claude/plans/hoardini-design.md` §5.3.
Code: `crates/core/src/{catalog,evidence}`, data: `catalog/*.toml`.

### EVID-CATALOG: Known-location catalog
**Applies to:** hoardini
**Verification:** test

For an absolute path the catalog shall return the most specific matching entry (deepest matching ancestor-or-self wins;
ties go to the longer literal pattern), supplying `app`, `category`, `regenerable`, `verdict_hint`, an optional policy `cap`,
`shared_blobs`, `safe_unit`, and `notes`. Patterns support `~`, `%VAR%` expansion, `*`, `?`, `**`, and "any directory
named X" (`**\X`); matching ignores case. Shared-blob stores shall name their safe unit and carry a `review` cap; hiberfil.sys
and pagefile.sys shall carry a `keep` cap.

**Acceptance criteria:**
- `~\.lmstudio\models\TheBloke\x\x.Q4_K_M.gguf` matches app `LM Studio`, category `model-store`, hint `redownload`, notes mention "whole model folder".
- `~\.cache\huggingface\hub\models--a--b\blobs\0123` matches `hf-hub-blobs` with `shared_blobs`, cap `review`, safe unit "a whole models--org--name folder"; a snapshot path under the same repo matches the shallower `hf-hub-cache`.
- `C:\hiberfil.sys` and `C:\pagefile.sys` carry cap `keep`.
- `C:\code\app\node_modules\react\index.js` matches `node-modules`, regenerable.
- A path with no entry (`D:\photos\2024\a.jpg`) matches nothing.
- The seed list resolves: Ollama blobs, Triton, MIOpen, NuGet, cargo registry, `.venvs`, llama.cpp build, Rust `target`, `$WINDOWS.~BT`, `SoftwareDistribution\Download`, `Windows\Installer`, `ESD`, Docker `.vhdx`, Steam libraries, Downloads installers, `C:\models`, `unsloth_compiled_cache`.

### EVID-GGUF: GGUF header identity
**Applies to:** hoardini
**Verification:** test

`identify` shall read a GGUF file's header and tensor-info region only (never tensor data) and report the model name,
architecture, quantization name (from `general.file_type`, else the dominant tensor type), size label, context length,
and parameter count (sum of tensor element counts).

**Acceptance criteria:**
- A synthetic GGUF v3 with `general.architecture=llama`, `general.name="Tiny Test 7B"`, `general.file_type=15`, `llama.context_length=32768`, a string-array KV, and two tensors yields name, architecture `llama`, quant `Q4_K_M`, context 32768, parameters equal to the tensors' element sum.
- Without `general.file_type` the quant is the tensor type holding most elements (F16 here).
- For a file with 5 MB of trailing payload, the reader does not read beyond the first buffered block (64 KB) of the file.

### EVID-ATIME-PRESERVE: Reads leave last-access time unchanged
**Applies to:** hoardini
**Verification:** test

Header reads and hashes performed by the evidence engine shall not change a file's last-access time. The engine shall open
the handle with `FILE_WRITE_ATTRIBUTES` and set last-access to `0xFFFFFFFF` before the first read; if that open fails it
skips the read and reports identity "unknown (no access)".

**Acceptance criteria:**
- After `inspect` reads a file's header and hashes it, the file's last-access time equals its value before the call.
- When the preserving open fails, identity is `unknown (no access)` and no read occurs.

### EVID-IN-USE: Open-handle detection
**Applies to:** hoardini
**Verification:** test

`inspect` shall list the processes that hold a file open (Restart Manager). For a directory it shall check the files inside,
largest first, up to a cap (default 500).

**Acceptance criteria:**
- A file opened by a helper process is reported with that process's name and pid; after the process exits it is not.
- For a directory containing that file, the file's holder is reported.
