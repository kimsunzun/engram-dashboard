# ADR-0316: claude 실행 재료 = 합친 지침 파일 + `--system-prompt-snapshot off` + `--plugin-dir` 정션 + 합친 `--settings` 하나

- 상태: 제안 (2026-10-31, 근거: TRD 결정 T7(확정 2026-10-18 — 추천 그대로) · T10(사용자 2026-10-18) · R35 · G42(2026-10-31) · 실측(TRD §1-5 표 · §6-1))
- 관련: ADR-0334 · ADR-0327 · ADR-0328 · ADR-0109 결정 9 · TRD `docs/process/S25-preset-library/trd.md` §1-5 「backend 번역」 · §7 4

## 맥락
claude 두 모드(JSON · 터미널)에 프리셋 재료를 어떤 플래그로 싣나.

## 결정
1. 지침: 실리는 순서로 합친 파일 하나 → `--append-system-prompt-file`(지금 프라이밍 자리를 대신). 실 재료일 때만 `--system-prompt-snapshot off`(이어받기에도 새 글).
2. 스킬: 휘발 폴더 `engram\skills\<이름>` = 라이브러리 스킬 폴더로 가는 디렉터리 정션 → `--plugin-dir <휘발>\engram` → 이름 `engram:<스킬>`.
3. 설정: 붙인 순서로 합친 claude 원문 → 휘발 `settings.json` 하나 → `--settings` 하나(ADR-0328).
4. 모델은 따로 싣지 않는다 — 원문의 `model` 키가 `--settings` 로 간다(R35).

## 거부한 대안
- **스킬을 `--add-dir` 로** — 접두가 없어 네이티브 스킬과 이름이 겹치면 승자를 모른다(T7). 고른 쪽은 사용자 `~/.claude/skills` 와 안 섞이고 적재가 실측됐다(T7).
- **`--settings` 여럿** — 두 번째 `--settings` 의 동작이 문서에 없고 재지 않았다(§7 19).

## 근거
- claude 2.1.284 `--help`: `--plugin-dir`(반복 가능) · `--settings <file-or-json>`(한 번 꼴) · `--system-prompt-snapshot <on|off>` 있음(TRD §6-1 — 확실).
- 플러그인 폴더 적재 = 실측 확실 · 정션 따라가기 = 미측 · snapshot off 로 이어받기에도 새 글 = 실측 확실 · 같은 글이면 캐시 손해 없음 = 실측(TRD §1-5 표).

## 영향 / 불변식
- 정션 따라가기는 미측이다 — 해당 단계에서 잰다. Noop 재료는 지금 argv 를 바이트 그대로 낸다(ADR-0328).
- 떠 있는 에이전트는 정션 너머 라이브러리 스킬 폴더를 본다 — 세션 도중 그 폴더를 옮기거나 이름을 바꾸거나 지우면 그 세션이 깨지거나 「파일 열림」 으로 실패한다. 지금 CLI 스킬 폴더와 같은 동작이고 따로 다루지 않는다(G42 · 사용자 「그건 기존에도 똑같은거 아냐?」 · TRD §1-11a 발견 14 닫힘).
- 정션 만들기 · 끄기의 부품 = ADR-0336(`junction` 2.1.0 `create` · 끄기 = 링크임을 먼저 확인한 뒤 `std::fs::remove_dir`).
