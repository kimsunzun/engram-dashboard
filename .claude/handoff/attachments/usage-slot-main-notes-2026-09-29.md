# Main orchestrator notes (session 2026-09-29) — survive compaction

## Commits this session (branch v0.3.2/feat/usage-limit-slot, NOT pushed)
731c6d4 merge master · 7aad8f7 5a · 4fd426a 5a fix · 56776d6 5b · c68b885 5b fix · 1ee3b29 6a · 547f83f 6a fix · 00e6683 6b1 · 31ca403 6b2 · 0561249 6b1 fix · (6c + 6b2-nits pending)

## Step-8 TRD amendments to record (docs/process/S21-usage-limit-slot/trd.md)
- §1-7 pending-shrink: same-interest recompute keeps gen/deadline (not "새 세대를 뽑아") — 6a fix 547f83f
- §1-7 창 소멸: hide marks removed only via forget_window on destroy (recompute no longer prunes)
- §1-7 링크 권한 (L196): grant lives in capabilities/usage-links.json (main + slot-popup-*), not default.json/popup.json
- §1-7 ⟳ (L210): loop does NOT exit after subscribe-write failure (next select sees disconnect); superseded socket (sync→None) sends refresh only
- §4 셸 row (L415): reword "반쪽만 나가는 경우 없음" → refresh-write failure = subscribe alone on wire + ⟳ fails
- §6 #9/§1-4: ⟳ Ack after REPLY_WAIT_MAX with in_flight is by design; ~2s blocking-pool margin accepted residual + const assert (5b fix)
- 5b: detail_code Option<i64> (TS bigint) — codex FIX vs Claude PASS disagreement → USER DECISION in final doc
- step-log: record step-5 done criterion (real daemon engram usage.get Claude/Codex OK, 2-3s) 
- CLAUDE.md numbers drift (59 result lines etc.) — user pending

## Step 7 forward notes
- menu toggle of show_* should send partial update → apply::set_usage_slot (not full set_slot_content replacement) to avoid clobbering LLM toggles
- SLOT_CONTENT_TYPES in src/commands/tabCommands.ts must accept 'usage'; prompts/engram-help.md list Usage
- opener URLs exact: https://claude.ai/settings/usage , https://chatgpt.com/codex/settings/usage
- interface: event "usage-limits-updated" {labels, socket_epoch, snapshot}; invoke get_usage_snapshot → {socket_epoch, snapshots}; ⟳ via forward_daemon_command {RefreshUsageLimits:{vendor, request_id}} → Ack; frontend discards smaller socket_epoch / lower revision

## User-pending decisions (final doc)
1. final overview doc review  2. QA binding fixes (3 + slot recipe + engram token recipe)  3. CLAUDE.md numbers  4. detail_code bigint vs number (macro-level fix = separate)
## Known/accepted
- tungstenite trace logs frame payloads at RUST_LOG=trace (pre-existing)
- 64 concurrent usage.* on hung probe can starve bus slots ≤5s (design note)
- UsageVendorRow derived Debug prints upstream (macro limitation; doc warning)
- Claude 5h window 96% used at test time (account data)
- 6c 871bc8f · 6b2 nits c1eb031

