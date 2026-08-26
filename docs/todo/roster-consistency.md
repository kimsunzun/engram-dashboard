# 명부 정합 (T-32 + 「재생·구독」의 뷰·데몬 정합 + T-16) — 큰 덩어리, 조사는 끝나 있다

> **상태:** 착수 가능 · **정본 색인:** [`../todo.md`](../todo.md)

> **연관:** 아래가 가리키는 「뷰·데몬 껐다 켜기 정합」 항목의 본문은 [`replay-subscription.md`](./replay-subscription.md) 에 그대로 있다.

- **범위:** 하나의 결함이 아니라 **세 요구가 묶인 덩어리**다 — ① 발행 순서 ② 프레임 유실 복구 ③ 상태 알림과 스냅샷 사이의 경쟁. ★③은 조사 도중 적대 리뷰가 새로 찾은 축이라 `../tracking.md`·[`replay-subscription.md`](replay-subscription.md) 어디에도 없다★.
- **★먼저 읽을 것 = [`../research/roster-broadcast-ordering.md`](../research/roster-broadcast-ordering.md)★** — 피어 서베이·판정 규칙·저장소 제약·적대 리뷰 결과가 거기 있다. **후보 A/B/C/D/E 가 정리돼 있고 선택은 안 됐다**(잠정 추천이 리뷰에서 기각됐다). 그 문서 없이 시작하면 같은 조사를 다시 돌게 된다.
- **정본:** `../tracking.md` T-32·T-16 · [`replay-subscription.md`](replay-subscription.md) 「뷰·데몬 껐다 켜기 정합」.
- **★사용자가 한 번 미뤘던 것을 다시 연 것이다★** — [`replay-subscription.md`](replay-subscription.md) 의 그 항목에 「사용자가 나중 이슈로 분류」가 붙어 있는데 2026-08-26 에 처리하기로 뒤집었다.
- **착수 선결:** 같은 항목이 적어 둔 구조 결정이 먼저다 — *명부 조회가 원래 읽기였는데 부착 권위로 승격됐다*. 그걸 정하지 않으면 순서 수정의 설계가 갈린다.
- **★굵은 설계 결정이라 사용자 선택 없이 구현 진입 금지★**(`CLAUDE.md` 「개발 스텝」). 조사는 끝났으니 선택지를 사용자에게 올리는 것이 다음 행동이다.
