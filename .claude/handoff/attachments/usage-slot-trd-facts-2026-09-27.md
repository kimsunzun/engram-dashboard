# Usage-limit slot — TRD fact sheet (read-only survey, 2026-09-27)

Repo: `I:\Engram\apps\engram-dashboard-wt2` (branch `v0.3.2/feat/usage-limit-slot`). Paths are repo-relative.

---

## 1. Passive sources today

### Claude (`crates/engram-dashboard-agent/src/backend/claude/mod.rs`)
- Decoder = `ClaudeStreamDecoder` (`:653`), state = partial-line byte buffer + `discarding` resync flag. `decode()` `:688`, `flush()` `:726`, `consume_line()` `:744`.
- Doc at `:743`: "`system`/`rate_limit_event`/그 외 unknown type → skip(0개)". Skip arm at `:834`: `// system/init·rate_limit_event·thinking_tokens 등 메타 라인, unknown type → skip.  _ => {}`.
- Decoded types: `assistant`/`user` → per-block `consume_block` (`:839`); `result` → `OutputEvent::Usage` (if non-zero tokens) then optional `Error` then `MessageDone`.
- Fixture shape (real capture): `backend/claude/fixtures/claude_text.jsonl:3`
  `{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":1782567000,"rateLimitType":"five_hour","overageStatus":"rejected","overageDisabledReason":"org_level_disabled","isUsingOverage":false},"uuid":…,"session_id":…}`
  Note: no utilization/percent field in the captured sample — only status + resetsAt + type. Test at `:2757` feeds `{"type":"rate_limit_event","rate_limit_info":{}}` and expects skip.
- Only JSON mode (`-p --input-format stream-json --output-format stream-json --verbose`, `:169-182`) passes through the decoder. Terminal (PTY) claude has no decoder → no passive source there.

### Codex (`crates/engram-dashboard-agent/src/backend/codex/decoder.rs`)
- `KNOWN_UNTRANSLATED_METHODS` `:133` lists `"account/rateLimits/updated"` (`:135`) — classed `Observed::Routine` (debug-log only, counted), not translated.
- Dispatch `translate()` `:443-481`: handled = `ITEM_AGENT_MESSAGE_DELTA`, `ITEM_STARTED`, `THREAD_TOKEN_USAGE_UPDATED`→`usage()`, `ERROR`, `TURN_COMPLETED`, `ITEM_COMPLETED`, `DEPRECATION_NOTICE`; `other =>` observe + `Vec::new()` (`:469`).
- `consume_line()` `:809` → `protocol::classify` → `Inbound::Notification` → `translate`. Requests/responses must not reach the decoder (transport splits envelopes first).
- Module header `:13-18`: "모르는 것은 버린다 — `Structured` 를 배출구로 쓰지 않는다" (Structured escape hatch would render codex method names/JSON on screen).
- `protocol.rs` has **no** rateLimits constant/type (grep 0 hits).
- Decoder is moved into the pump as `Box<dyn OutputDecoder>` (header `:7-11`) — no side door; anything it produces must be an `OutputEvent`.
- Only codex StreamJson mode = app-server (`backend/codex/mod.rs:812-815`); codex Terminal mode = TUI over PTY, no app-server, no passive source.

### Where decoded things go
- `OutputEvent` enum `crates/engram-dashboard-agent/src/types.rs:37` — variants: `TerminalBytes`, `TextDelta`, `ToolCall`, `Usage{input_tokens,output_tokens,turn_id}`, `MessageDone`, `TurnEnd`, `Error(String)`, `Structured{kind,json}`. No rate-limit/account variant.
- Flow: decoder → `OutputCore::emit` (`output_core.rs:203`, inner `:310`) → replay ring + seq + fan-out to `OutputSink`s (`types.rs:872`) → daemon maps via `output_event_to_wire` (`crates/engram-dashboard-daemon/src/connection_core.rs:737`) to wire `StructuredEvent` (`crates/engram-dashboard-protocol/src/messages.rs:628`). **Per-agent, per-subscriber, replayed** — it is chat output, not global state.

