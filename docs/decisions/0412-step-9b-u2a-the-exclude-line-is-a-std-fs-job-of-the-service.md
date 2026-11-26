# ADR-0412: Step 9b U2a: the exclude line is a std-fs job of the service spawner; the bundle record name and its temps; one trash take per hold

- 상태: 확정 (2026-11-23, 근거: main decision during step-9b U2a coding · code review deep (lead, risk axis, Codex) PASS 2026-11-23)
- 관련: Amends ADR-0409 (M12 the exclude line write path) · ADR-0400 (decision 24) · ADR-0365 · ADR-0360 · T-124 · step-9b TRD `docs/process/S25-preset-library/trd-file-operations.md` §3.9

## 맥락
ADR-0409 M12 said the `.git/info/exclude` line of a mount's `.trash` is written after the hold "through `BoundedIo`". `BoundedIo` (`folder.rs`) has no no-follow stat and no write, so step-9b U2a wrote the line another way. U2a also had to name the bundle record file, tell Engram's own record temps from other files at a bundle's top, and decide what a second take of the trash lock in one hold does. The review found that base `write_atomic` opens its temp with `File::create` (follows a link planted at the temp name — T-124).

## 결정
1. **The exclude line is written by a job of the service's spawner with std `fs`** (`LibraryService::exclude_trash` → `trash::exclude_trash`), after the hold. The step never waits for it and holds no library lock while it runs. A share that stops answering holds that job and the exclude mutex; later exclude jobs queue behind it. This replaces "through `BoundedIo`" in ADR-0409 M12; the rest of M12 (anchored and escaped line, bytes kept, one leaf mutex, skipped when `info` or the exclude file is a link, never a failure of the step) stands.
2. **The exclude file is replaced through a create-new temp** beside it (a taken temp name draws another), synced, then renamed over the exclude file — not through base `write_atomic`.
3. **Where the line goes.** `.git` and the common git folder (after `gitdir:` and `commondir`) must be real folders holding `HEAD`; a linked `.git`, a stale `gitdir:` and a plain `.git` folder that is no repository are skipped, and nothing is created above `info` (`info` itself one level with `create_dir`). A mount path below the repository top holding a control character is skipped.
4. **The bundle record is `.engram-bundle.json`** at the bundle top. A record temp is a base temp name whose whole target (base `file::temp_target`) is that name; any other file there — a temp of another name included — is a fallback row and keeps the bundle.
5. **The trash lock is taken once per hold.** `Hold::take_trash` answers `Err(NotReserved)` for a hold that did not raise the waiting count and `Err(AlreadyHeld)` for a second take in the same hold (a bug in the body), never a wait on itself.

## 거부한 대안
- **Write the exclude line through `BoundedIo`** — it has neither a no-follow stat nor a write; adding both to `folder.rs` for a write the step does not wait on anyway. (code)
- **Base `write_atomic` for the exclude file** — its temp open follows a link already at the temp name and truncates its target (T-124). (code)
- **Follow a linked `.git`, as git does** — the write would land wherever the link points, outside the mount's repository folder as seen; ADR-0365 skips links on every path. (code: `is_git_folder` uses a no-follow stat)
- **A second `take_trash` waits** — the only possible holder under the write lock is the same hold, so it would wait forever holding the library write lock. (code)

## 근거
- Code review deep of U2a (lead, risk axis, Codex) rounds 1–3, PASS 2026-11-23: the record-temp match, the bundle check before any read, the create-new exclude temp, the stale-`gitdir:` guard and the re-entrant take were review findings, each fixed with a test.

## 영향 / 불변식
- `trash.rs` (`exclude_trash`, `replace_file`, `is_git_folder`, `is_record_name`, `RECORD_NAME`), `lib.rs` (`exclude_trash`, the `exclude` mutex), base `file.rs` (`temp_target`), `gate.rs` / `trash.rs` (`Hold::take_trash`, `TrashUnavailable`).
- Accepted: exclude jobs are not capped — a stalled share leaves one blocked thread per later trash step on that repository until it answers. The library and the steps are never held.
- Later units: a step takes `take_trash()` once before its per-mount loop and keeps it; the view and restore treat `.engram-bundle.json` and its temps as Engram's own.
