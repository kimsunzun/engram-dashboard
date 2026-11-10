# TRD — S36 connection band (frontend)

- Status: edition 1 · 2026-11-10 · canonical ADRs = ADR-0350 (bands, one stop path), ADR-0356 (version band only informs, 「다시 연결」 on both), ADR-0345, ADR-0287 · PRD = `prd.md`

## 1. Scope and PRD relation

Implements PRD R1–R9. The shell side (band rules, `daemon_shutdown`, stop path) is merged on master and is not touched, except removing the dead `daemon_stop` command (R9). Material = parked commit `4d25dfa7` (diff `093b0eff..4d25dfa7`): copy the pieces named below, never whole files.

## 2. What master has today

- The band reaches the frontend but stops at the transport: `TauriTransport.connectionBand` getter (`src/api/tauriTransport.ts`), parsed from the shell DTO; a band-only change already notifies `onConnectionStateChange` subscribers through `ProtocolClient.notifyState`. `Transport`, `WsTransport`, `ProtocolClient` and `AgentClient` have no band member.
- `ConnectionNotice` reads only `agentClient.connectionError`; it is mounted in the main window (`AppLayout`, inside `NoticeOverlay`) and in popouts / the spare (`PopoutPage`, no props).
- `DaemonControl.start()` = `daemon_start` → `client.connect()` → clear `connectionError`; on error sets it and rethrows.
- `daemon_shutdown` is registered; no frontend wrapper. `daemon_stop` is registered with no caller.
- No command is gated to the main window yet.

## 3. Design

### 3-1. Band plumbing (frontend API)

- Add `readonly connectionBand: Band | null` to `Transport` (`transport.ts`), `WsTransport` (always `null` — the dev WS path has no shell), `ProtocolClient` (delegates to the transport) and `AgentClient`. `Band` = `src-tauri/bindings/Band.ts`. Parked hunks for this member only.
- No new event: subscribers keep using `onConnectionStateChange` and read `connectionState`, `connectionBand`, `connectionError`.
- **Cause guard (R21, parked `ProtocolClient` hunk):** `refusesCause(band)` = band ② or ① `refused`. The transport listener clears `connError` when such a band arrives; `reportConnectionError` writes `null` while one holds. Without it the normal ② recovery shows a stale cause: a version-mismatch boot records 「protocol 버전 불일치 …」 from the failed `daemon_start`, ② hides it, and after the tray stop the plain ① shows it (review 2026-11-10, M1). The same holds for a token refusal followed by a stop.
- Not taken from the parked API diff: `attach()` (R15), `refreshStatus` / `refreshConnectionStatus` (C10), the `TauriTransport` listener single-flight (C14).

### 3-2. Band model — `src/api/connectionBand.ts` (new, from parked)

```ts
// 제안
export type BandKind = 'not_connected' | 'version'
export interface BandView { kind: BandKind; cause: string | null }
export function bandView(band: Band | null, connectionError: string | null): BandView | null
export function bandText(view: BandView): string // head + (' ' + cause)?
```

- ① `not_connected`: cause = `refusedCause` when `refused`, else `connectionError` (or `null`).
- ② `version`: cause = `versionCause`; `connectionError` is ignored; `can_stop` is not read.
- `null` band → `null` view (no band) regardless of `connectionError` (PRD R3).

### 3-3. Texts — `src/i18n/ko.ts` (`connection` group)

