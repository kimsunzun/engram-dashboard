# ADR-0196: A dead daemon's record is stale regardless of version direction — check liveness before comparing versions

- 상태: 확정 (2026-09-13, basis: measured in the GUI + user decision)
- 관련: `crates/engram-dashboard-discovery/src/lib.rs` `check_acceptable` · ADR-0134(the client does not delete daemon.json) · ADR-0021(ensure = attach-only) · **ADR-0178**(the daemon decides a version mismatch at the handshake and cuts the connection — ★the spot where that ADR pointed at discovery as 「둘째 판정 주체 의심, 미검」 (“suspected second decision maker, unverified”) is the target of this decision★) · step-log S21

## 맥락

Right after the protocol was raised from 4 to 5, the app **could not attach to the daemon at all.** The leftover `daemon.json` had been left by a protocol-4 daemon that was **already dead**, but the decision function **compared versions before checking that the pid is alive**, so it came out as `VersionMismatch`, and the startup path treated that value as a hard failure meaning 「살아 있는 남의 데몬을 spawn 으로 덮지 않는다」 (“do not overwrite a live daemon of someone else with a spawn”) — ★the comment on that branch itself says it is for a *live* daemon★. The screen had no recovery path (the control surface has no explicit connect command), so deleting the file by hand was the only recovery.

There are two recurrence conditions: **every release that raises the protocol**, and **every time you switch between branches whose protocols differ**.

## 결정

`check_acceptable` looks at **pid liveness first** and looks at the version after that. And that decision **deliberately does not look at the version direction** — a dead record is stale whether it is lower or higher than ours, so we overwrite it with our daemon. A version mismatch of a live daemon is a hard failure as before (that protection does not change).

In addition, when the liveness recheck finds that the candidate is **alive but of a different version**, it returns `VersionMismatch` instead of `Timeout` — this puts the diagnosis that the old order gave at once back in the same place under the new order.

## 거부한 대안

- **Keep the status quo (a dead record is also a hard failure)** — rejected by measurement. Right after the protocol went up the app could not connect, and the only means of recovery was deleting the file by hand. The reason that propped up the hard failure (「살아 있는 데몬을 덮지 않는다」 (“do not overwrite a live daemon”)) **does not hold for a dead record** — there is no process to protect and no work in progress.
- **Add a version-direction decision (back off if the record is newer than ours)** — rejected by user decision. A dead record has nothing to protect, and telling 「네 빌드가 낡았다」 (“your build is out of date”) **as a connection failure** is a channel the user cannot act on (that wording is `데몬=6, 기대=5` — to tell about staleness it belongs on the update-check side). ★What we gave up in exchange★ = when two builds share one data folder, 「먼저 띄운 쪽이 폴더를 차지한다」 (“whichever launched first holds the folder”) becomes two-directional (with a direction decision, the new build would always win). Because branch switching during development happens in both directions, we chose 「지금 띄운 것이 돈다」 (“whatever is launched now runs”).
- **The client deletes the stale file** — a direction ADR-0134 already rejected, so we do not revive it. The reasons it does not pay off (polling already filters out dead pids) and the damage (deletion in front of the gate · network-shared portable folders) are canonically there. This decision achieves the same goal by changing **only the decision order**, without deletion.

## 근거

- **Measured in the GUI** — we planted a protocol-4 file with a dead pid and checked twice. The second was the production shape: after actually killing the daemon, we reverted that daemon's own file to 4, and the connection succeeded and the new daemon rewrote the file as 5.
- **Regression net** — the twin tests dead pid × (old version · new version) **both** go red if the order is reverted (payload `daemon=4`·`daemon=6`). That pair fixes the symmetry: just as the old order did not look at direction in *rejection*, the new order does not look at direction in *acceptance*. The live daemon + version mismatch test stays green untouched, so it testifies that the protection is kept.
- **Finding raised in review** — we received the finding that changing only the order makes the remote daemon always read as dead in a **network-shared configuration** (a local `OpenProcess` decision — a fact that file already records), so the diagnosis is smeared into `Timeout`; so we put in the `VersionMismatch` branch above together.

## 영향 / 불변식

- ★**The decision order is load-bearing**★ — in `check_acceptable`, the liveness check must come before the version comparison, and **do not "fix" it by adding a direction decision of 「더 새 기록이면 물러난다」 (“back off if the record is newer”).** The twin tests are what keep those two.
- **Protection of a live daemon is unchanged** — live pid + version mismatch = hard failure. The existing tests hold that spot.
- **Duty to recover the diagnosis** — when the liveness recheck meets live+mismatch, it carries the version instead of `Timeout` (one body with ADR-0134 decision 4's 「원인 없는 시간 초과를 대신한다」 (“replace a timeout with no cause”)).
- A warning log is attached at the decision point (pid · file version · expected version — ★token fields are not included★).

### Known residuals (not created by this decision, and not closed by it either)

- ★**An old record with `start_time` of `0` is open to pid reuse**★ — `is_dead` decides liveness by the pid alone when `start_time == 0` (the `#[serde(default)]` in `crates/engram-dashboard-protocol/src/discovery.rs`). If a record from the days before that field is still around and its pid has been reused by an **unrelated live process**, it reads as alive and goes to the version-mismatch hard failure — the very symptom we measured comes back through another door. It is narrow and pre-existing, and it cannot be closed here without reopening ADR-0134's 「지우지 않는다」 (“do not delete”).
- **In a network-shared configuration the diagnosis does not arrive at once** — the `VersionMismatch` branch above does return the version value, but that path is reached **only after spawn + the polling deadline are used up** (on the way, one local daemon that will fail is launched). The same cost was already being paid in the version-match case (the existing recovery test is that spot), and the mismatch case joined it there.

### Relation to ADR-0178 — this decision is a provisional placement

★**ADR-0178 decision 3 is 「클라 쪽 버전 검사를 두지 않는다」 (“do not put a client-side version check”), and this decision fixes only the order while *keeping* that check**★. We leave the measurements so that the next session does not dig into the overlap of the two decisions again.

- **The daemon-side rejection is real** — `crates/engram-dashboard-net/src/ws.rs:273` sends a reason frame and cuts the connection when the client version differs from the expected one (that behavior approved by ADR-0178 decision 2).
- ★**There is no client side that tells the user about that rejection**★ — the popup of ADR-0178 decision 5 (「새로 만들 부분은 이것뿐」 (“the only part to build new”)) is unimplemented (measured 2026-09-13: 0 hits searching non-test code in `src/`·`src-tauri/src`).
- So **what actually blocks this path today is discovery's client-side decision**, and to remove it, ADR-0178 decision 5 must stand first. If it is removed before that, a mismatch comes out as **a connection drop with no guidance**.
- **So there is one reopen trigger** — when ADR-0178 decision 5 (the rejection-reason popup) is implemented, reopen whether to **remove** discovery's version check of a live daemon. This decision is the placement until then.