### Existing non-output paths from core/backend
- `StatusSink` trait `types.rs:878-900`: `status_changed`, `agent_list_updated`, `restore_result` (default no-op), `turn_ended` (default no-op, ADR-0113). Daemon impls: `crates/engram-dashboard-daemon/src/status_fanout.rs:47` (`DaemonStatusSink`) wrapped by `messaging_host.rs:862` (`MessagingFlushSink`, decorator that must forward every hook — comment `:883-888`).
- Backend-owned observers outside the output stream: `AgentBackend::session_id_source()` (`backend/mod.rs:619`, runner = `session_tracker.rs`, claude impl `backend/claude/session_file.rs:128`); codex `SessionIdSink`/`LinkSink` ports held by the transport (`backend/codex/transport.rs:546-549`); capabilities via `AgentBackend::capabilities()` (`backend/mod.rs:375`) composed with transport caps (ADR-0030).
- **No existing path carries account-level/global (non-agent) data out of a backend.** Nearest seams: add a `StatusSink` hook (like `turn_ended`, default no-op) or a new injected port like `SessionIdSink`.

## 2. Codex app-server client
- Transport: `crates/engram-dashboard-agent/src/backend/codex/transport.rs` (`CodexAppServerTransport`, header `:1-40`). Owns child, single stdin writer thread, reader(pump), `pending: Arc<Pending>` (`:543`) + `next_id: Arc<AtomicI64>` (`:545`).
- Blocking request/await: `request_blocking<P,R>()` `:742-797` — registers `Waiter::Handshake(mpsc::Sender)`, writes line, `recv_timeout` loop with deadline; used only on the writer thread during handshake (`initialize` `:833` → `initialized` `:852` → `thread/start`/`thread/resume` `:865`/`:877`).
- `Waiter` enum `:426`: `Handshake(tx)` / `TurnStart{seq}` / `Fire` (log failures only). Responses matched in reader at `:2087` (`self.pending.take(id)`).
- Non-handshake requests go via the outbox (`s.outbox.push_back(line)`), e.g. `interrupt()` `:2540-2565` (`Waiter::Fire`, `OUTBOX_LIMIT`). There is **no generic "send request X and return its result" API** — `Fire` discards the result, `Handshake` is writer-thread-only.
- Process model: one `codex app-server --stdio` child per agent incarnation (handshake "화신마다 정확히 한 번" `:804`); `AgentTransport` trait (`crates/engram-dashboard-agent/src/transport/mod.rs:53-90`) exposes only `start/send_input/flush_input/resize/interrupt/shutdown/capabilities` — nothing for arbitrary RPC. So `account/rateLimits/read` cannot be sent through a live conversation from outside without adding a new transport/port surface.
- **No code launches app-server without a conversation** (only spawn site `transport.rs:598` via agent spawn; `APP_SERVER_SUBCOMMAND` `backend/codex/mod.rs:461`, used `:813`).

## 3. Daemon → client broadcast of non-agent state
- Wire enum `AgentEvent` (`crates/engram-dashboard-protocol/src/messages.rs:362`). Broadcast variants: `AgentListUpdated` `:444`, `InputLeaseChanged` `:459`, `ProfileListUpdated` `:465`, `PresetListUpdated` `:477` (+ request/reply twins `AgentList`/`ProfileList`/`PresetList` with `request_id`).
- Emission: `connection_core.rs:1966` `broadcast_profile_list`, `:1976` `broadcast_preset_list` (called after preset CRUD `:1509/1518/1528`), `:1935` `broadcast_lease_changed`; all go `event_json(&ev)` → `fanout.broadcast_text(text)` (`FrameFanout`, `crates/engram-dashboard-net/src/frame_port.rs:79`, impl `net/src/ws.rs:210`). `RosterFanout` `:1949-1963` shows the port pattern for "something changed → rebroadcast".
- Late joiners: no push-on-connect; client pulls (`ListPresets` etc.) at boot and on reconnect (`src/store/eventBus.ts` `resyncAfterReconnect`, ~`:180-195`).
- Shell relay: `src-tauri/src/daemon_client/connection.rs:1249` `AgentEvent::PresetListUpdated{presets} => events.preset_list_updated(presets)` → `src-tauri/src/daemon_client/events.rs:90` (port trait) / `:138` `self.0.emit("preset-list-updated", presets)`.
- Frontend: `src/api/tauriTransport.ts:237` `listen('preset-list-updated')` → `ProtocolClient` dispatch `src/api/protocolClient.ts:697` → `onPresetListUpdated` `:1014` → `src/store/eventBus.ts:176` → `useAgentStore.setPresets`.
- UI settings/layout are **shell-owned** (src-tauri), not daemon: layout via `layout:updated` Tauri emit; theme via `ui.refresh` (`src-tauri/src/layout/commands.rs:355`).
- ts-rs: `#[derive(TS)] #[ts(export)]` on wire types (e.g. `messages.rs:13-14`, `:361`); output dirs `crates/engram-dashboard-protocol/bindings/`, `crates/engram-dashboard-agent/bindings/`, `src-tauri/bindings/` (default ts-rs `bindings/`, no `export_to` override). Generated by `cargo test` (`export_bindings_*`); CI sync gate diffs all three dirs (CLAUDE.md 「CI」). Frontend imports e.g. `src/api/layoutTypes.ts:5-7` from `../../src-tauri/bindings/…`.

