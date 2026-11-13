# ADR-0365: Links inside the library are skipped on every path; only the configured library root may be a link

- 상태: 확정 (2026-11-13, 근거: user decision U1 2026-11-11 (S25 step 7 TRD) · child TRD `docs/process/S25-preset-library/trd-asset-library.md` §10-1 U1 · landed in P5 (store, scan) through P9 (launch overlay))
- 관련: ADR-0312 (one root — outside content is brought inside; decision 3) · ADR-0320 (trash) · ADR-0324 (catalog writes) · ADR-0336 (walkdir) · ADR-0362 (`LibPath`) · child TRD §10-1 U1 · §6 (admission) · §12 (store row)

## 맥락
ADR-0312 decision 3 keeps outside content out of the library: it is copied in, not used in place. A link (a symlink or a Windows junction) inside the library root still lets outside content in — a link to `~/.claude/skills` placed in the library would make the scan list, the launch read and later moves or deletes reach the far side. The step-7 TRD had to say what the store does with a link below the root on each path into the catalog: the full scan, a partial scan, the `list` safety net, the launch check's fresh-bytes overlay and the late merge. The user's view: 「애초에 바깥폴더를 못넣게 했잖아. 근데 어찌어찌 들어가더라도 건너띄는게 좋지 않음?」.
Convention (main 2026-11-11): generic walkers (walkdir by default, ripgrep) do not follow links; skill loaders (codex skill discovery, the Claude Code CLI) do.

## 결정
1. **Only the configured library root may be a link**; the root is followed.
2. **Below the root, a link (folder or file) is never entered, listed, read or merged — on every path into the catalog**: the full scan, a partial scan, the `list` safety net, the launch overlay reads and the late merge. A partial scan whose range is a link or lies below one walks nothing.
3. **A link is reported, not hidden**: `library.refresh` names the skipped links. A reference at or below a link inside the library resolves to a ② row with the code `path-through-link` (`args` `{path}`), never to its far side and never to 「찾을 수 없음」.

## 거부한 대안
- **Follow every link** (as skill loaders do) — the scan would list the far side, and a later move or delete (9b) would reach files outside the library, against ADR-0312 decision 3 (G37). Offered with (a) and (c); the user chose (a). 약함 — 사용자 직접 기각.
- **List each link as an error row** — offered; the user chose to skip and report the links instead. 약함 — 사용자 직접 기각.

## 근거
- User decision U1 (2026-11-11), quoted above; convention check by main (walkdir default, ripgrep vs codex `codex-rs/ext/skills/src/loader/discovery.rs` and the Claude Code CLI).
- Review: child TRD review round 1 must 5 extended the rule from the scan to every path into the catalog.
- Code: asset-library `store.rs` (link policy in the module header; `LibraryStore::first_link`; `ScanListing::skipped_links`), `lib.rs` `PATH_THROUGH_LINK` and `RefreshReport.skipped_links`, the launch overlay in `launch.rs` (a path through a link is never read).

## 영향 / 불변식
- Every new way into the catalog (9a edits, 9b moves and deletes, relink) must pass the same admission: `admits` and `first_link` before any read or merge.
- Engram deliberately differs from the backends' skill loaders here: a skill linked into the library loads nowhere through Engram; the user copies it in (ADR-0312 decision 3).
- Folder entries (ADR-0341) are outside this rule: they are not library assets, and their listing lists a link without entering it.
