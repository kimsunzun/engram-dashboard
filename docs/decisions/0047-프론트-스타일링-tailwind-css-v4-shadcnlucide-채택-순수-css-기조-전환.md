# ADR-0047: 프론트 스타일링 = Tailwind CSS v4 + shadcn/lucide 채택 (순수 CSS 기조 전환)

- 상태: 확정 (2026-07-05, 근거: /research medium + Codex 적대리뷰 + CSS 실측)
- 관련: CLAUDE.md "기술 스택(프론트)" (`CSS 변수(Tailwind X)` 문구 갱신 대상) · `src/styles/theme.css`(data-theme 3종 테마) · step-log 2026-07-05 · Amended by ADR-0048 (채팅 UI 렌더 방식: CC룩 네이티브 직접 구현·OSS 참조한정(코드 복붙 아님) → Cline 잎 컴포넌트 verbatim 코드 포트(Apache-2.0 귀속))

## 맥락
구조화 출력(StreamJson) 슬롯의 채팅 렌더를 **Claude Code VS Code 확장 룩**(세로 점선 타임라인 · 접힘 "Thought for Ns" thinking · 미니멀 툴 IN/OUT)에 맞추려 했으나, 직전 이식은 "근사"(이모지 아이콘·손수 만든 `<details>` 아코디언)에 그쳐 품질이 크게 떨어졌다.

원인 규명을 위해 `/research`(medium, 설계-결정 모드) + Codex(cross-family) 적대 리뷰를 돌린 결과:
1. **Claude Code 확장 = 완전 폐쇄 소스**("All Rights Reserved"). CLI 소스맵 유출(2026-03)도 법적 사용 불가 → **CC 룩을 복사할 OSS 소스가 없다.**
2. **OSS 후보(Cline·Roo·Kilo-legacy·Continue) 전부** Tailwind CSS + shadcn/Radix(일부 styled-components) 강결합이고, **아무도 CC 시그니처 룩을 안 가진다**(전부 "VS Code 사이드바" 미학).
3. 우리는 **순수 수기 CSS**라 후보 코드를 가져오면 `className` 유틸을 **전량 CSS로 번역**해야 한다(이식 난이도 HIGH). 직전 세션이 "흉내"에 그친 근본 원인도 이것 — 정합하는 복사 대상이 없어 억지로 맞추다 근사가 됐다.

여기서 스택 질문이 떠올랐다: **우리도 Tailwind를 도입하면 이 마찰이 사라지나?** CSS 실측 결과 keeper CSS가 거의 없어(아래 근거) 전환 비용이 지금이 최저점임이 확인됐다.

## 결정
프론트 스타일링을 **Tailwind CSS v4 + shadcn/ui(Radix 기반, 필요분만) + lucide-react(아이콘)** 로 전환한다.
- `data-theme`(dark/light/e-ink) **3종 테마는 그대로 유지** — CSS 변수를 Tailwind 색 토큰에 매핑해(`bg-background` → `var(--...)`) 테마 전환 메커니즘을 바꾸지 않는다.
- 기존 keeper 기반 CSS(~20줄)는 Tailwind 체계로 흡수, `lab/` 스크래치 CSS(layouts.css 등)는 폐기 대상.
- 그 위에 채팅 UI는 **Claude Code 룩을 네이티브로 직접 구현**한다 — 어떤 OSS도 그 룩을 안 주므로 이건 "복붙"이 아니라 우리 설계다(참조: claude-code-webui = 데이터 매핑 참조, Continue `ThinkingBlockPeek` = "Thought for Xs" 문구 참조).

이는 CLAUDE.md 기술 스택의 `CSS 변수(Tailwind X)` 기조를 **전환**하는 결정이다(정식 선행 ADR은 없었음 → supersede 대상 없이 신규 박제).

## 거부한 대안
- **순수 CSS 유지(기존 기조)** — keeper CSS가 ~20줄뿐이라 전환 비용이 *지금이 최저점*이고, shadcn 생태계 없이 폴리시·속도를 손으로 재발명하는 비용이 장기적으로 크다. 이식 대상 OSS가 전부 Tailwind라 상호운용성도 손해. (10년 유지 전제 + 저위험·장기 = over-engineering 허용 판단, CLAUDE.md 아키텍처 §0.)
- **OSS(Cline 등) 통째 이식 — 순수 CSS로 번역** — 결과물이 "Cline 룩"이라 CC 벤치마크에 **영구 미달**(장황한 "Cline wants to…" 헤더·점선 타임라인 없음·"Thought for Ns" 없음) + 번역 비용 HIGH.
- **claude-code-webui 포크(스켈레톤)** — 같은 claude stream-json을 렌더하지만 **아카이브됨(2026-05, ~1.1k star)·룩은 일반 웹챗**(CC 점선 타임라인/Thought-for-Ns 없음 — 적대 리뷰가 "최고 충실도" 주장을 과장으로 판정)·**Tailwind라 어차피 CSS 재작성**. 우리 백엔드가 이미 구조화 아이템을 방출하므로 그 파서 이득도 작다.
- **siteboon/claudecodeui 등** — 적대 리뷰에서 **AGPL-3.0**(카피레프트)로 확인 → 우리 앱 이식에 부적합.

## 근거
- **/research(medium) + Codex(cross-family, medium) 적대 리뷰** — grounding으로 (a) CC 확장 폐쇄소스 확정 (b) 후보 4종 Tailwind 강결합·CC 룩 부재 (c) claude-code-webui 과장·siteboon AGPL 정정.
- **CSS 실측** — 전체 763줄 중 `lab/` 스크래치 394 + 재작성 대상 채팅 렌더 348 = **keeper 기반 CSS ~20줄**(theme/font/index/App). 전환 비용 최저점.
- **성능 무해** — Tailwind = 빌드타임 정적 CSS 생성(런타임 JS 오버헤드 0). 후보 일부가 쓰는 styled-components(런타임 비용 有)보다 오히려 가볍고, 미사용 클래스는 tree-shake로 번들에서 제외.

## 영향 / 불변식
- **CLAUDE.md "기술 스택(프론트)"** `… CSS 변수(Tailwind X) …` → `Tailwind CSS v4 + shadcn/lucide + CSS 변수 테마` 로 갱신(이 ADR과 한 묶음).
- **테마 불변** — `data-theme` 3종(dark/light/e-ink)은 CSS 변수로 유지하고 Tailwind 색 토큰이 그 `var()`를 참조한다. **테마 전환 메커니즘을 바꾸지 않는다**(Tailwind `dark:` variant 단독으로 3종을 표현하지 않는다).
- **채팅 UI 룩 = 네이티브 설계** — CC 룩은 복사 소스가 없으므로 우리가 디자인한다. OSS는 데이터 매핑·문구 참조에 한정(코드 복붙 아님 → 라이선스 법무 트리거 없음).
- **§5 LLM 제어 불변식과 무관** — 스타일링 체계 변경은 제어 표면(command 버스·invoke)에 영향 없음.
- **후속(이 ADR 실행분)** — Tailwind v4 + Vite 플러그인 셋업 · 테마 변수↔토큰 매핑 · lucide/shadcn 도입 · `StructuredTextView` 후신 재구현. load-bearing 코드에 `// ADR-0047` 앵커.
