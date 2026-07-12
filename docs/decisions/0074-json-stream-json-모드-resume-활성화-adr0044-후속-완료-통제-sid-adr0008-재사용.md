# ADR-0074: json(stream-json) 모드 resume 활성화 — ADR-0044 후속 완료 (통제-sid/ADR-0008 재사용)

- 상태: 확정 (2026-07-13)
- 관련: ADR-0044(json 모드 배선 — resume 을 후속으로 스코프아웃) · ADR-0008(세션 복원 통제-sid) · ADR-0030(capability) · `crates/engram-dashboard-core/src/agent/backend/claude.rs`(build_spec StreamJson·capabilities) · `session.rs`

## 맥락

ADR-0044 는 json(stream-json) resume 을 MVP 밖(후속)으로 두고, build_spec 이 SpawnMode 무관 항상 `--session-id`(fresh)로 고정하고 caps 를 resume=false 로 신고했다. 사용자가 "터미널과 동일하게 resume 되게 붙여"를 요청.

## 결정

json 모드도 터미널 분기와 동일하게 build_spec 이 mode 로 세션 플래그를 가른다(Fresh→`--session-id`, Resume→`--resume <sid>`), caps 는 두 모드 모두 resume=true. 통제-sid(ADR-0008) 인프라를 그대로 재사용 — json 전용 신규 기계 없음. 트리 "에이전트 생성"의 기본 output_format 도 이와 함께 StreamJson 으로 전환(resume 이 보장되므로 안전).

## 거부한 대안

- **보수적 resume=false 유지(현행)** — 사용자가 명시 요청했고, spike 로 안전이 확인됐으며, resume 없으면 정지 후 재개 시 대화가 조용히 소실되므로 유지할 이유가 없다.
- **Mechanism B(매 턴 `claude -p --resume` 재기동)** — ADR-0044 가 이미 거부(매 턴 시동비용 + mid-turn 개입 불가). 현 Mechanism A(지속 프로세스)에 startup `--resume` 만 더한다.

## 근거

spike 실측(2026-07-13, claude 2.1.170) — `claude -p --input-format stream-json --output-format stream-json --replay-user-messages --resume <sid>` 가 `-p`/stream-json 과 공존하며 "session already in use" 없이 과거 대화를 무손실 재개함을 확인(비밀코드 회상 성공 + `cache_read_input_tokens` 로 이전 컨텍스트 로드 입증). 즉 능력 부재가 아니라 미배선이었다. restore 경로는 backend-agnostic(`needs_session` 무조건 true + `claude_session_id` 영속) 이라 json 도 터미널과 동일 레일을 탄다.

## 영향 / 불변식

- resume 실패 시 조용한 fresh 가 아니라 시각적 `FreshFallback`(loud).
- `capabilities` 는 이제 두 모드 모두 resume=true(mode 무관).
- ADR-0044 §6 의 "resume 복원 정합 후속" 항목은 이 ADR 로 완료.
- **미검증 잔여:** 앱 레벨 왕복 실측(데몬 재빌드 필요)은 미완 — CLI spike + 코드 리뷰(deep)로 정합 확인, 앱 E2E 는 후속.