`head` 「데몬에 연결하지 못했습니다.」 · `refusedCause` 「데몬이 이 앱의 인증을 거절했습니다」 · `versionCause` 「실행 중인 데몬이 이 앱과 버전이 다릅니다. 데몬을 끈 뒤 다시 연결하세요」 · `reconnect` 「다시 연결」 · `reconnecting` 「연결 중…」 · `dismiss` (✕ aria-label, today's 「알림 닫기」) · command titles `statusCmd` 「연결 상태」, `reconnectCmd` 「다시 연결」, `stopCmd` 「데몬 끄기」. Parked stop / confirm / `reconnectUseStop` keys are not taken.

### 3-4. `ConnectionNotice` (rewrite on master's file, parked skeleton)

- Props: `actions?: boolean` — `AppLayout` passes `actions`; `PopoutPage` passes nothing (text only, PRD R6).
- State read on every `onConnectionStateChange`: `connectionState`, `connectionBand`, `connectionError` → `bandView`.
- ✕ memory: `dismissed` = key `kind|cause`; hidden while the current key equals it; cleared when `connectionState === 'connected'` (replaces today's "cleared when the reason becomes null").
- Button (only with `actions`, on both kinds): 「다시 연결」 → `busy = true` → `getDaemonControl().start()` → `.catch(() => {})` (the error already lands in `connectionError`; the band follows the shell) → `.finally(() => busy = false)`; while busy the button reads 「연결 중…」 and is disabled.
- Keep the layer contract of master's comment (covers the canvas top, `wrap-anywhere`, ✕ load-bearing, not always-on chrome). Rewrite the "not a new control surface / sends no command" paragraph: the button calls the existing start; the LLM path is `connection.*` (§3-6).
- `data-band={kind}` on the root for tests and QA.

### 3-5. `DaemonControl` (`src/api/daemonControl.ts`)

- `start()`: first line `this.client.reportConnectionError(null)` (PRD R7 — a new attempt clears the old cause); the rest unchanged (single try/catch).
- New `shutdownDaemon()`: `invoke<DaemonShutdownReply>('daemon_shutdown')` → returns the reply. No `disconnect()`, no band write (ADR-0350 U6).

### 3-6. LLM commands — `src/commands/connectionCommands.ts` (new, parked structure)

- Registered only when `readWindowLabelFromHash() === MAIN_WINDOW_LABEL` (module-level gate); `import './connectionCommands'` in `contributions.ts`. No `help`.
- `connection.status` → `{ state, band, text }` (`text` = `bandText(view)` or `null`).
- `connection.reconnect` → `await start()`; success → the `connection.status` reply; failure → rethrow the start error (its message is the cause the band shows). Without the parked re-pull (C10) the band in a reply right after a failure can be one event behind, so a failure answers with the error rather than a status. On success the state is `connected`, so the reply is exact.
- `daemon.stop` → `shutdownDaemon()` reply `{ outcome, reason }`.
- Dependency injection as parked (`registerConnectionCommands({ client, start, shutdown })`) so tests use fakes.

### 3-7. Remove `daemon_stop` (Rust)

Parked Rust hunks apply as-is: the command in `src-tauri/src/commands/discovery.rs` and its `generate_handler!` entry in `lib.rs`; in `src-tauri/src/discovery/mod.rs` `ProcessKiller`, `daemon_stop`, `stop_with`, `TaskKiller`; in `discovery/tests.rs` `CountingKiller` and the three `stop_*` tests. Fix the comments that name `daemon_stop` (`daemonControl.ts`, `wsTransport.test.ts`, `commands/discovery.rs`, `discovery/mod.rs`). `platform::process::kill_tree` loses its last caller but stays (public, other platform code may want it; removing it is not this topic).

## 4. Tests plan

- **Unit (vitest):**
  - `connectionBand.test.ts` (new): ① plain / refused / with `connectionError`; ② ignores `connectionError`, new text; `null` band with a `connectionError` → no view.
  - `ConnectionNotice.test.tsx` (rewrite): band-driven render per kind; button on both kinds only with `actions`; busy label and disabled during `start()`; button returns after a rejected `start()`; ✕ key (same key hidden, different key shown, cleared on `connected`); popout (no `actions`) shows text, no button.
  - `connectionCommands.test.ts` (new, trimmed from parked): status shape in each state; reconnect success → status, failure → rethrow; `daemon.stop` passes the reply through; not registered outside the main window.
  - `daemonControl.test.ts`: clear at the start of `start()`; `shutdownDaemon` invokes `daemon_shutdown`.
  - `protocolClient.test.ts`: `connectionBand` delegates to the transport; a cause is cleared when ② or ① `refused` arrives and not recorded while one holds; a plain ① keeps it.
  - Fakes gain `connectionBand`: `protocolClient.test.ts`, `slotDaemonRestart.test.tsx`, `usageStore.test.ts`, `AppLayout.test.tsx`, `PopoutPage.test.tsx` (their alert cases move from `connectionError` to a band).
- **Rust:** `cargo test -p engram-dashboard --test lib_unit` after the removal (the three `stop_*` tests go with the code).
- **QA (`/engram-qa`):** full regression per the qa binding (frontend `npm test` + `npx tsc --noEmit`, backend lines, gates). GUI on a release build — PRD AC1–AC7 scenes: kill the daemon → ① → 「다시 연결」; `v0.3.2` daemon → ② → button keeps ② → tray stop → ① → reconnect (if the `v0.3.2` build is unavailable, say so — unit tests cover ②'s text and button); token rewrite in an isolated data dir → ① refused; start failure → ① with cause; ✕; popout text only; the three LLM commands through `__engramCmd`.

## 5. Units

One coder, sequential; each unit ends with its tests and `tsc` green.

| Unit | Files |
|---|---|
| **U1** Rust | `commands/discovery.rs`, `lib.rs`, `discovery/mod.rs`, `discovery/tests.rs`, comment fixes |
| **U2** API + model | `transport.ts`, `wsTransport.ts`, `protocolClient.ts`, `agentClient.ts`, `daemonControl.ts`, `connectionBand.ts`, `ko.ts`, fake updates, their tests |
| **U3** UI + commands | `ConnectionNotice.tsx`, `AppLayout.tsx`, `connectionCommands.ts`, `contributions.ts`, their tests, `PopoutPage.test.tsx` |

## 6. Decisions

Main:
- Rebuild on master; parked hunks only for the pieces in §3 (parked predates ADR-0352 and ADR-0356).
- `connection.reconnect` failure = rethrow (§3-6); PRD R8 wording follows.
- `can_stop` stays on the wire, unread (ADR-0356 영향).
- `kill_tree` kept (§3-7).
- No clearing of `connectionError` on a stop result (PRD does not need it; it clears on `connected` and on the next start).

User: see PRD §5; plus (2026-11-10, PRD+TRD review hold H1) boot start stays in every window — ADR-0350 decisions 5 (R17) and 2's R15 do not land here; decided in the multi-daemon topic (T-85). User: 「나중에 다중에이전트 선택할때 결정하는게 좋을듯? 일단은 이대로 유지하자」.

## 7. Risks

- Copying parked files whole would bring back the stop button, the confirm and the guards — the coder brief lists the pieces to take.
- `WsTransport` returns `null` for the band only to keep the interface complete — no production code builds it (`clientFactory` always builds `TauriTransport`, ADR-0029).
- Boot: the shell's boot hold (R20) keeps a never-connected handle bandless until the main window's start answers; nothing here changes it.

## 8. Review record

- (pending) PRD+TRD review.
