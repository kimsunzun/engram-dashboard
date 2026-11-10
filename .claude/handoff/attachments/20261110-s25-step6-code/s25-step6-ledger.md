# S25 step 6 code — running ledger (main)

## Pending user decisions (commit held until decided)
- H1 (P1, lead reviewer): near-verbatim mirrors of codex code (Apache-2.0). Round 1: extract_front_matter / repair_front_matter / pty_append_quoted (MIT) rewritten by coder — lead accepted as no longer near-verbatim (repair keeps codex's decision order). Round 2: new `is_structured_feature_path` (codex/settings.rs ~:445) is a character-level copy of codex merge.rs; insert_at_path/merge_values follow codex arm by arm. Same class two rounds in a row -> stop case fixes, ask the user how to treat the class (rewrite all mirrors / keep with attribution pointer / ask legal). No license judgment by main (org rule).
- H2 (P1, lead reviewer): two Noop behaviour changes in the safe direction — (a) space-less priming holding & ^ | < > is now skipped with a warning (was injected unquoted into cmd); (b) control URL holding " or metachars is now refused (precheck -> failed spawn). Neither input occurs today. Accept as a deliberate Noop exception named in the commit body, or move out.
- H3 (P2, lead reviewer): main-added real-terminal cwd guard refuses % & ^ | < > strictly; a path with a space is quoted on the line, so & ^ | < > would be safe there (only % must stay refused). Strict (claude precedent claude/launch.rs:79) vs space-aware like priming_value. User-visible: e.g. `D:\R&D Projects` fails every real terminal launch under strict.
- H4 (P2, lead reviewer, convention-challenge on U3): notice wording `$<이름>으로` gives `$review으로` for vowel-ending names; user's exact wording -> user decides (e.g. `(으)로`).

## Tracking / later-step candidates (not fixed)
- Noop terminal budget is not an upper bound after M10a (shim line +148, resolved cmd.exe path 27 vs 7, pass-through " and \ doubling) — window ~7.8k-char priming; latent (priming 1.3 KB). (P1 risk reviewer)
- Non-npm codex shims (pnpm / npm<=6 / yarn v1) put %* inside IF ( ) blocks: an unquoted ) in a -c line could break the block. From memory, unverified. (P1 risk reviewer)
- Noop terminal cwd not checked for cmd metachars (pre-existing; `C:\Work\R&D` -> runs `D`). Real path now guarded (P2). (P1 risk reviewer)
- JSON merged instructions text has no size cap (Noop priming read unbounded too). Cap before/at P3 or composition. (P2 risk reviewer)
- Step-7 hand-over: check side must judge the canonicalized cwd with path_is_cli_safe (else SpawnFailed every launch, never Broken). (P2 lead + risk)
- plan.include_priming=true with instructions_file=None launches with no priming and no warning (relies on prepare). (P2 risk)
- R10 pass-through extra_args unguarded on the real line — recorded decision (convention-challenge, not re-raised).

## Doc pass (TRD §8-1 + additions)
- ADR candidates (a)-(e) per TRD §8-1; plus: real-terminal cwd refusal (next to (b)); ADR-0337 decision 3 says the notice states "intended behaviour" — U3 wording does not -> amend; TRD §4-3 priming rule amended (space-less text refuses CMD_UNSAFE chars); `-` leading -c line refused; structured feature merge mirrored.
- Research file M10 added by P1.

## Restore points
see restore-points.txt

## P2 status
- P2 review deep: round 1 FIX (codex must: TEMP-dependent test) -> round 2 PASS (lead + risk). Batched closed by main (cwd exception module doc, two Noop-only notes in build_spec, link-layout doc narrowed).
- Doc pass additions: TRD §4-4/§4-5 step 4/§7-1 P2 `instructions_changed` now also requires text; codex/mod.rs ~:1028 "priming_file not read on real materials" true for JSON only after P3 (P3 coder told to keep it true).
- Release contract (step 7/8): must detach every directory link under the volatile folder regardless of depth (constants are pub(crate)).

## P3 status
- P3 review deep round 1: PASS x3 (no must). Fix round (tests only): foreign/mismatched echo rows, interpretation (b) pin, live L5 current_dir, live cleanup + drop guard — fresh coder (P3 coder hit 500k).
- H5 (P3, lead, hold): new screen text CONTEXT_DROPPED_NOTE 「바뀐 지침을 알리는 내용이 두 번 거절돼 이번 실행에서는 더 보내지 않는다」 uses 「지침」; user-approved notices use 「지시서」. Wording = user decision. Also hard-codes 「두 번」 (bound constant; a write failure counts as Rejected).
- Convention-challenge (P3 risk): retry bound counts only Rejected (decision #16); a carrying turn accepted then `turn/completed{failed}` before its echo re-sends the context every turn without limit (halts until next user message each time). Recorded decision -> report to user, not a hold.
- Tracking: delivery confirmed by turn window, not clientId — a server-originated userMessage in our thread during a carrying turn would report true wrongly (no known codex feature; unverified). Hardening for floor-Above: match clientId.
- Tracking: whether codex emits a live userMessage notification during a resume handshake (excludeTurns) — would report false every resume and re-send each launch; unverified.
- Batched by main: FirstTurnSink doc ("delivery not confirmed"), thread_open doc dedupe, instructions_text doc gate; transport.rs gate-doc dedupe (:326 :1442 :1711) pending until the fix coder finishes.
- Tracking (P3 fix round): a mismatched-turnId echo on our thread (e.g. a late echo of an earlier turn after the carrying turn got its id) consumes the one-shot FirstTurnSink with false (witness_first_turn checks thread only); the context is still cleared by the matching echo, so the next launch re-sends it (accepted-duplicate direction).
- Next: QA rerun from the start (full regression -> claude lane -> codex lane incl. codex_launch_live -> mutation -> GUI). Open 권장: print the L1 thread id before the asserts (codex_launch_live.rs ~:587). Batched: ci.yml gate-6 header language.

## QA rerun 1 (tree 385023b9) — FAIL at codex lane L4
- Full regression PASS (5737 Rust, 2762 frontend, tsc, gates 34/34). claude lane PASS (lanes.tsv claude line rewritten). codex: usage_probe_smoke 2, backend_contract 11 PASS; live L1/L2/L3/L5 PASS; L4 FAIL — real ~/.codex shows the codex update modal (0.163.0 -> 0.167.0; version.json rewritten by codex at 2026-11-09T21:54Z), test assumed none. Mutation + GUI not run (stop rule).
- Codex thread left in the user's real ~/.codex: 01a1c24d-f65a-75a0-8b6b-1cd27dcaefe4 (also the earlier step-6 measurement thread). Deletion = user's call.
- User-facing note: the GUI codex terminal agent on this PC will also show the update modal until the user skips or updates codex (updating changes the codex lane version).
- Fix in progress: L4 kit raw line `check_for_update_on_startup=false` (test only).

## QA rerun 2 (tree 225a93f0) — PASS
- Full regression PASS (5737 Rust / 2762 frontend / tsc / gates 34/34). claude lane PASS. codex lane PASS (usage_probe 2, backend_contract 11, codex_launch_live L1-L5 all PASS, 28 s). Mutation PASS (cmdline odd-quote; transport echo clear). GUI PASS via CDP only (Windows session locked — no screenshots): JSON agent turn + resume (resume_instructions=true, M8), terminal agent up after Esc on the update modal (version.json unchanged).
- lanes.tsv claude + codex rewritten to 7d1e0caa +uncommitted.
- Threads left in real ~/.codex: 01a1c264-a0cb-70c3-8b1d-88d843ea9c05 (L1/L2), 01a1c26f-b831-7262-aef4-f057b9b33710 (GUI JSON agent), 01a1c263-8c2a-73b2-80d4-2ca50e941d76 + 01a1c24c-d8a3-7733-bd24-78e44d4d2b11 (usage_probe_smoke codex_ — existing lane, one per run), 01a1c24d-f65a-75a0-8b6b-1cd27dcaefe4 (rerun 1 L1). Deletion = user's call.
- Tracking: L5 leaves its temp folder every run (os error 32 — a codex child still holds it at drop); main recycled both leftovers.
- Commit still held: holds H1-H5 pending the user.

## User decisions 2026-11-10 (morning)
- H1 mirrors · 선택지 A 다시 짜기 ★ / B 그대로 + 출처 표기 · 고른 것 「일단 다 만들어놓았으면 그냥 그대로 가던지」 · 갈림 · 조건: 출처 주석(Apache-2.0) 한 줄 — 표기 충분성은 사용자가 담당 부서 확인.
- (묻지 않음) 범위 원칙 · 고른 것 「일단 어지간한건 그냥 그쪽 클로드나 코덱스 룰로 가고 우리는 최소한만 막자 이것저것 다 하려고 하지 말고.」 → CLAUDE.md 설계 원칙 한 줄(「ㅇㅇ 한줄 언헝」) — commit 9969ac60.
- Applied from that principle (presented, no objection): H2 Noop priming/URL defense kept as is; H3 cwd guard space-aware (% always, & ^ | < > only without a space); H4 `$<이름>(으)로`; H5 「지시서」 + no hard-coded count; no JSON size cap; retry rule unchanged.
- Fix in progress (coder): H3/H4/H5 + attribution comments. Then delta review + QA rerun (codex lane again).
- Tracking (cwd guard review): launch.rs repeats & ^ | < > as a literal instead of platform CMD_UNSAFE_CHARS (a future T-84 '!' would not reach it); no separate '>' row.

## QA rerun 3 (HEAD 9969ac60, tree f9cb17db) — FAIL (claude lane flake)
- Full regression PASS (external target clean hit messaging + net --all-features once; rerun after files stable, PASS). codex lane PASS (L1-L5; thread 01a1c367-b0d7-75f1-9caf-01b81b9f98b3). lanes.tsv codex -> 9969ac60 +uncommitted.
- claude lane FAIL: usage_probe_smoke `claude_get_usage_answers_with_the_scratch_root_inside_the_repo` — ProbeFailure Timeout, claude printed nothing in 29 s. Baseline 7d1e0caa copy: pass. Diagnostic rerun on the working tree: pass. => intermittent (ADR-0038), no label. Load from other worktree cold builds / disk cleanup at the time (unverified).
- Not run: mutation, GUI (stop rule).
- Measured (no tokens): single % in --cd reaches codex.exe argv intact through cmd /c + npm codex.cmd shim, with and without a space; %USERNAME% expands (why >=2 % is refused).

## QA rerun 4 (tree f9cb17db, HEAD 9969ac60) — PASS
- User asked to rerun. Full regression + codex lane reused from rerun 3 (tree identical, bindings unchanged). claude lane fresh PASS (control_send 5, usage_probe_smoke claude_ 5 incl. the earlier timeout test). Mutation 2/2 (cwd guard `%` count < 2 -> < 1; cmdline odd quote). GUI PASS (session unlocked): codex JSON turn + resume (resume_instructions=true), terminal agent up after Esc; version.json unchanged. New thread 01a1c3a0-d86f-7831-b260-e0d20109f5a5 (GUI JSON agent).
- Commits: 090ed679 P1 · a3891273 P2 · abee6ebe P3 (tree f9cb17db = QA tree). ADRs 0353/0354/0355 (28b2bcf7, b6223f08, dc977a4d, bodies 45e06271).
- Doc pass open items for main: ADR-0337 decision 3 amendment (notice wording no longer says "intended behaviour"); check ADR-0216 impact of the Noop control-URL refusal; optionally add the single-% measurement to the research file.
