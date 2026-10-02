# 공용 구조체를 어디에 두나 — wire 패킷 crate 와 다른 crate 의 구조체 (light 조사)

- **상태:** light 조사(수집자 1 · 적대 리뷰 없음) · 2026-10-02 · 결정으로 이어짐 → `../refactoring/architecture-discussion-2026-09-26.md` 결정 후보 8
- **질문:** 클라·서버가 wire 패킷 crate 를 공유할 때, 패킷 crate 가 다른 crate 의 구조체를 직접 품어도 되나 — 아니면 wire 전용 사본 + 끝자락 변환, 또는 공용 스키마 crate 를 따로 두나.
- **확신도 범례:** 확실 = 1차 자료로 메인이 확인 · 가능성 높음 = 1차 자료 인용이나 메인 미확인 · 불확실 = 관행 관찰뿐.

## 결론

- **B(패킷 crate 가 wire 전용 사본을 갖고 끝자락에서 변환)가 가장 명시적으로 문서화된 관례다.** rust-analyzer 아키텍처 문서: LSP·JSON 직렬화를 아는 crate 는 하나뿐이고, 내부(`ide`) 자료구조는 직렬화 가능하게 만들지 말고 직렬화용 짝을 따로 만들어 손으로 변환하라 — 변환은 `to_proto`/`from_proto` 모듈. **가능성 높음**(수집자 인용 · 메인 원문 대조는 못 함).
- **C(공용 스키마 crate)는 「행동 없는 데이터 형식」을 여러 소비자가 같이 쓸 때 쓴다.** Cargo 의 `cargo-util-schemas` = "Low-level Cargo format schemas"(serde·FromStr 중심) — 외부 도구가 Cargo 로직 없이 스키마만 쓰려고 갈라냈다. **확실**(docs.rs 메인 확인). `lsp-types` 도 여러 서버가 공유하는 스키마 crate 로 이 모양 — **가능성 높음**.
- **A(패킷 crate 가 다른 crate 구조체를 직접 품기)를 기본으로 권하는 1차 자료는 없다.** prost/tonic 쪽에서 변환 보일러플레이트를 피하려 도메인 구조체에 직접 붙이는 사례는 있으나 왕복 테스트로 결합을 막아야 하는 지름길로 취급된다 — **불확실**(관행 관찰).
- **불투명 칸(이름 + JSON 인자)은 JSON-RPC/LSP 봉투의 표준 모양이다** — 봉투 층은 내용 타입을 모르고, 위층이 `method` 로 갈라 푼다. A/B/C 와 다른 축이다(봉투는 불투명, 그 위에서 B 식 변환이 다시 생긴다). **가능성 높음.**

## 우리에게

- `protocol` = 우리의 `lsp-types` 자리(C 모양의 공용 스키마) — **아무것도 품지 않는 패킷 목록**으로 둔다.
- 기능 데이터 = B — `protocol` 사본 + 셸·데몬 끝자락 변환(rust-analyzer 의 `to_proto`/`from_proto` 자리 = transport 어댑터).
- 명령 버스 = 불투명 칸 — 이름(겉봉) + JSON(속), JSON-RPC 와 같은 모양.

## 못 본 것

- Zed · Deno · nushell · wezterm 의 관례 — 예산 안에서 1차 자료를 못 찾아 기권.
- rust-analyzer 문서 원문 대조 — 메인 확인 시도가 이번 세션에서 중단됐다.
- `lsp-types` 의 실제 의존 그래프.

## 출처

- rust-analyzer 아키텍처: https://rust-analyzer.github.io/book/contributing/architecture.html
- cargo-util-schemas: https://docs.rs/cargo-util-schemas · https://github.com/rust-lang/cargo/pull/13178
- prost: https://github.com/tokio-rs/prost