## 4. Command bus
- Macro `declare_commands!` `crates/engram-dashboard-command/src/macros.rs:80` (doc `:9-78`): generates args/ok structs (serde+ts-rs), `CommandSpec` via `inventory`, JSON Schema, `CATALOG_VERSION`. One block per module; handlers injected by `make_table(deps)`; `#[effect(Read|Write)] #[since(N)]`; closed type alphabet (primitives, `Vec`, `Option`, `Option<Option<T>>`, block-local struct/enum; enums = unit-variant string enums, no serde rename).
- Producers: daemon-side `crates/engram-dashboard-agent/src/commands.rs:30` (catalog_version 4; `agent.list` `:95`, `agent.spawn`, `agent.new`, `agent.rename`, `agent.move`), table `make_table` `:441`, installed in daemon `crates/engram-dashboard-daemon/src/control/commands.rs:20,426`. Shell-side `src-tauri/src/layout/commands.rs:73` (catalog_version 8), `make_table` `:383` (plug one handler per name), installed `src-tauri/src/lib.rs:133`; shell registers its table with the daemon via `AgentCommand::RegisterCommands` (`src-tauri/src/daemon_client/connection.rs:1719`). Daemon routes by name to owner (`crates/engram-dashboard-daemon/src/command_delivery.rs`; 3-step route `crates/engram-dashboard-command/src/route.rs`).
- Frontend registry commands (`src/commands/registry.ts`, `register({id,title,run,help?})`) — the `window.__engramCmd` / cdp / keybinding / menu surface. Only commands with a `help` field are offered to the bus (`src/commands/viewCommandBridge.ts:40-58` `offeredCommands`, reported via invoke `:136`).
- LLM text help: `prompts/engram-help.md` (loaded at runtime by `crates/engram-dashboard-daemon/src/bin/engram.rs:207`, embedded fallback `:231`). ★Exactly five sections `root·mail·agent·window·theme` (header `:8`) — a new section needs a binary change; slot-content help lives in `## window` (`:94`).
- `catalog_version` bump rule: bump whenever the declaration changes (names, arg types, **enum vocabulary**, reply shape) — `src-tauri/src/layout/commands.rs:58-72` ("이름이 늘 때만 올리는 번호가 아니다 — 선언이 바뀌면 올린다"). Diagnostic only; receivers must not reject on it.
- End-to-end example `layout.setSlotContent`:
  1. Declaration `src-tauri/src/layout/commands.rs:302-309` (`content: SlotContentKind, agent_id: Option<String>`).
  2. Handler `verb_set_slot_content` `:760-777` → `slot_content()` `:921-940` (kind+agent_id → `SlotContent`, rejects mismatches) → `apply::set_slot_content` `src-tauri/src/layout/apply.rs:449` → `ViewManager::set_slot_content` `src-tauri/src/layout/manager.rs:569` → `layout:updated` emit.
  3. LLM CLI: `engram layout.setSlotContent --view_id … --slot_id … --content PresetPalette` (help `prompts/engram-help.md:94`).
  4. Frontend human path: `src/commands/tabCommands.ts:176-199` registry command `layout.setSlotContent` → `useViewStore.setSlotContent` (`src/store/viewStore.ts:213-219`) → `invoke('set_slot_content')` — converges on the same `apply.rs` service.

