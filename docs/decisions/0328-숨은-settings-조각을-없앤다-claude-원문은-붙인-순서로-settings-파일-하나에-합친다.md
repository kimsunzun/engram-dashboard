# ADR-0328: Remove the hidden settings fragment — merge the claude raw text into one `--settings` file in attached order

- 상태: 제안 (2026-10-31, basis: user decision T10 (2026-10-18 「프리셋으로 해줘. 사실 감추는 설정이 있으면 안되지.」 · 「이어붙이기 진행」)) · 부분 폐기 by ADR-0351 (쓰기 실패 처리)
- 관련: Amends ADR-0109 (the settings fragment part of Decision 9) (see 「바뀌는 조항」 below) · ADR-0099 (mcp-config lifecycle) · ADR-0327 · ADR-0316 · PRD R32 · TRD §1-2 · §1-5 table · §3 merge rules · §7 19 · Amended by ADR-0351 (쓰기 실패 처리)

## 맥락
Today the control channel loads a single-key fragment file `{"allowedMcpServers":[{"serverName":"engram"}]}` with `--settings` on every spawn (ADR-0109 Decision 9 · `control/mcp_config.rs`). S25 also loads the preset's claude raw-text settings with `--settings`.

## 결정
1. There is no hidden settings fragment. `allowedMcpServers:[{serverName:"engram"}]` and `DISABLE_AUTOUPDATER` are visibly contained in the claude raw text (`engram:/settings/default`) of the built-in 「기본」.
2. Merge the claude raw text of the attached presets (including 「기본」) in attached order into one volatile folder `settings.json` and load it as one `--settings`. Merge = later values win · objects are recursive · lists are concatenated and equal values collapse to one · no per-key rules · no Engram-priority rule. The merging side is the backend settings-check port (ADR-0334).
3. For real material, the control channel fragment (`ControlEndpoint.settings_file`) is not loaded. `NoopLaunchKits` still loads it as it does now (argv bytes identical).
4. `--mcp-config` is out of scope (run arguments as they are · not a preset).

**Clauses that change (ADR-0109 Decision 9):**
- Changed: 「조각은 `allowedMcpServers` 단일 키 유지(잡동사니화 금지 — 회귀 테스트로 고정)」 → there is no fragment, and that key moves into the 「기본」 raw text. 「mcp-config 디렉토리 · 스윕 · revoke 수명주기에 동승」 → the merged file lives in the launch-materials volatile folder (`daemon\run\launch\` — TRD §1-4).
- Unchanged: 「`--settings` = 파일 경로 주입(인라인 JSON 아님)」 and its reason (`cmd /c` quote collapse).
- **Not in the source (to confirm)**: what happens to 「쓰기 실패는 warn+계속」 in the new edition.
- The Noop path runs as in Decision 9 until it is withdrawn.

## 거부한 대안
- **A fragment injected in hiding** — user 「사실 감추는 설정이 있으면 안되지」 (T10).
- **For `allowedMcpServers`, the Engram value is the authority** (the old recommendation — drop that key of the preset and warn) — the user chose no hidden settings (T10).
- **Swapping the list in** — user 「이어붙이기 진행」. Consequence: a later preset cannot remove `engram` — accepted.
- **A second `--settings`** — its behavior is not in the documentation and was not measured.
- **Not loading the raw text** (not loading the preset's claude raw text — a branch only named, never worked out, since the first edition of the TRD on 2026-10-15) — basis (main 2026-10-31 — reported): the settings asset is a per-backend raw-text field, and the claude field is passed through `--settings` (PRD R5 — user 2026-10-18) — if it is not loaded, the settings asset does nothing for a claude agent. Also, since T10 moved the Engram channel keys into the claude raw text of 「기본」, not loading the raw text means those keys go missing or must return to a hidden fragment (T10 — user 「사실 감추는 설정이 있으면 안되지」).

## 근거
User decision T10 (2026-10-18) · TRD §2-2 T10.

## 영향 / 불변식
- Detaching 「기본」 may remove the Engram message and control channel (when the user's global settings block it with `allowedMcpServers: []`) — the detach confirmation text says so (R32 · `DetachDefaultConfirm`).
- The merged claude settings are also checked — an error is attached to the line of the later asset that brought it in (M14).
- The anchor `mcp_config.rs` (settings fragment) of ADR-0109 remains only on the Noop path.
