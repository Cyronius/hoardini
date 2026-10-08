# Secrets vault (prefix VAULT)

API keys, SMB passwords, search keys, the rclone config password, and OAuth tokens live in one DPAPI-encrypted file.
Canonical design: `.claude/plans/hoardini-design.md` §5.9. Code: `crates/core/src/vault`. Tests: `specs/vault/tests/vault.rs`.

## Model

`DpapiVault` stores `secrets.vault` in the data folder: `CryptProtectData` (user scope, entropy `hoardini-vault-v1`) over
a JSON map from reference to value. Every change rewrites the file through a temp file and a rename. Values are held as
`SecretString` (zeroized on drop); `Debug` never prints them. A file that cannot be decrypted (another Windows user, or
corrupt) is reported, never overwritten. References look like `secret://provider/<id>`,
`secret://destination/<id>/password`, `secret://search/<kind>`; config rows and files hold only these references, and
`destinations::create`/`update` reject a secret-looking config value that is not a reference.

### VAULT-AT-REST: No plaintext secret at rest
**Applies to:** hoardini
**Verification:** test

No secret value shall appear in plaintext in the vault file, the SQLite database (including its WAL), or any config
file in the data folder. Every reference shall still resolve to its value after the vault is reopened.

**Acceptance criteria:**
- With a provider API key, an SMB destination password, a search key, and an rclone password stored in the vault, and provider, destination, and search-provider rows plus a settings file referencing them, no file in the data folder contains any of the four values as UTF-8 or UTF-16LE bytes; `hoardini.db-wal` and `secrets.vault` are among the files scanned.
- Creating an SMB destination with a plaintext `password` is rejected with a message pointing to `secret://`.
- After reopening the vault, each reference returns its original value and `refs()` lists all four; `Debug` output of the vault and of a returned secret does not contain the value.
