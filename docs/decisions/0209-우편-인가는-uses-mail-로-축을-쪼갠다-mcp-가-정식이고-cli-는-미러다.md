# ADR-0209: Mail authorization splits its axis with uses_mail — MCP is the official path and the CLI is a mirror

- 상태: 확정 (2026-09-19, basis: commit `75bdfda` · 2 user decisions on 2026-09-18)
- 관련: CLAUDE.md 「백엔드 확장」 · ADR-0133(marker + single derivation point) · ADR-0132(control plane CLI) · ADR-0128(hard unification of the channel) · ADR-0004(the backend states its own facts) · `crates/engram-dashboard-agent/src/backend/mod.rs:451` · `crates/engram-dashboard-agent/src/backend/codex/mod.rs:240` · `crates/engram-dashboard-daemon/src/control/mod.rs:179` · step-log S21

## 맥락

To open a control plane entrance to codex, `supports_control_channel` has to be turned on (ADR-0208 — the assembly point skips provision on that slot). But the moment that slot was turned on, **the authorization to send mail was opened along with it.** Because the daemon derived whether mail is allowed **from a single axis**, `!accepts_mcp_config`.

The result is half-open — codex was opened for **sending only**, while `reads_messages=false` (terminal mode has no turn signal, so it cannot tell 「바쁜 때」 (busy time)). That asymmetry was **written in no declaration**, and the daemon records that it 「CLI 우편을 가르쳤다」 (taught the CLI mail), yet the actual spawn diverged into a state that received not one character of that teaching.

The user's judgment was that ★a half-open state is the worst★ (「나중에 하는 거 아니냐」, "isn't this something to do later", 2026-09-18) — we revert it now.

And on which side is the official path, the assistant **had it backwards and the user corrected it** (2026-09-18): ★for mail, MCP is the official path and the CLI is a mirror★. The basis was already in the code — the CLI header describes the `mail` family as 「MCP 툴 2종의 미러」 ("a mirror of the 2 MCP tools") (`crates/engram-dashboard-daemon/src/bin/engram.rs:2-3`), and ADR-0132's 「CLI 로만 낸다」 (emit only through the CLI) applies only to the `agent` (control) family, which has **no** paired MCP tool (same file `:3-4`). The existing derivation (`mail_allowed = !accepts_mcp_config`) was already that policy — **if there is MCP, MCP; if not, the CLI.**

## 결정

1. **Split the mail authorization axis with `uses_mail()` — the backend states it itself.** Which programs take part in the mail plane is a **per-program fact**, so the backend carries that declaration (ADR-0004). The daemon derives **one value** (`mail_allowed`) from this value and `accepts_mcp_config` — ★the single derivation point of ADR-0133 decision 2 stays as it is, and all that split is that the inputs grew from one to two★ (`crates/engram-dashboard-daemon/src/control/mod.rs:179` = `needs.uses_mail && !accepts_mcp_config`).

2. **codex is `uses_mail() -> false`** — closed **together with** the receive axis (`reads_messages`). What this slot closes is only the mail entrances (`/control/send`·`/control/messages`), and **the CLI entrance (token · address) and the control routes are opened as before by `supports_control_channel`** — `false` means 「우편을 못 쓴다」 (cannot use mail), not 「제어를 못 쓴다」 (cannot use control) (control verbs are open to everyone — ADR-0132).

3. **The official path for codex mail is attaching MCP to codex, and that is a separate chunk.** Opening CLI mail now would be mistaking the mirror for the official path.

4. **We nail down the order of opening — open the receive axis first.** If only sending is opened first, there will be senders who cannot receive a reply, and that remains in the mail ledger as a **permanent pending item**. The condition for opening codex's receive axis is written separately (splitting that axis by mode — this method does not take `command`, so it cannot tell the two modes apart, and terminal mode has no turn signal at all).

## 거부한 대안

