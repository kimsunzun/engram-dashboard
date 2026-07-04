# ADR-0043: mount-replay = actor 경유 + deliverable 게이트 + 배정·등록 fresh 분기

- 상태: 확정 (2026-07-01, 근거: S14 모듈① 5차 cross-family 리뷰 BLOCK + 재검증 + cdp 실측)
- 관련: ADR-0040 · ADR-0006 · ADR-0007 · ADR-0042 · crates/engram-dashboard-core/src/output_view_store.rs · src-tauri/src/daemon_client/connection.rs · src-tauri/src/output_channel.rs · step-log S14 · Amended by ADR-0046 (deliverable gate·미러 cursor 메커니즘 조항: 폐기 → 뷰 buffering phase + gen 펜스로 대체 — mount-replay 원칙 자체는 전량 재replay로 승계)

## 맥락
새 창/슬롯이 agent를 보기 시작할 때 그 시점 버퍼를 **즉시 replay**해야 한다(수용기준 5 — 조용한 agent·재연결 대기 창도 빈 화면 0; on_frame fan-out만 기다리면 다음 출력까지 빈 화면). 이 mount-replay를 어디서·어떻게 하나. 그리고 출력 평면의 핵심 불변식 **"전달 성공한 slot의 cursor만 전진한다"(cursor advance ⟺ Channel delivery)** 를 모든 진입점에서 지켜야 한다 — core는 Tauri registry(Channel 등록 여부)를 모르므로(ADR-0012) src-tauri가 "현재 등록된 창 집합"(deliverable)을 주입한다.

## 결정
세 조항으로 확정한다.

1. **actor 경유 (순서 직렬화)** — mount-replay는 별도 경로가 아니라 출력 평면 connection actor의 select! arm(`ReplaySlots`)에서 `on_frame`(live)과 **같은 단일 스레드로 직렬 처리**한다. replay snapshot은 버퍼 락 밖에서 flush(ADR-0006). → replay와 live의 순서 역전 방지.

2. **배정 / 등록 트리거 fresh 분기** — 두 트리거를 구분한다:
   - **배정 트리거**(layout 델타, `fresh:false`) = `subscribe` — cursor가 **없을 때만** 신설+replay(이미 있으면 불가침).
   - **등록 트리거**(창 출력 Channel 등록 = webview mount/reload, `fresh:true`) = `resubscribe_slot` — cursor가 있어도 None 리셋 후 전체 replay(Channel 교체 = viewer 재시작이라 stale 이어보기 금지).

3. **deliverable 게이트 — cursor advance ⟺ Channel delivery (세 경로 균일)** — cursor 전진/snapshot은 그 창 Channel이 현재 registry에 등록된(deliverable) slot에만 한다. **membership(cursor 엔트리 신설)은 deliverable 무관, advance/emit만 게이트**한다. `on_frame`·mount-replay(`subscribe`/`resubscribe_slot`)·`reconcile_slots` 복구 **세 경로 모두** 동일 적용한다. reconcile 복구 판정은 엔트리 유무가 아니라 **cursor 값(전달 진도)** 기준이다: 값 None+deliverable=복구(전체 replay) / 값 Some=불가침 / 미deliverable=membership만 유지.

## 거부한 대안
- **mount-replay를 actor 밖에서 직접 flush** — `on_frame`(live)과 순서 역전(replay가 live 뒤에 도착) 가능. actor 직렬화가 replay→live 순서를 보장한다.
- **두 트리거 모두 무조건 fresh 리셋(옛 구현)** — 정상 mount(배정→등록)에서 같은 버퍼가 연속 2회 전체 flush, 무중복이 프론트 dedup에만 의존(ADR-0037 정신 어긋남). fresh 분기로 정상 mount = 전체 replay 1회.
- **deliverable 게이트를 on_frame에만 적용 + reconcile가 "엔트리 존재 = 전달됨"으로 판정** — 미등록 창에 배정 시 cursor만 전진하고 출력은 flush registry miss로 유실되는데, reconcile이 "엔트리 있음"을 전달 완료로 오판해 복구 못 함 → 등록 트리거마저 채널 full로 drop되면 **영구 빈 화면**. (5차 cross-family 리뷰가 수렴 적출한 BLOCK — 이 ADR의 직접 동인.)
- **min_render_seq(가장 뒤처진 창 합산) 재구독 모델** — ADR-0040이 폐기한 모델. 부활 금지(축 A는 버퍼 최신 seq).

## 근거
5차 적대 리뷰(opus doc-aware + Codex blind)가 deliverable 게이트 누락을 **BLOCK으로 수렴 적출**(cross-family) → 게이트를 mount/reconcile에 균일 전파하는 수정 → fresh adversary 재검증에서 **BLOCK 닫힘·신규 결함 0** 확인. cdp 실측(TauriTransport 런타임 경로): ① 라이브 배달 — 렌더러 등록 후 resize→redraw 출력이 새 평면으로 도착 ② 버퍼 전체 replay — 창 출력 Channel 재등록 시 seq 0~2(183 bytes) 전체 재배달 end-to-end. core 회귀 테스트: 미deliverable membership-only(전진 0) / 값None+deliverable 복구 / 정상 mount 1회 / reconcile 멱등.

## 영향 / 불변식
- `output_view_store.rs`: `subscribe`/`resubscribe_slot`/`reconcile_slots`가 `deliverable: &HashSet<S>`를 받고 advance/emit만 게이트(membership은 무관). reconcile 복구는 cursor 값 기준. on_frame 불변식 헤더가 "cursor advance ⟺ delivery" 정본.
- `connection.rs`: `ReplaySlots` arm + `resubscribe_and_sweep`가 deliverable를 **버퍼 락 전** `registered_labels`로 빌드해 주입(ADR-0006: buffer 락 ⊃ registry 락 중첩 0, snapshot은 락 밖 flush).
- `output_channel.rs`: `build_deliverable` 순수 헬퍼(3 호출부 통합·drift 차단).
- **어기면:** 순서 역전(actor 밖 replay) / 중복 replay(무조건 fresh) / 영구 유실(게이트 누락·엔트리 기준 reconcile).
