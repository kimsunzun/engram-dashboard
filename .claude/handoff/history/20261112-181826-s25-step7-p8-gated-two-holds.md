# Handoff: S25 step 7 — P5·P6·P7 landed; P8 review+QA PASS but UNCOMMITTED (two user holds); next = answer holds → commit P8 → P9

## STATE AT SAVE — read first
- **P8 is gate-passed but uncommitted** (게이트 통과 · 미커밋 — not untrusted): review full 3 rounds PASS, QA PASS on tree `070071b3` (= working tree `crates` + `Cargo.lock`). Commit is blocked only by two `hold`s waiting on the user (below). The tree object `070071b3` exists in `.git/objects` (written via a temp index) — recovery: `GIT_INDEX_FILE=/tmp/x git read-tree 070071b3 && GIT_INDEX_FILE=/tmp/x git checkout-index -a -f --prefix=<dir>/`.
- **Two holds asked, not yet answered** (asked in chat 2026-11-12; ask again):
  1. Library root unusable (no root / first scan failed / root stat not Ok): today each attached library preset ALSO gets ① `not-found` (with a relink offer) beside block ④. Options: A ★ show only ④ (one cause; like rustc suppressing cascade errors; adds one branch) / B keep both (no change). Either way the launch is blocked.
  2. A built-in file (`builtin\presets/default.yaml` etc.) that reads but does not parse → Broken → blocks EVERY agent launch (default_preset is on by default). Options: A ★ treat like a missing built-in — empty stand-in + warn (matches B15 "built-in missing never blocks"; merges a branch) / B keep blocking.
  If A is chosen for either, change `resolve.rs` / `builtin.rs` (P8 coder brief style), re-review the delta (lead + cross-family), re-run QA (full regression), then commit.
- P8 commit message draft: write from `.review-s25-d9/p8-coder-report.md` + `p8-fix-r1.md` + this file's verification lines (pattern: `.review-s25-d9/p7-commit-msg.txt`).