## Step 7 main decisions (report to user in final doc)
- split 7a data path (in progress) / 7b rendering (UsageSlot, usageFormat, theme, i18n, LayoutLeaf, slot.fill.usage) / 7c menu+LLM (slotMenu checked, usageCommands, tabCommands SLOT_CONTENT_TYPES, engram-help.md + daemon tests)
- broadcast label = getCurrentWindow().label
- toggle = full replace set_slot_content (TRD choice, accepted race) — no new partial Tauri command
- Unavailable wording: neutral, no cause asserted → flag for user review
- ⟳: follow TRD (popup + menu; small view opens popup); per-vendor ⟳ in popup, disabled for visibly Rejected vendor; menu ⟳ refreshes visible non-rejected vendors → flag PRD R12 "three places" deviation
- help spelling = real bus spelling
- ADR for SlotMenuItem.checked → step 8 /adr
- 6c codex FIX: tray hide marks before OS hide → fix: hide = mark after successful OS hide; show = mark before (pending doc-aware)
- 7a 088266e
- 6c fix 6539832 (parent 088266e)
- TRD #9 scenario amendment: move_slot_to_window phase A does not Defer (source slot stays until phase C, same lock) → zero frames
- 6c fix re-review: code PASS. Step-8 doc fixes: usage_interest.rs:46-47 (USAGE_INTEREST_SHRINK_DELAY doc) + :717 test doc wrongly cite "tab detach to other window" as the reason — real flows: tab switch away/back, show_* off/on, tray hide/show, close+re-place within 1.5s. TRD L202 + L255(#9) same.
- 7a codex FIX: startup pull terminal failure → no later pull (slot mount pull + broadcasts mitigate). pending doc-aware.
- 7a review: doc-aware PASS, codex FIX. Main decision: add usageStore.pull() to eventBus resyncAfterReconnect (idempotent; covers codex terminal-failure + doc-aware #2; TRD deviation → step-8 amendment + report). Also Rust serde key test for UsageLimitsUpdatedPayload {labels, socket_epoch, snapshot}. → give to 7c coder (after 7b, avoid src/ conflicts).
- 7b note: UsageSlot mount pull is mandatory (window gaining a slot later gets no broadcast).
- 7b 846a3e4 (strings: Unavailable 「이 계정의 한도 정보를 받을 수 없음」; stage-3 labels Cl/Cx; ⟳ not disabled while pending)
- 7b review FIX (both). Fix round AFTER 7c finishes (ko.ts overlap) via 7b coder a4d74079caa5ccb56: remove 1e-9 epsilon; popup focus after pos set (visibility:hidden blocks focus); height-aware stage fallback to 3 (R31 badge always visible) [flag]; reset-only windows show reset time; popup flip from rect.top; no bar for none (R21); aria-hidden small bars + short button label; Esc/focus-out only when focus inside; interpolate stale 30분; motion-safe spinner + e-ink static; code w/o upstream shown "(kind · code)" [flag]; Rejected minutes humanised via formatDuration [flag]; sentence joins into t() templates; NotInstalled upstream prefix neutral + no doubled vendor name in popup.
- 7c b59a7ac + 664d011 (menu: 사용량 새로고침 / Claude 표시 / Codex 표시; Command.when widened to args — ADR-0055 when-context touched → step-8 record)
- 7c review FIX (both). Plan (after 7b fix lands, via 7c coder a35e283d15128f56f): main decision = option (b): revert Command.when widening (registry/menu stop honoring when); add contribution-level `enabled?(ctx)` to SlotMenuItem (additive like `checked`, ADR-0064); LayoutLeaf subscribes to usage store and passes needed state (per-vendor blocksRefresh) into ctx → reactive (fixes stale F2); recheck on activation; role="menu" on menu root; help F4 wording (hidden rejection → Cached) in engram-help.md + agent commands.rs usage.refresh summary; F5 requireUsageContent boolean check; root help pointer mentions usage. ADR in step 8 covers checked+enabled.
- 7b fix a7946c0 (spinner kept spinning per prior user decisions agentGlyph.css 2026-08-24 / ADR-0226)
- 7c fix 626279c (when reverted; SlotMenuItem.enabled(ctx); origin:'slotMenu' arg flag; ctx.usageRefreshable) · 7b nits d49a93f
- 7c fix re-review: doc-aware PASS, codex FIX (origin spoofable) → drop origin (pending commit). Step-8 ADR (with /adr): ADR-0064 additive `checked` + `enabled(ctx)` (≠ Command.when; menu-shown state subscribed by assembler into ctx; usageRefreshable in generic ctx accepted for one consumer; reopen trigger = 2nd content type needing store state → generic per-content ctx provider). Also note ADR-0064 invariant "new item = command + registerSlotMenu only" bent by LayoutLeaf SlotMenu subscription.

## USER UI FEEDBACK (2026-09-29 evening) — collect all, then fix in one round
1. Vendor names "Claude"/"Codex" as text → replace with icons (cleaner). Codebase has NO vendor icons yet (grep) → add two small SVG marks.
2. Stage-1 row layout: columns not vertically aligned between Claude/Codex rows (name widths differ, % and countdown widths differ → '5시간'/'주간' bars start at different x). Use one grid across both vendor rows (icon | 5h label | bar | % | countdown | weekly label | bar | % | countdown), numbers right-aligned tabular-nums. Screenshot: images/1.png
3. Remove the ⚠ glyph next to <20% numbers (user: looks like an error/other-purpose marker; number already shows it). Revises PRD R10 (user decision 2026-09-29). Non-color cue remains via e-ink danger bar (black fill + thick border). Update tests that assert ⚠.
- step-8 docs committed: 257359c ADR 0246–0256 (+0064 link, index; written in full by sonnet worker from TRD §7 — needs /review doc) · eeb7258 TRD #90–#101 + 구현 marks · a7df21f step-log. Final HTML: docs/process/S21-usage-limit-slot/final-2026-09-29.html (untracked).
- Claude Code updated 2.1.283 → 2.1.284 (effective next session)
4. Reset display in small view: show reset clock instead of remaining duration — 5h '↻14:05', weekly '↻금 21:00' (weekday + time); remaining duration only in popup (already 「리셋 14:05 (2시간 25분 뒤)」). User OK in principle, wants to see a mock in the final doc first.
- USER APPROVED items 1–4 as in the preview mock (placeholder icons OK) + bar and % slightly closer. Implement now.
- USER: width/height 3-stage shrink must adapt correctly to the redesign (icons, grid, ↻ clock) — stage measurement uses the new markup; stage 2 aligned too; re-verify all stages (incl. height fallback) in /qa full GUI with screenshots.
- USER: a colour-preset system will come later (3 modes still needed). Keep usage colours layered: components → semantic --usage-* → base palette tokens (no literals). Sent to redesign coder: move literal reds + vendor icon colours to base tokens.
- Colour centralization survey (2026-09-29): usage slot colours all in src/styles/theme.css. App-wide NOT fully centralized: 15 non-test src files with hex/rgb literals outside styles (src/themes.ts 12 — likely xterm terminal palette; index.css 4; AgentList.tsx 4; structuredItems.css 3; TerminalSlot.tsx 3; richBranding.css 2; SlotContextMenu.tsx 2; AgentMonitoringPicker.tsx 2; …) + 11 Tailwind palette classes (text-red-500 style). → first step of the future colour-preset system = migrate these to tokens. Not in this branch's scope.
- REDESIGN COMMITTED 6598291 (items 1–4 + closer bar/% + stage adaptation + preset-ready tokens --status-danger/--vendor-claude/--vendor-codex). tsc ok, vitest 1505. Stage-1 natural width 411→366px, stage-2 height 98→74px (fallback needs ~82px). Stage 2 = icon spans its two window rows. Popup icon decorative. NOT yet: /review code, /qa full real-screen check, TRD/PRD record of R10 revision + reset clock + icons (user decisions).
- USER (2026-09-29): colour system = future work, NOT now. If effort is significant, build a full colour(-preset) system rather than a patch. Record only (handoff follow-up): inventory above (15 files + 11 Tailwind palette classes) → migrate to tokens, add a grep gate against new literals, add the principle to CLAUDE.md via /review doc.
- USER (end of session): "작은 화면 리프레시 · 슬롯 우클릭 「사용량 새로고침」은 내가 하라고 한 적 없다." → next session: check where PRD R12 (⟳ three places) and TRD §1-8 menu ⟳ came from (PRD history / handoff history 2026-09-27), show the user, and let the user decide ⟳ placement (possibly remove the menu item). Do not argue from the PRD text alone.
- USER: final doc did not show the POPUP LAYOUT → next session: add a popup layout section (screenshot g2 + labelled structure: header status line, per-window rows with absolute reset + remaining, age, plan, model-scoped windows, per-vendor ⟳, link) for user review, after the redesign (popup also changed: no ⚠, icon + name).

## USER DECISIONS — session 2 (2026-09-29 late) · ⟳ placement + toggles
- Provenance traced: all ⟳ placements first appear in docs/research/usage-limit-display-ui-survey-2026-09-26.md:13-22 "사용자 결정" (interview summary, no user utterance stored). Menu ⟳ absent from same-day handoff (history/20260927-014314 :18) → likely session addition. Only verbatim user quote: TRD §1-7 「체크온오프하고 동기화버튼 정도로 매칭」.
- Peer survey (light): docs/research/usage-manual-refresh-peers-2026-09-29.md (untracked, must be linked from PRD/TRD revision — no orphan). Global refresh is the norm; in-flight lock universal; time cooldown rare.
- DECIDED (user): small view gets ONE global ⟳ (Claude+Codex together), right of the grid, all 3 stages; hidden when both vendors off. Popup per-vendor ⟳ REMOVED. Right-click menu usage items ALL REMOVED (refresh + toggleClaude + toggleCodex) → only generic slot menu. Commands usageSlot.refresh/toggle* stay for LLM (not menu).
- DECIDED (user, option A): show/hide toggles move to the popup — bottom row 「슬롯에 표시: ☑ Claude ☑ Codex」 (scales as vendors grow; wraps). Popup detail shows ENABLED vendors only (off vendors aren't subscribed → no data; user: 「끄면 사용량도 안보내고 좋네」). Both off → small view hint changes 「우클릭해서…」→「클릭해서 표시할 항목 고르기」; popup then shows only the toggle row.
- Revises PRD R5 (popup ⟳), R8 (menu-only toggles, popup view-only), R12 (⟳ three places); TRD §1-7/§1-8; related ADRs (check 0246–0256 for menu/⟳). Record as 사용자 결정 2026-09-29.
- OPEN: (2) remove 30s manual-refresh guard (D11)? main recommends remove, keep in-flight lock + 429 block. (3) ↻ reset-time glyph collides with ⟳ refresh button — keep ↻? (button planned as bordered icon button).
- DECIDED (user): REMOVE the 30s manual-refresh guard (PRD D11 "직전 조회 뒤 30초 안의 ⟳ 는 조회하지 않고 들고 있는 값"). Keep: ignore presses while a fetch is in flight (until result arrives) + no fetch during 429 rejection (R24/D22 unchanged). User: 「다른데도 쿨타임 없으면 그대로 하자. 막히면 그네들이 알아서 적게 누르겠지」.
- DECIDED (user): reset-time glyph ↻ in small view → WORD. ko 「리셋 00:58」, en 「resets 00:58」 (Korean shorter). Popup already uses 「리셋 …」 → consistent.
- DECIDED (user, pending peer tally confirmation): weekly (>24h) reset shown as DATE not weekday — ko 「리셋 10/1 01:48」, en 「resets Oct 1 01:48」. Reason: official tools use date (Codex CLI `14:30 on 3 Oct`); weekday is ambiguous when reset is same weekday next week. Date-first then time (Korean order). Peer-format tally running (light research) — if it contradicts, report to user.
- IMPL (/implement standard, 2026-09-30): docs revised by worker (PRD/TRD #102 #103, ADR-0258 new, ADR-0252 stamped). Daemon coder removed REFRESH_MIN_SPACING (+ agent commands.rs summary, schema regen, prompts/engram-help.md:135). Frontend coder: global ⟳, popup ShowToggles, menu removal, reset 「리셋 M/D HH:MM」, dead usageRefreshable removed. /review code full round1 = FIX (3/3; toggle lost-update, stale help, popup anchor, comments, anchors) → fixed. Round2 codex = FIX: intentRef pending-selection approach still races (older echo after latest response clears intent; failure discards earlier success). Main leaning: replace with SERIALISE (ignore toggle clicks while a toggle write is outstanding until content changes / failure / timeout) — consistent with user's ⟳ rule. Loop counter 1, total re-fix 1.
- REVIEW ROUND 2 (2026-09-30): doc-aware = FIX low (registry.ts runAsHuman caller comment; help "served=Cached 뿐" misleading) and says toggle race CLOSED; codex = FIX high (race not closed under reordered echo / partial failure). DISPUTE on finding 1 → autonomous-mode conservative: adopt FIX. Main decision: make it moot — expose shell apply::set_usage_slot (lock-atomic per-field merge, apply.rs:479, previously bus-only) as a webview Tauri command; toggles send only their own field; delete intentRef + fireAndSettle. Also fixes pre-existing LLM-concurrent overwrite. Coder (FIX round 2) spawned. Total re-fix = 2 (cap 3). Docs to update after: TRD §1-8/#102 + ADR-0258 note new IPC command.
- REVIEW ROUND 3 (2026-09-30) — STOPPED HERE (usage limit). Uncommitted; all coder gates green (tsc 0 · vitest 1528 · shell lib_unit 401 · layout_commands 96 · layout_apply 65 · daemon 927 · fmt).
  - codex = FIX med: same checkbox double-clicked before echo sends same absolute value twice (2nd click lost). Fix option: lock-atomic FLIP in shell for toggles. Pre-existing: in-window layout.setSlotContent mergeUsageShows non-atomic (deferred, recorded TRD #102 ⑤).
  - doc-aware = FIX (docs): new IPC set_usage_slot contradicts ADR-0063 invariant 「슬롯 콘텐츠 변경은 set_slot_content 단일 경로」 + its rejected alternative 「variant별 전용 command」 → need `/adr link` stamp on ADR-0063 (Amended by ADR-0258) + ADR-0258 sentence why (partial merge can't ride full SlotContent: serde default true) — this overrides an ADR's rejected alternative → ASK USER. Also add `// ADR-0258` at src-tauri/src/commands/layout.rs:447 and src/commands/usageCommands.ts:8. Notes: route tabCommands mergeUsageShows via setUsageSlot now cheap; toggle racing content change can convert non-usage slot back (pre-existing; optional require_usage flag). Doc-aware says same-box double click = intended (WYSIWYG, ADR-0035) — DISAGREES with codex → user decision.
  - Re-fix count used: 2 of 3 cap. Next: user decides (same-box double-click: flip vs accept; ADR-0063 override OK?), then coder → review round 4 → /qa standard (+ GUI, popup screenshots for final doc) → /review doc → commit.
- USER DECISION (2026-09-30): popup checkbox same-box re-click before echo = keep current (absolute value, 2nd click effectively ignored). Cross-box overwrite already fixed via set_usage_slot.
- FOLLOW-UP TOPIC (user, 2026-09-30 — like colour system, NOT this branch): app-wide 「반영 전 재입력」 common handling. Research = docs/research/pre-echo-reinput-2026-09-30.md (medium, codex review applied). Hazards: tab + twice → 2 tabs, split/kill/spawn duplicates, Ctrl+Tab twice → 1 step, LLM spawn_into concurrent duplicates. Decide per-command policy (drop/queue/pass), guard at store/invoke layer, release on echo vs reply. Must be linked (no orphan) — record in handoff 후속 + step-log/tracking when branch docs are written.
- STILL PENDING for this branch: ADR-0063 conflict (new set_usage_slot IPC vs 「set_slot_content 단일 경로」 + rejected 「variant별 전용 command」) → ask user; then ADR anchors ×2 (layout.rs:447, usageCommands.ts:8), review round 4, /qa standard (+GUI, popup screenshots), final doc popup section, /review doc, commit.
- USER (2026-09-30): 「todo는 폐기해야겠네 동일한거잖아」 — docs/todo.md(색인 14줄 + docs/todo/*.md 7개) overlaps docs/tracking.md(T- 37개, 「나중에 다룰 것」의 정본 경로 — docs/README.md:40). Direction: merge todo INTO tracking, then delete todo.md + todo/ (per todo.md rule 3: confirm each item's destination before deleting). Also update docs/README.md:30 row, docs/handbook/documentation-system.md, any links (rg "todo.md|docs/todo/"). Separate small doc task — NOT started (context >57%); needs /review doc (load-bearing doc-system change).
- USER DECISION (2026-09-30): todo 폐기 — 「그냥 트래킹에 적립해두는 식으로」. From now on all deferred/not-done items go to docs/tracking.md (T-). Migration of existing docs/todo.md + docs/todo/*.md into tracking = task AFTER handoff (next session), with /review doc.
