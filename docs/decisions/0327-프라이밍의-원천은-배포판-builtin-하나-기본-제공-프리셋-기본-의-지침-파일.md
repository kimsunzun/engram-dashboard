# ADR-0327: The source of priming is one distribution `builtin\` — the instruction file of the built-in preset 「기본」

- 상태: 제안 (2026-10-31, basis: main session decision G1 (2026-10-20 · reported) · M7 (2026-10-21 · reported · TRD review #4 · #8) · M18 (reported) · G46 (2026-10-31 — main decision · delegated by the user: a partial amendment, not a discard))
- 관련: Amends ADR-0092 (Decision 1, the priming source path) (see 「바뀌는 조항」 below) · ADR-0099 · ADR-0100 · ADR-0211 · ADR-0215 (points at the same path or delivery — an item to confirm in 「영향」) · ADR-0329 · ADR-0337 · ADR-0328 · TRD §1-1 · §1-2 · §2-4 1 · 2 · §5 17 · §7 35

## 맥락
Today priming is a shared markdown file at a fixed path (`prompts/agent-priming.md` · cover `ENGRAM_PRIMING_FILE`) that a loader reads at spawn and injects into the system prompt — for everyone unconditionally (ADR-0092 Decision 1). S25 keeps built-in assets (the preset 「기본」 and its instructions and settings) as files of the same shape as the library and points to them with the fake path `engram:/…` (Δ1). The source that had been split across 「코드 + 배포판 `prompts\agent-priming.md`」 is gathered into one (G1).

## 결정
1. Built-in assets live in one distribution `builtin\` folder in the same shape as the library (preset `.yaml` · instructions `.md` · settings `.yaml`). `engram:/…` corresponds to those files (G1). The file names and the shape of the correspondence are implementation judgment.
2. The source of priming is one instruction file inside it (`engram:/priming` — today's priming text as is). The 「기본」 preset loads it as instructions (M7).
3. Resolution is done by asset-library itself — `LaunchKitSource::prepare` has no priming argument (M18).
4. If `builtin\` is absent, it comes up without priming — it does not block (M7).
5. It ships in the distribution zip (§5 17). How to find it in development, tests and `ENGRAM_DATA_DIR`, and when to withdraw the old path (`priming_file` that only `NoopLaunchKits` uses now), are open (TRD §2-4 1 · 2 · M25 LEFT OPEN).

**Clauses that change (ADR-0092):**
- **Only Decision 1 changes — it is a partial amendment** (G46 — main decision · delegated by the user · TRD §7 35 was also fixed to 「폐기가 아니라 개정」). Decisions 2 (1:1 first · multi-recipient abstraction) · 3 (format decision axis acceptance > tokens) · 4 (happy-path-first) are unrelated to the priming source and stay.
- What changes in Decision 1: ① path — `prompts/agent-priming.md` in the repo → the instruction file in the distribution `builtin\` ② loader seam → the launch-materials port `LaunchKitSource` (the place that ADR-0092 foretold as 「미래 구현체 = 에이전트별 프롬프트 인젝션/스킬등록 시스템(로더 교체로 흡수)」) ③ 「임시판은 전원 무조건 주입」 → only agents with 「기본」 attached (attached at creation, detaching is confirmed by `yes` — M21).
- What stays in Decision 1: no hardcoding (still an external file) · the daemon reads it and passes it toward the system prompt (the agent does not read the file by itself). The first line of ADR-0092 「영향」 (no hardcoding of priming content · the loader is a seam) is still kept.
- The TRD writes Decision 4 of this ADR (not blocking when absent) as 「ADR-0092 와 같은 너그러움」, but there is no such clause in the ADR-0092 body — we have not confirmed whether it is today's code behavior.

## 거부한 대안
- **The 「코드 + 배포판 `prompts\agent-priming.md`」 split** (the shape that G1 replaced — built-in preset and settings definitions are in a content table in code (`builtin.rs`) · only the priming text is a distribution file · TRD first edition 2026-10-15 §1-1) — basis (main 2026-10-31 — reported): ① built-in assets are browsed in the library screen as a read-only root (preset → the instructions, settings and detail areas inside) and copied out into the library (R43 · G2-REVISED · P6 · M14 — user decisions) — only files of the same shape as the library let the same scan, resolution and detail run as they are, and a content table in code demands a second path for the built-in share on each of those paths. ② The settings of 「기본」 (`allowedMcpServers` · `DISABLE_AUTOUPDATER`) must be visible (T10 — user 「사실 감추는 설정이 있으면 안되지」) — if they are files, they show on the same screen as they are. ③ The reason ADR-0092 did not embed priming in code (「내용 수정마다 재빌드」 · not swappable — the rejected alternative of that ADR) applies equally to the preset and settings definitions of 「기본」.
- **Discarding ADR-0092 as a whole (supersede)** (the old wording of TRD §7 35 · §5 0c) — only the fixed path of Decision 1 changes, and Decisions 2~4 are unrelated to the priming source and still stand (G46 — main decision · delegated by the user).
- (Reference · not a rejection) Keeping priming itself as a user library asset is a non-goal · a later thought (PRD R52 · decision record ROUND 2 「LATER IDEA」).

## 근거
G1 · M7 · M18 (main — reported). TRD review #4 · #8 that M7 points at. The basis of the rejected alternative = above (main 2026-10-31 — R43 · G2-REVISED · P6 · T10 · ADR-0092).

## 영향 / 불변식
- The one place to edit the priming text is the instruction file of `builtin\` — it is not put back into code.
- Detaching 「기본」 removes both the priming and the Engram channel settings (`allowedMcpServers` — ADR-0328) — the detach confirmation text says so (R32 · `DetachDefaultConfirm`).
- The old path (`priming_file` of `NoopLaunchKits`) stays until the time to withdraw it is decided — the Noop regression golden (argv bytes identical) relies on it (§2-4 2).
- **To confirm (at `/adr link`)**: ADR-0099 (Decision 4 — already partially discarded by ADR-0211) · ADR-0100 (distribution assembly) · ADR-0211 · ADR-0215 point at the `prompts/agent-priming.md` path or at priming delivery. We have not decided whether this decision changes those clauses too.