## Know first
- `src-tauri/bindings/*.ts` (~45) line-ending noise — never stage.
- The user's review criterion (2026-11-12): a finding that is only theoretically reachable is fixed only if the fix adds no new conditions/states/branches; otherwise record it. 「조건이 많아지는것도 버그가 끼어들 틈이 있어서.」 Applied in P6–P8 (and stated in coder briefs and re-ask prompts). Recorded in Engram `7246f13` (engram-implement feedback, 2nd occurrence).
- The user wants continuous progress without asking at unit boundaries: 「이어서 진행해. 나한테 물어보는 이유는? 뭔가 끊긴건 아니잖아.」 — announce and proceed; ask only real decisions (holds, user-visible policy).
- Before raising a user-visible policy, check conventions (the user asked 「관행을 한번 살펴봐」 for the subfolder case).
- `gate.rs` was written with CRLF line endings (other files LF); git normalizes — no action taken.
- Codex threads left in `~/.codex` by QA: `01a1cdcb-946b-7840-8bfb-55391e62d481` (P5 codex lane) plus older ones in earlier handoffs — deleting is the user's.
- Local backup branches still present (user's call to delete): `v0.5.0/wip/s25-p4-final-pending`, `v0.5.0/wip/s25-p4-schema-validate-parked`.

## One-line state · next first action
Step 7 = new crate `engram-dashboard-asset-library` in 9 units: P1–P7 committed (P5 `4d920305`, P6 `72effa1d`, P7 `0525a5b5`), P8 gated-uncommitted. **Next: ask the two holds → apply (if A) → re-review/QA → commit P8 → P9 launch (deep) → step-7 doc pass.** Step 7 is *Stop (after)*.

## Repo state (branch v0.5.0/feat/preset-library · worktree wt1)
- HEAD `0525a5b5` (+ this handoff commit); upstream 0/0 before this commit; vs origin/master 0 behind / 201 ahead.
- Uncommitted (P8): `crates/engram-dashboard-agent/src/launch.rs`, asset-library `src/backends.rs`, `src/builtin.rs` (new, staged A), `src/catalog.rs`, `src/format/read.rs`, `src/lib.rs`, `src/resolve.rs` (new, staged A).
- CI: green through `0525a5b5` (P7).
- This session's commits: `4d920305` P5, `f93a7816` T-105 note, `72effa1d` P6, `0525a5b5` P7. Engram: `7246f13` feedback.
- `.review-s25-d9/` (git-excluded) holds every brief, TRD extract, diff, coder report, fix list, re-review sheet, QA brief and commit message for P5–P8 — reuse the pattern.

## Verification state
- P5: review deep 3 rounds PASS; QA PASS (workspace 6010, codex lane 11/11 incl. q4/q11/q12 at 120 s — `a55195b4` confirmed), mutation 5/6 named-test (listed_at real-FS test survives its mutation; covered by two others; rename pending — doc pass).
- P6: review deep 3 + 2 rounds (skip change) PASS; QA PASS (6053; folder tests 6x incl. under load; mutation 11/11).
- P7: review deep 2 rounds PASS; QA PASS (6096; gate/service tests 6x incl. full-CPU load; mutation 8/8).
- P8: review full 3 rounds PASS; QA PASS on `070071b3` (6131; frontend 2807; gates 33/33; flake 4/4; mutation 8/8; lanes N/A — paths and CLI versions unchanged). **Not committed (holds).**
- Rerun: `cargo test --workspace --locked -- --test-threads=4`; lanes per `.claude/skill-bindings/qa.md`.

## Failed approaches (do-not)
- Do not stop a turn after saying 「진행하겠습니다」 without starting — the user called it out.
- Do not recycle `%TEMP%/detached-cmd-*.bat` you did not create (another session's runner may use them); QA runs now put `$QA_TMP` under the session scratchpad.
- A QA run started on a tree that will still change is wasted — stop it (TaskStop) before the change, then clean its runners/worktrees.
- Do not let the risk-axis lens run more than 2 rounds per target (P6 skip change went lead + cross-family only).

## Decisions
User (2026-11-12):
- 라이브러리 읽기 상한 · 상황: P5 hold — over-cap read kept stale forever; cap value is P7's · 선택지 error row ★ / keep · 고른 것 「일단 상한은 그냥 높이는게 좋을 것같은데? 아니면 해당 모델 제조사 앱 룰에 맡기는게 좋지 않음?」 → main: cap is Engram's daemon memory guard, not the CLI's; proposed 16 MiB + keep handling · 「ㅇㅇ 그렇게 가고 나중 얘기이긴한데 이런것도 설정으로 다 빠져야됨.」 · 갈림 (★ was error row) → 16 MiB in P7 (`LIBRARY_READ_CAP`), T-105 widened (`f93a7816`).
- 리뷰 고칠 범위 · 상황: P5 round 1 fixed many theoretical findings · 선택지 fix if few lines ★ / reachable only · 고른 것 「이론상 가능한것 수정하느라 코드가 복잡해지고 개발이 크면 빼는게 맞고, 딱히 문제없으면 넣는게 좋다고 생각. 조건이 많아지는것도 버그가 끼어들 틈이 있어서.」 · 갈림 (new criterion) → applied P6–P8.
- 읽을 수 없는 하위 폴더 · 상황: P6 hold, TRD silent; ADR-0341 blocks an unreadable folder entry · 선택지 skip ★(first) → block ★(revised after lead noted ADR-0341 + new state) → skip ★(after survey) · 고른 것 「읽을 수 없는 하위 폴더는 관행을 한번 살펴봐.」 → survey (codex skill discovery, ripgrep, find, git, walkdir all skip+warn) → 「ㅇㅇ 저희는 건너띄고 하자.」 + 「ㅇㅇ 건너띄되 어떤게 건너띄웠는데 알림은 해줘야됨.」 · 같음 (final ★ skip) → `FolderListing.skipped` + notice `folder-listing-skipped` (≤3 paths + 「외 n개」), top unreadable still blocks.
- 단위 경계 진행 · (묻지 않음) · 「이어서 진행해. 나한테 물어보는 이유는? 뭔가 끊긴건 아니잖아.」 → proceed without asking.
Main (2026-11-12): P5 deep (dependency + seq semantics); P6/P7 deep, P8 full; `LIST_CAP` = agent `FOLDER_LISTING_CAP`; `ListAnswer.folders` shape for subfolder rows (parent TRD requires them); `list` → `Result` (ADR-0324 decision 6); unparsable attached path → ② `item-path-invalid` (parent §1-3 8); `path-through-link` via a post-pass `first_link` probe (R11, no new catalog state); `Problem = BlockReason`; withdrawn-after-re-ask musts recorded (P6 guard order, P6 lookahead error, P8 root race).
Open: the two P8 holds above.

## Doc pass owed at the end of step 7 (add to the previous lists — history `20261111-122129-*`, `20261112-*` P4 handoff)
- TRD decision rows: library read cap 16 MiB; unreadable-subfolder skip + `folder-listing-skipped`; P8 holds' outcomes; `ListAnswer.folders`; `list` → `Result`; `item-path-invalid` for attached paths; `path-through-link` in read entrances.
- TRD text fixes: §8-1 P6 aged-joiner clause (follow §3-8); "spawn failed" has no OS text; §8-1 P7 "ticketed change during a walk rescans" vs §6 FIFO; §3-7/§3-11 shapes (`State.root: Option<PathBuf>`, `Problem = BlockReason`, `ItemView` owned file / no `at`, `Builtins::new(dir, backends)`, `LibraryValues` without io_limit/launch_waits, `RefreshReport.errors` `Option<LibPath>`); ADR-0324 decision 7 "fold" vs §6 ⑤.
- Recorded-not-fixed list (each unit's commit message has it): P5 listed_at test rename; P6 advisory failure answers stored, spawn-failure read → Other (P9 decides), Instant overflow (T-105), lookahead error; P7 safety net walks unknown folder subtree / no subfolder lines, `changed` counts racy re-reads; P8 root race, `mark_links` probes when root not Ok, quadratic skill collection, `ListRow.state` preset line problems, unreadable-file ② in read entrances.
- step-log entries for P5–P8 and this session's decisions.

## Improvements
- Recorded (Engram `7246f13`): theoretical-findings criterion for engram-implement §3 (2nd occurrence).
- Report only: QA workers asked the binding to say which lines take `--locked` and how to scope the root `tests::` module (`--exact` list) — qa binding wording.

## Resume command
`git -C I:/Engram/apps/engram-dashboard-wt1 log --oneline -3 && git -C I:/Engram/apps/engram-dashboard-wt1 status --short | grep -v bindings/`
Next: ask hold 1 then hold 2; then commit P8; then `/engram-implement` P9 (child TRD §3-8 launch, §8-1 P9 row, §9 P9 row; deep; P9 emits `folder-listing-skipped` / `-partial`, carries resolve's omitted notice, decides the spawn-failure read mapping).

## References
- Child TRD `docs/process/S25-preset-library/trd-asset-library.md` (draft 12) — §3-8 (P9 launch), §8-1/§9 P9 rows, §12 hand-over.
- ADR-0324 · 0341 · 0359 · 0360 · 0357 · 0315.
- `.review-s25-d9/p8-*` (current), `p7-*` (patterns incl. QA brief and commit message).
