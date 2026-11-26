# Leftover temp files from atomic writes — when and how they get cleaned up (practice survey · light)

- **Status:** light · 2026-10-31 · collector 1 (worker-scout) · 2 main grounding spot-checks (Syncthing and VS Code, checked against the source text — both supported). No adversarial review (light).
- **Trigger:** In the S25 TRD, the preset file is written to a temp file in the same folder and then swapped in (G38). The leftover temp file that remains if the process dies was written as 「데몬 기동 때 쓸기」, which collides with user decision 「데몬이 켜질 때 미리 훑지 않는다」 (2026-10-17) (TRD §1-11a finding 15).
- **Confidence legend:** certain = primary source (source code or official docs) checked · likely = single source / forum staff answer · uncertain = no source.

## Conclusion

- **None of the sampled tools sweeps leftovers by scanning at app startup.** Two approaches are common:
  1. **Do not clean up — a fixed name means the next write to the same file overwrites it** (VS Code · git `index.lock` · Rust crates · JetBrains tells the user to delete by hand).
  2. **Clean up lazily during a folder scan or indexing — a recognizable prefix plus an age threshold** (Syncthing — prefix + 24 hours · at rescan / git `prune` — removes only the expired `tmp_obj_*`).
- Common shape = **the temp file is in the same folder** (for the swap to be atomic it must be on the same volume) · **the name carries an app marker** (prefix or suffix) · **the scanning side skips that name and deletes only old ones.**
- The only way to remove the leftover itself is Linux `O_TMPFILE` (an unnamed file) — on Windows a named temp file cannot be avoided.

→ Applied to the S25 candidates: **(a) clean up at first scan** has the same shape as Syncthing and git prune (the same if an age threshold is added). (b) A separate hidden folder has no precedent in the sample. (c) Not cleaning up is also common (with a fixed name, the next write overwrites it).

## Per tool

| Tool | Location · name | When it cleans up | Confidence · source |
|---|---|---|---|
| Syncthing | same folder · Windows `~syncthing~<이름>.tmp` · Unix `.syncthing.<이름>.tmp` · hashed if over 160 characters | At folder rescan, deletes those older than `keepTemporariesH` (default 24h) (the check runs about hourly → up to about 2x delay). Leaves them on purpose so a retry can reuse the blocks | name · `IsTemporary()` = certain (`lib/fs/tempname.go` — main checked the source text) · policy = certain (forum · issue #3215) · the call site where the scan skips by prefix = likely |
| VS Code file service | same folder · fixed suffix `foo.txt.vsctmp` | No cleanup code — fixed name + overwrite, so the next atomic write to the same file overwrites it. Atomic write is refused for symbolic links | certain (`src/vs/platform/files/common/files.ts` about 314–323 — main checked the source text · `diskFileSystemProvider.ts` `doWriteFileAtomic`) · whether it is used only for internal state files = likely |
| JetBrains 「safe write」 | same folder · `이름___jb_tmp___` → rename the original to `___jb_old___` → tmp becomes the original → delete old | No evidence of startup cleanup — tells the user to delete by hand or turn the setting off | likely (support forum staff answer) · cleanup = uncertain |
| git | same folder `<경로>.lock` · `tmp_obj_*` in `objects/` | Lock: removed on normal exit and on signals, but an `index.lock` left after a forced kill is **not removed automatically**; git refuses and leaves it to a person (staleness cannot be decided because of PID reuse) · data temp: `git prune`/`gc` delete by age | certain (`lockfile.h` · `lockfile.c` · `builtin/prune.c`) |
| Logseq | not an atomic write (in-place write) · `<graph>/logseq/bak/` backup only on failure | No leftover — but a truncated file is possible if the process dies | likely (`handler.cljs` 103–130) |
| Obsidian | unknown (closed source) — only an atomic-write request post (2023) exists, with no staff answer | unknown | uncertain |
| Rust `atomic-write-file` · `tempfile` | same folder · `.이름.XXXXXX` / `.tmp` + random | No cleanup mechanism — the docs state that it 「갑자기 끊기면 남을 수 있다」 | certain (docs.rs) |

## Gaps

- No primary source on whether Obsidian or JetBrains cleans up at startup or on open.
- Not confirmed how the VS Code watcher filters `.vsctmp`, or whether editor save is atomic by default.
- Per-tool choice between Windows `ReplaceFile` and `MoveFileEx(MOVEFILE_REPLACE_EXISTING)` not surveyed.
- No precedent found for creating in a separate hidden folder and moving (candidate b).

## Sources

- https://github.com/syncthing/syncthing/blob/main/lib/fs/tempname.go
- https://forum.syncthing.net/t/when-sync-error-how-long-will-remove-or-keep-the-temp-file-syncthing-xxx/1126 · https://github.com/syncthing/syncthing/issues/3215
- https://github.com/microsoft/vscode/blob/main/src/vs/platform/files/common/files.ts · `src/vs/platform/files/node/diskFileSystemProvider.ts`
- https://intellij-support.jetbrains.com/hc/en-us/community/posts/206864695/comments/206381175
- https://github.com/git/git/blob/master/lockfile.h · `lockfile.c` · `builtin/prune.c`
- https://forum.obsidian.md/t/atomic-file-writes/56065
- Logseq `src/electron/electron/handler.cljs` · `backup_file.cljs`
- https://docs.rs/atomic-write-file/latest/atomic_write_file/ · https://docs.rs/tempfile/latest/tempfile/