- **Keep the single axis (`accepts_mcp_config`) and run both on it** — then codex stays **half-open** (send authorized · receive impossible, an asymmetry written in no declaration). ★The user judged that state the worst★ (2026-09-18). The rejection basis also remains in the code — the gate that makes the grant list emit not a single line when `uses_mail == false` is that spot (`crates/engram-dashboard-daemon/src/control/mod.rs:72-74`): without that gate, we get a state in which the daemon records 「발신을 허가했다」 (send was permitted) while a mail request with the same credential is rejected.
- **Open CLI mail to codex now** — the CLI is a **mirror and not the official path** (user correction 2026-09-18 · code basis = the two CLI header lines in 「맥락」 above). The official path is MCP, and codex's MCP injection is a global TOML override (`-c mcp_servers.<name>={…}`), so its **mechanism differs** from claude's `--mcp-config <path>` (measured) — that wiring is not in the scope of this stage.

## 근거

- **2 user decisions (2026-09-18)** — ① revert the CLI mail authorization that was opened as an attachment to codex (a half-open state is the worst) ② for mail, MCP is the official path and the CLI is a mirror (the user corrected the assistant's opposite-direction understanding).
- **Code basis** — 「MCP 툴 2종의 미러」 (a mirror of the 2 MCP tools) in the CLI header (`crates/engram-dashboard-daemon/src/bin/engram.rs:2-3`) · the next line of the same header saying that ADR-0132's CLI-only is limited to the `agent` family · the fact that the existing derivation was already 「MCP 있으면 MCP, 없으면 CLI」 (if there is MCP, MCP; if not, the CLI).
- **Canonical survey = `docs/research/codex-session-id-recovery-survey-2026-09-19.md`** (this axis came to light as a result of that survey making us open the codex control plane entrance — ADR-0208).
- **Commit = `75bdfda`.** Regression = workspace 54 binaries · 2465 passed · 0 failed. ★**claude is unchanged in every expression**★ (it receives the default `uses_mail() = true` as it is).
- ★**The gate degradation is shared with ADR-0208**★ — since it is the same commit, the 2nd review being a single family and the last fixes not being re-reviewed apply to this decision too.

## 영향 / 불변식

- **The default of `uses_mail` is `true` (fail-open)** — cutting mail because we do not know would make letters silently vanish, so **only a backend that is outside the mail plane declares false itself** (`crates/engram-dashboard-agent/src/backend/mod.rs:451`).
- ★**Residual debt: `shell` does not declare `uses_mail`**★ — it has `reads_messages=false` (because input is **executed** as a command — a different classification reason from codex), but the default is fail-open, so it immediately violates the discipline this ADR set up (「두 축이 갈린 행은 사유를 그 backend 폴더에 적는다」 (a row where the two axes diverge writes its reason in that backend's folder)). **The only reason it is harmless today is `supports_control_channel=false`** (`crates/engram-dashboard-agent/src/backend/shell/mod.rs:29-31`), and that is **exactly the same shape** as the bug fixed this time — a send authorization with no declaration being opened as a side effect of another slot. The day that slot is turned on, the same defect comes back.
- **ADR-0133 decision 1's hidden classes became two** — the old class of the axis that decision splits (are mail families visible in `engram help`) was one, the **MCP-capable spawn**, and now the **spawn outside the mail plane** (`uses_mail=false`) is the second. ★**We do not discard ADR-0133**★ — both the single derivation point and 「표식은 교육, 강제는 데몬 거절」 (the marker is teaching, enforcement is the daemon's rejection) stand as they are, and all that changed is the number of classes that fall on that axis.
- **Keep the single derivation point as one** — the only place that produces `mail_allowed` is that one line in the daemon (ADR-0133 decision 2). Do not re-write the derivation at each consumer just because the inputs grew to two.
- **A `uses_mail=false` spawn gets neither the priming mail teaching nor the send-entrance grant** — the grant is split by the gate above, and the priming variant by `needs.uses_mail.then_some(…)` (`crates/engram-dashboard-daemon/src/control/mod.rs:214`). If those three (authorization · teaching · grant) diverge, 「가르쳤는데 거절된다」 (it was taught but is rejected) occurs again.
