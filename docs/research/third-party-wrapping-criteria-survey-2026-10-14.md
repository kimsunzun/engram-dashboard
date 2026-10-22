# 서드파티 라이브러리를 언제 감싸나 — 기준 요약

- **상태:** light · 프리셋 라이브러리 슬롯 부품 결정에 적용(아래 「Engram 에 옮기면」) · 코드베이스 전체 점검은 별건
- **날짜:** 2026-10-14
- **방법:** research light. 수집자 1 · 메인 grounding(Cockburn 원문) · 적대 리뷰 없음.
- **계기:** 사용자 「라이브러리는 항상 브릿지로 감싸야 한다 — 단 표준 라이브러리까지 감싸는 건 쓸데없다, 확인해 봐라」.

## 결론

1. **감쌀 것 = 외부 시스템 · I/O 경계**(파일 · 네트워크 · 프로세스 · 벤더 API). Fowler Gateway — 복잡한 외부 API 를 일반 객체 인터페이스로 감싸 교체 때 그 안만 고친다(원문 · 수집자 확인). **확실.**
2. **감싸는 단위는 라이브러리가 아니라 「목적」이다.** Cockburn 헥사고날 — 포트는 기술이 아니라 「목적과 대화의 성질」로 가른다(예: 데이터 조회 목적의 대화 하나에 SQL DB 든 메모리 모의든 어댑터만 바꿔 붙인다) · 의도 = 앱이 실제 장치 · DB 없이 개발 · 시험되게 · 포트 수는 직관(2~4개)(메인 원문 확인). → 라이브러리 하나당 래퍼 하나가 아니다. **확실.**
3. **감싸지 않을 것 = 언어 표준 · 어디에나 쓰이는 기반(serde · tokio · React 자체) · 한 곳에서만 쓰는 순수 도우미.** 교체 가능성이 사실상 없고 래퍼는 비용만 든다. 얇은 1:1 투과 래퍼는 아래 세부가 그대로 새서(Spolsky 「사소하지 않은 추상화는 모두 어느 정도 샌다」 — 원문 · 수집자) 이득이 없다. **가능성 높음**(권위 문서의 명시 근거는 못 찾음 — 통설).
4. **공개 API 에 남의 crate 타입을 내보내면 그 crate 의 안정성에 묶인다**(Rust API Guidelines C-STABLE — 원문 · 수집자). 앱 내부에서는 약하고, crate 경계를 넘는 타입에서 걸린다. **확실.**
5. 「직접 소유한 타입만 흉내 내라」(GOOS) — 외부 타입을 직접 mock 하지 말고 자기 얇은 어댑터를 둔다(기억 — 미확인).

## Engram 에 옮기면 (세션 판단 — 사용자에게 보고)

| 라이브러리 | 감싸나 | 자리 |
|---|---|---|
| walkdir · serde-saphyr · `std::fs` | **목적 포트 하나로** — 「라이브러리 저장소」(에셋 목록 주기 · 에셋 읽기 · 에셋 쓰기). walkdir · YAML 을 각각 감싸는 래퍼는 두지 않는다 | 데몬 · 시험은 메모리 가짜로 |
| serde · tokio | 안 감쌈(기반) | — |
| react-arborist · TanStack Table/Virtual · react-selecto | 우리 화면 부품(`LibraryTree` · `LibraryList` — 자리표시) 안에 넣는다 — 어차피 짜는 부품이 곧 경계 | 프론트 |
| React · Zustand | 안 감쌈(기반) | — |
| TanStack Query(채택 시) | 기반 성격 — 「슬롯 데이터 받아 오기」 훅 정도만 | TRD |

## 출처

- https://martinfowler.com/eaaCatalog/gateway.html (수집자 원문 확인)
- https://alistair.cockburn.us/hexagonal-architecture/ (메인 원문 확인)
- https://rust-lang.github.io/api-guidelines/necessities.html (수집자 원문 확인)
- https://www.joelonsoftware.com/2002/11/11/the-law-of-leaky-abstractions/ (수집자 원문 확인)

## 한계

- light 라 적대 리뷰 없음. React 디자인 시스템 래퍼 관행 · 「래퍼 과잉」 직접 비판 · 「std · serde 는 감싸지 않는다」의 권위 문서는 못 찾음.
