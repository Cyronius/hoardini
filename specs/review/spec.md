# Review specification (prefix REVIEW)

The review sweeps a drive (or a folder) with no model call and lists what could go and what is big but stays.
Plan: `specs/review/plans/review-first.md`. Code: `crates/core/src/review`. Tests: `specs/review/tests/review.rs`
(`cargo test -p hoardini-specs --test review`).

### REVIEW-UNITS: Items resolve to deletable units
**Applies to:** hoardini
**Verification:** test

`review::units::resolve(conn, env, lister, root, now)` shall walk down from `root` (a folder or a drive root in the
index) and return the units it stops at, each with its `path`, `kind` (`self`, `children` or `contents`), `why`
(`catalog`, `model-store`, `leaf` or `kept`), `catalog_id` when an entry decided it, `container` (the folder the walk came
from, or the store for `children` units; none for `root` itself), `size`, and for `contents` units the `parts` past the
entry's minimum age. At each folder, in order:

1. A protect note covering it, or a `keep` location (MEM-LOCATION) covering it that came from the user, a job or the
   content mix, makes it one `kept` unit; the walk does not go inside.
2. A catalog entry naming the folder itself (`Catalog::lookup_exact`; an entry matched only through an ancestor does
   not count) whose verdict is not `keep` and whose `unit` is not `container` decides by its `unit`
   (EVID-CATALOG): `self` makes the folder one unit; `children` makes each child one unit (each grandchild when
   `unit_depth = 2`); `contents` makes one unit of the folder whose `parts` are its children, and with `min_age_hours`
   only the children not modified for that long (a child counts as modified when it or any indexed folder or file inside
   it was). A file a `contents` entry names is a `self` unit.
3. A Windows system area of the deny list (POLICY-DENY: the Windows folder, Program Files, `ProgramData\Microsoft`,
   `System Volume Information`, `$Recycle.Bin`, paging files, Hoardini's data) or a catalog entry with verdict `keep`
   makes it one `kept` unit. The walk then visits the catalog locations named literally inside it (no wildcards) whose
   verdict is not `keep` and whose unit is not `container`, so `Windows\Temp` and `SoftwareDistribution\Download` are
   still found.
4. A file is a `self` `leaf` unit.
5. With no catalog entry naming it, a model store makes each child one `children` unit (`why = model-store`). A folder
   is a model store when at least 80% of its bytes are model weights and at least 80% of those weight bytes lie in
   files directly inside its child folders. Weights are files of 16 MiB or more ending in `.gguf`, `.safetensors`,
   `.onnx`, `.pt` or `.ckpt`, `.bin` next to a `config.json` (checked by listing the folder), and every type a file-type
   row files under `model-weights` (MEM-FILETYPE), such as `.q4nx`.
6. Otherwise the walk goes into every child of at least 1 GiB or at least 10% of the folder and repeats; a folder with no
   such child is one `self` `leaf` unit. A catalog entry with unit `container` (AppData, `~\.lmstudio`, Program Data)
   only lets the walk go on this way.

**Acceptance criteria:**
- `~\.flm` holding `models\Qwen3.6-35B-A3B-NPU2` (60 GiB of `.q4nx`), `models\Qwen3.8-27B-NPU2` (40 GiB) and a small `logs` folder resolves to the two model folders: `children`, catalog id `flm-models`, container `~\.flm\models`; `~\.flm` itself is no unit.
- The same tree at `C:\npu`, which no catalog entry names, ends at the two `.q4nx` files (`leaf`); once `.q4nx` is filed under `model-weights` it resolves to the two model folders: `children`, `model-store`, no catalog id, container `C:\npu\models`.
- `C:\hf\bert` and `C:\hf\gpt`, each holding a `pytorch_model.bin` and a `config.json`, make `C:\hf` a model store with those two units; the same `.bin` files without a `config.json` (`C:\blobs`) end at the files.
- In `C:\code\proj`, `target` (3 GiB of build output) is one `self` unit with catalog id `rust-target` and container `C:\code\proj`; the 4 KiB `src` is not visited.
- `C:\work` goes into `big` (2,000 MiB of 50 MiB files) and `medium` (300 MiB, 12.8% of the folder) and not into `tiny` (50 MiB, 2.1%); both are `leaf` units, since no child is 1 GiB or 10% of them.
- `%TEMP%` is one `contents` unit with catalog id `temp-user`: a child folder whose file is 3 days old and a 3-day-old file are its `parts`, a child folder holding a file written an hour ago is not, and `size` is 7 GiB, the sum of the parts. A walk of the whole drive reaches the same unit.
- `C:\SteamLibrary\steamapps\common` gives one `children` unit per game (`steam-common`).
- `%SystemRoot%` is one `kept` unit, and `%SystemRoot%\Temp` inside it is still a `contents` unit (`temp-system`).
- After the user answers Keep for `C:\code\proj`, a walk of `C:\code` gives that folder as one `kept` unit and not its `target`.

Test: `specs/review/tests/review.rs`.
