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
- A cloud placeholder (offline / recall-on-access attributes, or flagged in the index) is never opened; its identity is `cloud placeholder`.
- The same holds on a second inspect, whether served from `evidence_cache` or recomputed.

### EVID-IN-USE: Open-handle detection
**Applies to:** hoardini
**Verification:** test

`inspect` shall list the processes that hold a file open (Restart Manager). For a directory it shall check the files inside,
largest first, up to a cap of 64 (`evidence::inuse::DIR_FILE_CAP`). Evidence is advisory; the policy gate checks up to 500
files of a folder at proposal and execution time (POLICY-IN-USE).

Measured 2026-10-08 on this PC (heavily loaded, debug build, C: index of 936k dirs, 5.9M files): one Restart Manager
session over the 500 largest files of a cold folder took 10–76 s, over 64 files 1–8 s, warm 0.1–0.4 s. Asking the kernel
per file (`FileProcessIdsUsingFileInformation`) was slower still (≈ 0.18 s per file), so the cap is the lever. With the cap
and the subtree-query fix, `inspect` of `~\.lmstudio` took 0.7–1.9 s per call (Restart Manager ≈ 1.3 s of it; one 12 s
outlier under load), against 11.4 s before. Folders of 6k+ files elsewhere on C: inspected in 0.6–1.4 s.

**Acceptance criteria:**
- A file held open is reported with the holder's process name and pid; once the handle is closed it is not.
- For an indexed directory containing that file, the file's holder is reported (one Restart Manager session over the largest files).
