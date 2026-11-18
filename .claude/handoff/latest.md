# Handoff: S37 multi-daemon — every PRD/TRD decision taken; next = reference docs for touched topics, then PRD+TRD review

## Know first
- Mode: autonomous. The user is learning networking as we go: explain big picture first, short, plain words, no invented labels; gloss work labels (S37, U1, D3, T-115 …) on first use (CLAUDE.md). Concrete data-flow / timeline explanations land; abstract ones do not.
- New process rules (commits 03d9ef22 · 54916d0c/cb360c75 · e297b42b/7b280a51 · 29e66b90/31dfb6c8, all on this branch): HTML only on request; step-log frozen; topic number from folder/commit max + 1; TRD = decisions and boundaries only; finished TRDs are not read; **existing topics without a reference md that this work changes must get one written from current code before that step's coding, and it goes into the PRD+TRD review** (implement binding 「레퍼런스」); decisions later work relies on → ADR before coding.
- Before PRD+TRD review the binding requires a chat summary of the TRD and the user's answer to 「이대로 리뷰로 갈까요?」 (autonomous mode too). Not asked yet.
- Untracked `.claude/explain/` (two old explainer HTMLs from this session's research phase) — leave or delete; not part of S37.
- The scratchpad of earlier days is gone (session restart); everything needed is in the repo.

## One-line state · next first action
S37 PRD and TRD are at draft 2 with all decisions taken (PRD D1–D7, TRD X1–X6); no code yet.
**Next first action:** write reference docs from current code for the existing topics S37 changes — transport crate, messaging kernel, daemon link/command table (only `agent-backend.md` exists under `docs/reference/structure/`) — as a docs-only commit; then show the TRD chat summary and ask 「이대로 리뷰로 갈까요?」; then `/engram-review` PRD+TRD (reference docs included); then U0 ADRs (five, listed in TRD §5) before coding.

## Repo state (measured 2026-11-19 02:27)
- Branch `v0.5.0/feat/multi-daemon` @ `4d74922b`, pushed (in sync with origin). 19 commits ahead of `origin/master`, 0 behind (origin/master merged at `d97082f9`; S35 i18n merge `87dbc7bb`).
- Untracked: `.claude/explain/` only. This handoff commit comes after these numbers.
- `v0.5.0/chore/misc` holds the four process commits not yet on master; their content is already on this branch (cherry-picks).

## Verification state
- Docs only so far — nothing to build or test. Research reports were cross-family reviewed (codex FIX applied twice). The PRD/TRD have **not** been reviewed.
- Spot checks done in code during decisions: `eg_send` has `request` + `reply_by` (daemon `control/mcp_server.rs`); notices are parked in the mailbox notice lane and injected at the idle gate (messaging `mailbox.rs`, ADR-0104); Syncthing `LoadOrGenerateCertificate` regenerates on any load failure; Reason Network `load_key` errors on a corrupt key.
- Unverified (U1/U5 will measure): iroh direct-path preference in practice, Windows Firewall prompt for a WMI-launched daemon, mDNS on corporate LANs, exact iroh ID text format.

## Do-not / lessons
- Do not present iroh-handled behaviour as user choices (the user rejected "options" for path priority — it is automatic, direct first). Do not say "our server": Engram runs none; users may host `iroh-relay` + `iroh-dns-server`.
- Do not claim a unit has "no decisions" without checking the TRD dependency table (the brief writer caught that U1/U2 carry D4/D6/X1/D1/D7/X5).
- A separate system input to an idle agent costs a whole turn (cache read ∝ context + output) — the user cares; see T-115.

## Resume command
`git -C I:/Engram/apps/engram-dashboard log --oneline -3 && git -C I:/Engram/apps/engram-dashboard status -sb | head -3`

## Decisions
User-decided:
- Multi-daemon direction · 상황: GPT federation draft vs our ADR-0179/0310 two relationships · 선택지 daemon↔daemon mail first ★ / shell→N daemons first / both · 고른 것 「난 그냥 데몬간 통신만 생각하고」 + 「그 순서로 하면 된다는거지? 괜히 구조 바꾸고 하는건 없는거고」 · 같음 · 조건: lay the permission field and per-relationship ALPN now so (a) is additive.
- Link library · 상황: user wants no external account/install · 선택지 iroh ★ / TLS+pinning over user VPN / Tailscale / libp2p · 고른 것 「로그인을 해야된다고? 그럼 우리게 아니잖아」 → iroh after an alternatives survey (「없으면 이걸 하되」) · 갈림 (Claude first recommended TLS over VPN; feedback logged in engram-research `1bd4dc7`).
- Helper servers (D4) · 상황: n0 free servers may change · 선택지 n0 default + user-hosted server URL or off + per-PC address, all paths tried, direct wins ★ / n0 only / none · 고른 것 「우리서버는 없어. 상대방이 알아서 서버 구축하는건 있고」 · 같음.
- Pairing (D1) · 선택지 paste ID both sides ★ / accept prompt / one-time code / invite link · 고른 것 「1먼저 해.」 · 같음 · 근거: 「결국 2,3,4는 1이 되야 가능한거 아냐?」.
- Remote addressing (D2) · 상황: LLM rules must not hard-code which PC an agent lives on · 선택지 `name@pc` only ★ / bare names via shared directory + `name@pc` · 고른 것 「한마디로 에이전트 목록 동기화가 되야된다는거네 모든 pc에」 → 「반영해봐」 · 갈림.
- Other PC off (D3) · 선택지 fail at once ★ / outbox now · 고른 것 「다음 단위로 해야될것같은데? 이득에 비해서 얻는게 적다」 · 같음 · 조건: a send cut off by a link drop still gets system feedback → late-outcome notice via the existing notice lane (「일단은 그냥 가자. 지금처럼」).
- Inbound limits (D5) · 선택지 Reason Network values ★ / other / none · 고른 것 「resason network 에 맞춰서 두던지. 그리고 설정값으로 빼둘 수 있도록」 · 같음 · 조건: one injected limits value, setting-ready (no settings module yet).
- LAN discovery (D6) · 선택지 on ★ / off · 고른 것 「손해 없음 켬으로해」 · 같음.
- When it runs (D7) · 선택지 only with ≥1 trusted PC ★ / always · 고른 것 「pc 하나 이상 등록한 뒤부터」 · 같음.
- Default port (X1) · 선택지 fixed default, configurable ★ / random · 고른 것 「디폴트 선택을 고정포트로 할거냐잖아. 다음」 · 같음.
- PC list place (X2) · 선택지 new slot ★ / dialog / none in v1 · 고른 것 「새로운 슬롯이 있어야지」 · 같음 · 조건: slots are plugins, daemon does behaviour, client presents → T-116.
- Held-mail receipt (X3) · 선택지 none ★ / notice · 고른 것 「송수신 우편 옵션일때만 보내는거잖아」 → none (requests already get reply/overdue) · 같음.
- Cross-PC reply deadlines (X4) · 선택지 receiver-side ★ / both sides · 고른 것 요지: rarely used, keep simplest (「거의 사용되지 않을것 같은데」) · 같음.
- Corrupt key (X5) · 선택지 auto new + backup + warning ★ / stop · 고른 것 「UI 쪽에 메세지 안내하고 유저가 ok 하면 백업만들고 새로 만들면」 · 갈림 (stop + user-OK regenerate).
- Directory copies (X6) · 선택지 memory ★ / file · 고른 것 「메모리에만」 · 같음.
- Input layer (T-115) · (묻지 않음) urgent vs deferred system inputs · 고른 것 「별도의 주제로 진행하면 될듯」, direction: typed inputs, flush all if any urgent; worry about invisibility; PTY vs JSON difference.
- Multi-user sharing of one PC's agents · 고른 것 「툴콜이 안되는거아냐? 애초에 좀 더 생각해보긴 해야됨」 → parked (Consumer Terms + tool location), plan doc §6.
Claude-decided: S37 number via the handbook (max 36 + 1); branch `v0.5.0/feat/multi-daemon`; merged origin/master twice (conflicts: CLAUDE.md 「언어」 took master's i18n lines minus the step-log mention; implement binding kept this branch's copy — master's copy still has the superseded 「사용자용 HTML — 멀티데몬 주제부터」 section, not fixed on master); iroh types stay inside a transport `iroh` feature; PeerWire lives in the daemon; ledger keys stay agent UUIDs; `peer.*` = first daemon-declared command table; U4 strings follow S35 i18n (en.ts source).
Open: PRD approval + 「이대로 리뷰로 갈까요?」 (ask after the reference docs); `.claude/explain/` cleanup.

## References
- `docs/process/S37-multi-daemon/` — README, prd.md (draft 2), trd.md (draft 2: §3 data shapes, §4 flows incl. §4-7 directory sync, §5 T1–T22 + five U0 ADRs, §6 test plan, §7 units U0–U5, §8 X1–X6, §9 plan vs code), spec-brief.html (big-picture, made on request; predates D3–X6 wording).
- Research: `docs/research/multi-daemon-federation-direction-2026-11-11.md`, `docs/research/multi-daemon-iroh-link-plan-2026-11-11.md`.
- Reference clones: `I:\Engram_Workspace\opensource\reason-network` (closest precedent: `src/net.rs`, `src/daemon/mail.rs`, `src/daemon/directory.rs`, `src/team.rs`), `syncthing`, `hcom`.
- Tracking: T-115 (agent input layer), T-116 (slots as plugins), T-85 · T-86 (shell → N daemons stage).
- Bindings: `.claude/skill-bindings/implement.md` 「레퍼런스」「TRD」, `review.md`, `qa.md`.
