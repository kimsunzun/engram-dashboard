# P3c1 B2 리뷰(deep · 3렌즈 전부 FIX) — 보류된 4번째 수정 라운드 (하드캡 3 초과 → 사용자 승인 대기)

## 반영할 것 (메인 판정)
1. **포커스 강탈(medium)** — LLM 이 뒤에서 수락하면 `focus(main)` 이 tao `force_window_active` 로 가짜 Alt 를 사용자 앞 앱에 쏜다(restore.rs:769-774 · placement.rs:526-532). 고침: 「우리 창 중 하나가 포커스를 가졌을 때만」 focus — 포트 `app_has_focus()`(② 전 표본) · 가짜로 시험.
2. **최대화 실패 시 대기 깃발 유실(medium · codex)** — `apply_deferred_maximize` 가 깃발을 먼저 지우고 maximize 실패를 삼킨 뒤 최대화 아님을 기록(placement.rs:398). 고침: 실패면 깃발 유지 · 성공 뒤에만 record.
3. **도움말(prompts/engram-help.md:77-83 · commands.rs ~:458 요약):** 답한 뒤 window.list · tab.list 다시 읽기(팝아웃 label 새로 매김) · 「windows 와 같은 방식으로 센다(같지 않을 수 있다)」 · TIMEOUT 이면 끝까지 진행됐을 수 있음 → restore.status 로 확인 · (선택) 쥔 label/view_id 가 갑자기 안 맞으면 restore.status 먼저.
4. INTERNAL(포트 미부착) 를 `CommandError::with_retry(Internal, …, AfterCondition)` 로(commands.rs:1165 · restore.rs:424).
5. commands.rs:60 「새로 짓는 이름은 여섯」 손 셈 — 숫자 걷기.
6. (작음) open_hidden 과 defer_maximize 사이 트레이 show+닫기 → 대기 항목 영구 잔류(label 재사용 없음이라 무해) — 순서를 defer 먼저로 바꿀 수 있으면.

## 기각
- codex 「생성 바인딩 끝 공백」 — 기존 커밋 바인딩도 같은 ts-rs 출력이고 CI 는 재생성 비교라 무관.

## 사용자 결정 필요
- **도움말의 답 정책:** LLM 이 수락/거절을 스스로 정해도 되나? 거절(durable)은 사본을 지워 되돌릴 수 없다. 추천 = 「사람이 시키지 않았으면 사용자에게 묻고 답한다」 한 줄.
