# ADR-0076: Activation = resume the existing session, Fresh issues a new sid (no reuse) — refines ADR-0008

- 상태: 확정 (2026-07-13, basis: reproduction of activating a reserved profile after a daemon cold boot — instant death with "Session ID <sid> is already in use") · 부분 폐기 by ADR-0185 (the premise that we issue the sid) · 부분 폐기 by ADR-0226 (honoring an explicit resume=true in the first item of the decision, the persistence mechanism for the new_session_id issuance in the second item, and the ensure_session_id invariant in the consequences)
- 관련: ADR-0008 (session restore, sid control) refined · `agent/manager.rs::spawn_agent`/`fallback_fresh` · `agent/profile.rs::new_session_id`/`ensure_session_id` · `daemon/connection_core.rs::SpawnProfile` · step-log · Amended by ADR-0077 (manual activation (activate_profile) also shares the same fresh-fallback as restore_one on an early resume exit) · Amended by ADR-0082 (the fallback_fresh-related invariants and the wording "fresh-fallback is valid" are discarded: activation=resume, Fresh=new sid, and the single sid-issuing point remain valid) · Amended by ADR-0185 (the premise that we issue the sid) · Amended by ADR-0226 (honoring an explicit resume=true in the first item of the decision, the persistence mechanism for the new_session_id issuance in the second item, and the ensure_session_id invariant in the consequences)

## 맥락
User decision: **"Activating an agent must continue the existing session. If it is going to load fresh, create a new agent."** Activation is conversation resume, and a Fresh start applies only to a truly new agent (no previous session).

Reproduced defect (daemon cold boot → activating a reserved profile): claude exited immediately with `Error: Session ID <sid> is already in use` (wire status `Exited, code:null`). Two defects overlapped.
1. **Wrong mode** — even when the profile had a previous `claude_session_id`, activation spawned with `SpawnMode::Fresh` (the frontend `spawnProfile(id, false)` → the daemon maps `resume=false → Fresh`). It started fresh in a situation that called for resuming.
2. **Fresh reused the old sid** — `spawn_agent` called `ensure_session_id` (returns the value as is if present) in both modes, so Fresh started with `--session-id <저장된 sid>`. The session file already existed on disk, so claude died instantly on the conflict. The fresh-fallback (ADR-0008) had the same reuse defect and risked a repeat conflict.

## 결정
- **Mode derivation (backend authority)** — on profile activation, the mode is decided by whether a session exists: `claude_session_id.is_some()` → **Resume** (`--resume <저장 sid>`, conversation resume), `None` → **Fresh** (new). The daemon `SpawnProfile` handler derives `mode = resume-요청 OR 세션-존재`, so a profile that has a session is always resumed regardless of the frontend wire `resume=false`. An explicit `resume=true` is honored as is.
- **Fresh always gets a new sid** — `spawn_agent` issues the sid per mode: Resume=`ensure_session_id` (the stored value as is), Fresh=`new_session_id` (**always a new uuid**, pushing the old sid into the `old_session_ids` history). `new_session_id` is introduced to enforce this contract, and `ensure_session_id` is narrowed in meaning to Resume only.
- **Single authority point for sid issuance** — issuance happens in `spawn_agent` only. `fallback_fresh` no longer plants a sid beforehand; it only bumps the epoch, then reads and reports the new sid that `spawn_agent(Fresh)` issued (double issuance removed).

## 거부한 대안
- **Always Fresh on activation** — the conversation disappears (a direct violation of the user decision: "must continue"). This is half of the cause of the reproduced bug.
- **Fresh reuses the stored sid (the current bug)** — `--session-id <기존 sid>` conflicts with the on-disk session and claude dies instantly ("already in use"). Fresh = a new conversation, so it must be a new sid.
- **Unconditionally re-derive the mode from session existence inside `spawn_agent`** — `fallback_fresh` plants a new sid and then deliberately calls with Fresh (it must be Fresh even if the session is `Some`). If `spawn_agent` looks only at existence and flips to Resume, the fallback breaks. So mode derivation belongs to the caller (the handler), and the sid issuance rule is separated into `spawn_agent`.

## 근거
Instant death with "Session ID already in use" was reproduced by activating a reserved profile after a daemon cold boot (daemon-debug.log), and we confirmed from code and logs that the cause is (1) mode=Fresh and (2) the sid reuse in `ensure_session_id`. With backend authority (session exists → Resume) plus a single issuance point, "no matter which caller, as long as the mode is right, sid conflicts are sealed off at the source". It reuses ADR-0008's controlled-sid resume infrastructure as is, and the fresh-fallback also has no repeat conflict thanks to the new sid.

## 영향 / 불변식
- **ADR-0008 is refined (not discarded)** — "we control the sid → `--resume` restores losslessly" and "early resume exit → fresh fallback → terminal Failed" remain valid. This ADR clarifies and corrects, on top of that, "activation=resume · Fresh=new sid (no reuse) · single issuance point".
- **`ensure_session_id` = Resume only** — calling it on the Fresh path reuses the old sid and regresses to "already in use". Fresh uses `new_session_id` only.
- **sid issuance happens in `spawn_agent` only** — if another path such as `fallback_fresh` plants a sid directly, double issuance tangles the causality (the old sid is pushed into the history right away).
- Anchors: `// ADR-0076` (profile.rs `new_session_id`, manager.rs the sid branch of `spawn_agent` and `fallback_fresh`, connection_core.rs `SpawnProfile`).
