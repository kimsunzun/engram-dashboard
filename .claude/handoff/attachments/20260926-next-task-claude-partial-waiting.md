# 다음 작업(사용자 지시 2026-09-26) — claude 조각 스트리밍 + 채팅 「waiting…」 표시

- **사용자 인용:** 「일단 클로드 조각 옵션 켜줘. 다음작업으로 기억하고, 채팅충에 waiting.. 도 이 참에 같이 진행하자.」
- **계기:** 사용자 관찰 — codex 는 글이 조금씩 나오는데 claude 는 다 끝난 뒤 한 번에 쏟는다. 원인 = claude 를 `--include-partial-messages` 없이 띄워 CLI 가 메시지 블록이 끝나야 통째로 보낸다(codex 는 `item/agentMessage/delta` 조각을 흘린다).
- **범위(둘을 함께):**
  1. claude JSON 모드 스폰에 `--include-partial-messages` 를 켜고, 해석기가 `stream_event`(content_block_delta 등) 조각을 `TextDelta` 로 흘리고 완료 블록과 겹치지 않게(중복 없이) 합친다 → 화면까지.
  2. 채팅의 「waiting…」(생각 중·경과) 표시 — 백로그 **T-12**(`docs/tracking.md:216` · 선결조건 목록 `:221` 부근: partial 미활성 · turn_id 부재 · Error 판별자 · 경과 시계 의미). 턴 도중 입력 P1 이 턴 끝 종류(`TurnEndKind`)·오류 사실을 이미 깔았다 — 선결조건 일부가 풀렸는지 확인할 것.
- **주의:** 플래그만 켜면 보이는 변화가 없다 — 오늘 claude 해석기는 모르는 줄(`stream_event`)을 건너뛴다. 실제 일은 해석기 + 프론트다. 턴 도중 입력 P3a 가 같은 claude 해석기를 고치므로 **둘을 동시에(병렬로) 하지 않는다** — 순서대로.
- **개발 스텝:** CLAUDE.md 순서(PRD/TRD 선택 → 사용자 결정 → 구현). T-12 는 리서치가 끝나 있다(`docs/tracking.md` T-12 항목의 조사 링크).
- **순서:** 사용자가 「다음 작업」이라 했다 — 진행 중이던 두 워커(P5a 코더 · P4 로컬 QA) 마무리와 핸드오프 뒤, 다음 세션의 첫 작업. 턴 도중 입력의 나머지(P5a·P5c·P3·P2·P7·P8·P6)는 그 뒤에 잇는다(순서를 바꿀지는 다음 세션 시작 때 사용자에게 한 줄 확인).