## 5. Slot content
- `SlotContent` `src-tauri/src/layout/types.rs:33-46`: `#[serde(tag="type", rename_all="snake_case")] #[ts(export)]`; variants `Empty`, `Agent{agent_id}`, `AgentList`, `PresetPalette` (last two unit, "데이터는 … 여기 담지 않고" `:41-45`). `agent_id()` exhaustive match `:53-57`. Other exhaustive/semantic matches: `layout/manager.rs:799` (spawn-slot occupancy treats `PresetPalette` as occupied), tests `tree.rs:581-608`, `manager.rs:1364`.
- `SlotContentKind` `src-tauri/src/layout/commands.rs:106-111` (unit enum for the bus); mapping `slot_content()` `:921-940`. Adding a variant = new `SlotContentKind` word + bump `catalog_version` (8 → 9).
- Frontend: `SLOT_CONTENT_TYPES` `src/commands/tabCommands.ts:147` `['empty','agent','agent_list','preset_palette']`; `validateSlotContent` `:156-172` (per-variant field check; unit variants pass).
- Render switch `src/components/layout/LayoutLeaf.tsx:206-207` (`isPresetPalette`/`isAgentList` → `hasContent`), JSX `:283-289` (`preset_palette` → `<PresetPalette />`, `agent_list` → `<AgentList />`). Focus exclusion for control slots mentioned `:79`.
- Fill commands + menu: `src/commands/slotContentCommands.ts` — `slot.fill.agentList` `:29`, `slot.fill.presetPalette` `:40-48`; `registerSlotMenu('empty', [{title: t('slot.newContent'), children:[…fill…]}])` `:94-105`; `'*'` contributions with `hideOn: ['agent_list','preset_palette']` `:110-117`.
- Context menu: `LayoutLeaf.tsx:302-307` `<SlotContextMenu items={buildSlotMenu(node.content.type)} ctx={{viewId, slotId, agentId}}>`; content-specific items via `registerSlotMenu('<type>', …)` (examples `src/commands/presetCommands.ts:84`, `agentCommands.ts:190`), common via `'*'` (`slotCommands.ts:138`); descriptor supports `group/order/children/hideOn` (ADR-0064/0065). A new type needing no menu still gets `'*'` items unless it's added to `hideOn` lists.
- Per-slot options: **only `Agent{agent_id}` carries fields today.** Bus args for setSlotContent have a single extra field `agent_id` (rejected for non-Agent kinds `:933-935`). A variant with fields would need new optional args (or a new command) + TS validator case. The other per-slot state precedent is `renderModeOverride[slotId]` — frontend-only, per-webview, deliberately not on the bus (`src/commands/renderModeCommands.ts:1-20`, ADR-0167).
- `PresetPalette` (`src/components/slot/PresetPalette.tsx`): reads `useAgentStore(s => s.presets)` `:33`; mutations via `agentClient` (`:94`, `:132`), no optimistic update — display follows `PresetListUpdated` broadcast. Its row menu is a local `position:'fixed'` popup (`:320`).

## 6. Frontend subscription to backend-owned state
- Two lanes (CLAUDE.md 「프론트 구조」): `eventBus` (`src/store/eventBus.ts`) takes abstract subscriptions from `agentClient` (`src/api/agentClient.ts:183/188` `onProfileListUpdated`/`onPresetListUpdated`); backend-authoritative shell surfaces (layout, tabs, window layout, UI settings) use Tauri `listen` directly with self-owned disposers (find via `rg "from '@tauri-apps/api/event'" src/`).
- Closest pattern for "global daemon-pushed state rendered in a slot" = **presets**: daemon `broadcast_preset_list` → shell `events.preset_list_updated` → Tauri emit `preset-list-updated` → `TauriTransport` listen → `ProtocolClient` → `agentClient.onPresetListUpdated` → `eventBus` → `useAgentStore.setPresets` → `PresetPalette` selector. Plus pull `ListPresets`/`PresetList{request_id}` for boot and reconnect.

## 7. Dependencies
- `Cargo.lock`: `reqwest` 0.12.28 (`:3237`) and 0.13.4 (`:3269`), `hyper` (`:1817`), `hyper-util` (`:1838`), `tokio-tungstenite` 0.26.2 (`:4577`), `tungstenite` (`:4882`). **No TLS crate at all** — `rustls`, `native-tls`, `schannel`, `hyper-rustls`, `hyper-tls`, `tokio-native-tls` absent. reqwest dependency lists contain no TLS deps.
- `reqwest 0.12` = daemon **dev-dependency** only (`crates/engram-dashboard-daemon/Cargo.toml:193`, `[dev-dependencies]` starts `:159`; `default-features = false, features=["json"]`, comment "로컬 평문 http"). `reqwest 0.13` comes via `rmcp` (daemon dev-dep `:195`) and **`tauri` 2.11.3** (normal dep of src-tauri) — `cargo tree -i reqwest@0.13.4`.
- WS stacks: `net` `tokio-tungstenite` optional, "rustls 등 TLS 불필요(로컬 평문 WS)" (`crates/engram-dashboard-net/Cargo.toml:102-103`); `transport`, `src-tauri`, `discovery` (blocking `tungstenite`) all plain.
- **No outbound internet call** in daemon/agent/src-tauri production code: only localhost `TcpStream::connect` (`crates/engram-dashboard-daemon/src/bin/engram.rs:2628`, `crates/engram-dashboard-discovery/src/lib.rs:842`). An HTTPS usage endpoint call would introduce the first TLS stack and first internet egress.

