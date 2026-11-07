# ★ Has the file changed — practice for telling without opening the content — light survey

| Item | Value |
|---|---|
| Status | survey done · application = follow practice (user delegation 2026-10-17 「관행 확인해보고 이부분은 그냥 맞춰」) |
| Date | 2026-10-17 |
| Method | `/research` light — 1 collector (about 13 searches · fetches) · 1 main-session grounding spot check (git 「racy-git」 doc via WebFetch) · no adversarial review |
| Question | The daemon's library read-cache tag = (path, modification time, size) — on a rescan, only files whose tag differs are reopened. Is this practice, and how are the traps (edited in the same instant · time precision · swapped-in files) prevented |

## Conclusions

- **(modification time, size) is a cheap gate for filtering 「바뀌었을 수 있다」** — places that must be exact add a 「아슬아슬한 시각」 (borderline time) rule or a content hash to it (git · Bazel · Unity). Cargo · Ninja · make look only at the modification time and accept the traps. (likely)
- **The fields git compares** — primary source: 「Git compares the file type … and executable bits … `st_mtime` and `st_ctime` timestamps, `st_uid`, `st_gid`, `st_ino`, and `st_size` members.」 (certain — main checked)
- **Edited in the same instant (racy git) and its fix** — primary source: the tag is unchanged but 「the `st_mtime` is the same as (or newer than) the timestamp of the index file itself … it also compares the contents」 · such an entry is 「racily clean」 · when it is rewritten, 「cached `st_size` information is truncated to zero」 so that a content comparison is guaranteed to happen next time. (certain — main checked)
- **Bazel** = the content hash is the real basis and (path · inode · mtime · size + ctime since 6.1) the tag is used to avoid recomputing the hash — the reason ctime was added: a file with the same mtime being swapped in by `mv`. (likely — search summary)
- **Windows times** — the Microsoft docs state NTFS 100ns · FAT write time 2 seconds · the write time is exact only after the handle that was writing is closed. (certain — collector fetched)

## Findings

| Tool | Fields compared | Content hash | Borderline · precision handling | Source | Confidence |
|---|---|---|---|---|---|
| git index | mode · mtime · ctime · uid · gid · ino · size | only when a field differs or the entry is racy | mtime ≥ index file time → compare content · a racy entry is recorded with size 0 so a comparison is forced next time | https://git-scm.com/docs/racy-git (main checked) | certain |
| make | target mtime vs prerequisite mtime (rebuild if newer) | none | none — clock skew · coarse time are known problems | — | likely (memory) |
| Ninja | output mtime vs input · command line in `.ninja_log` · `restat` | none | no precision handling in the manual | https://ninja-build.org/manual.html | certain |
| Cargo | local source mtime vs dep-info mtime | none | records the build start time to catch edits made mid-build | https://doc.rust-lang.org/nightly/nightly-rustc/cargo/compiler/fingerprint/index.html | likely (search summary) |
| Bazel | (path · inode · mtime · size · ctime) → hash cache | yes — the hash is the basis | adds ctime (swapping) | https://bazel.googlesource.com/bazel/+/refs/heads/release-6.1.2 | likely |
| Unity AssetDatabase | last modification time + content hash | yes | not found | https://docs.unity3d.com/ru/Manual/AssetDatabase.html | likely |
| Windows times | NTFS 100ns · FAT 2 seconds (local time) · access time can lag by up to 1 hour | — | write time is exact after the handle is closed | https://learn.microsoft.com/en-us/windows/win32/sysinfo/file-times | certain |

## Application (follow practice — main-session decision)

1. Tag = (path · modification time at full precision · size · file identity [volume number + file number — the equivalent of inode]) · **equality comparison** (not a 「더 새것」 comparison — clock skew · restored files) · re-read if even one differs.
2. Borderline rule (git style): record the scan time T alongside, mark an entry whose modification time is at least T − margin as 「아슬아슬」 so it is unconditionally re-read on the next scan · margin ≈ a few ms for NTFS · 2 seconds for FAT · exFAT · network volumes · once a later scan sees it as old enough, the mark is cleared.
3. Do not hash every file — re-read only entries that are borderline or whose file identity alone changed (re-reading is more direct than hashing — not measured).

## Gaps

- Not confirmed: the primary text of Cargo's list of traps · the meaning of git `core.trustctime` / `core.checkStat` · details of Unity Auto Refresh.
- Not looked at: removing the scan itself through Windows change notification (ReadDirectoryChangesW · USN journal) — watching is an already rejected decision (PRD R19).
- Whether the NTFS write time changes immediately on every write (or when the handle is closed) was only partly confirmed.