## 8. Claude process launching
- `ClaudeBackend::build_spec` `backend/claude/mod.rs:130`: always `--permission-mode bypassPermissions` (`:155-156`); Terminal: `--session-id|--resume <sid>` (`:160-167`); JSON: `-p --input-format stream-json --output-format stream-json --verbose` + `--session-id|--resume` (`:169-195`); then extra_args. Windows wrapping via `console_command` (`backend/mod.rs:44`, `cmd.exe /c`). Spawned by `transport/stdio.rs:79` (`Command::new(&spec.program)`).
- No `control_request`/`control_response`/`get_usage` anywhere in `crates/` (grep 0). Claude JSON stdin today carries only user turns.
- Short-lived claude not tied to an agent: **none in production.** Only dev bin `crates/engram-dashboard-daemon/src/bin/saturation_pilot.rs:1645-1656` runs `claude --version` via `std::process::Command`. All other spawns go through `AgentManager`.
- `CLAUDE_CONFIG_DIR`: `backend/claude/mod.rs:988-1000` `config_dir()` = `CLAUDE_CONFIG_DIR` (non-empty) else `~/.claude`; used for `projects/` transcripts and `sessions/`. `.credentials.json`: **no reference anywhere** in crates/src-tauri/src/scripts (grep 0). Credential file not opened.

## 9. Theme / i18n / popups
- Tokens `src/styles/theme.css:1-3` (per `:root[data-theme='dark'|'light'|'e-ink']`): `--status-running` (green: dark `#3fb950`, light `#1a7f37`, e-ink `var(--text-muted)`), `--status-blocked` (amber: dark `#d29922`, light `#9a6700`, e-ink `var(--text)`). **No red/error/danger token exists.** e-ink neutralizes color (glyph shape only, ADR-0062/0173). Tailwind maps `--color-*` → vars in `src/index.css:10-16`. Use pattern: `statusGlyphColor()` in `src/components/agent/AgentList.tsx` (test `AgentList.test.tsx:170-205`: vars only, no literals).
- i18n: `src/i18n/index.ts` `t(key, params?)` (ADR-0069), keys typed from `src/i18n/ko.ts` (`as const`, namespaces by UI domain `ko.ts:10`, e.g. `preset:` `:116`, `slot.*`).
- Fixed popups: `src/components/slot/SlotContextMenu.tsx` (`position:'fixed'` `:121`, flyout `:224`, helpers `clampMenuPosition` `:33`, `flyoutPosition` `:59`, `ANCHOR_GAP` `:27`); `src/components/slot/AgentMonitoringPicker.tsx:103` (fixed backdrop modal, "SlotContextMenu 의 fixed 오버레이 패턴과 동형" `:15`); `PresetPalette.tsx:320` row menu.

## 10. Relevant ADRs (`docs/decisions/README.md`)
- 0002 출력 seam = OutputEvent (`:52`) · 0030 capability = transport ⊕ backend 합성 (`:80`)
- 0004 AgentTransport seam + backend 지식 격리 (`:54`) · 0045 출력 정제를 백엔드로 (`:95`)
- 0028 백엔드가 이벤트버스 소유 — 단일 push 채널 (`:78`) · 0113 턴 상태 관측 공용 승격 (`:163`)
- 0035 레이아웃 권위 = src-tauri (`:85`, partially superseded by 0057/0169) · 0057 탭 소유 모델 (`:107`)
- 0055 command registry 구현 방향 (`:105`, part. by 0169) · 0064 슬롯 컨텍스트 메뉴 단일 기여 API (`:114`) · 0065 hideOn/children (`:115`) · 0155 통합 command 버스 (`:205`) · 0151 crate 분리 기준 (`:201`) · 0022 (`:72`) · 0081 (`:131`)
- 0060 슬롯 콘텐츠 = 타입드 유니온 SlotContent (`:110`) · 0061 프리셋 영속 데몬 소유 (`:111`) · 0063 set_slot_content 제네릭 command (`:113`) · 0011 agentClient 제어 표면 (`:61`)
- 0069 UI 문자열 중앙화 (`:119`) · 0167 창별 테마 (`:217`) · 0168 프레임 시각 정리 (`:218`)
- 0203 codex 화면 복원은 app-server 에 페이지로 요청 (`:252`) · 0212 engram help 외부 파일 (`:261`, part. by 0220)
- 0227 분할 렌더러 평평한 칸 목록 + 비율 명령 (`:276`) · 0068 LLM 공간 타깃 (`:118`)
- Rate limits / usage: **no ADR** (grep "rate limit|usage|사용량|한도" → 0 hits in index).
