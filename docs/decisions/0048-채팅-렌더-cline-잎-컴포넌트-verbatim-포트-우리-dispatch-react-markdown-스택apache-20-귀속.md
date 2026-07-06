# ADR-0048: 채팅 렌더 = Cline 잎 컴포넌트 verbatim 포트 + 우리 dispatch (react-markdown 스택·Apache-2.0 귀속)

- 상태: **폐기 (Superseded by ADR-0050)** — 벤치마크가 스샷 비교로 Cline→Claude Code VSCode 확장으로 재교정(사용자 결정 2026-07-06)되어 Cline 포트의 존재 이유가 소멸, Apache-2.0 귀속 부담만 남음 → 포트·귀속 제거하고 자체 구현으로 재작성. ~~확정 (2026-07-05, 근거: Cline 정찰·귀속 감사(서브에이전트) + 사용자 결정 · 구현은 후속 /review code + /qa 게이트)~~
- 관련: Amends ADR-0047 (채팅 UI 렌더 방식: CC룩 네이티브 직접 구현·OSS 참조한정(코드 복붙 아님) → Cline 잎 컴포넌트 verbatim 코드 포트(Apache-2.0 귀속)) · `src/components/slot/StructuredTextView.tsx`(dispatch 후신) · `src/components/slot/structuredAccumulator.ts`(StructuredItem 모델) · Cline 클론 `I:\Engram_Workspace\references\cline` · step-log 2026-07-05

## 맥락
ADR-0047은 채팅 UI를 "Claude Code(CC) 룩 **네이티브 직접 구현**, OSS는 데이터매핑·문구 **참조 한정**(코드 복붙 아님 → 라이선스 트리거 없음)"으로 정했다. 그 근거는 (a) CC 확장은 폐쇄소스라 복사 대상이 없고 (b) OSS 후보는 CC 시그니처 룩을 안 가지며 (c) 당시 우리 스택이 순수 CSS라 OSS(전부 Tailwind) 이식 비용이 HIGH라는 것이었다.

그러나 네이티브 직접 구현은 반복해서 "근사(approximation)"에 그쳤고 사용자가 반복 지적했다(손수 만든 아이콘·아코디언·타임라인이 진짜 룩과 계속 어긋남). 동시에 ADR-0047의 거부 근거 두 개가 무너졌다:
1. **Tailwind 도입(ADR-0047 본체)이 "번역 비용 HIGH" 근거를 스스로 무력화** — 이제 Cline의 Tailwind/shadcn/lucide 코드를 CSS 재작성 없이 그대로 옮길 수 있다.
2. **벤치마크 완화(사용자 결정)** — "CC 룩에 영구 미달"이 Cline 이식의 거부 사유였으나, 사용자가 "내 손디자인 CC 근사보다 실제 Cline 코드가 낫다"로 목표를 재설정했다.

착수 전 Cline 채팅 컴포넌트를 정찰·귀속 감사한 결과, Cline 채팅의 **최상위 dispatch 컴포넌트 `ChatRow.tsx`(1208줄)는 verbatim 복사 불가**임이 확인됐다 — VSCode 전용 gRPC 클라이언트(`FileServiceClient`/`UiServiceClient`)·`useExtensionState` 컨텍스트·`@shared/proto/*`에 통째로 묶여 우리 데이터로 떼어낼 수 없다. 복사 가능한 것은 콘텐츠를 그리는 **잎(leaf) 프레젠테이션 컴포넌트**뿐이다.

## 결정
채팅 렌더를 **Cline의 verbatim 복사 가능한 잎 컴포넌트 포트 + 우리가 작성하는 dispatch**로 구현한다.

- **verbatim 복사(VSCode 배선만 제거):** `chat/ThinkingRow`, `chat/MarkdownRow`, `common/MarkdownBlock`(`useExtensionState` Plan/Act 콜아웃 + `FileServiceClient` 파일존재 콜아웃 2곳만 제거), `common/CopyButton`, `ui/button`, `chat/ExpandHandle`.
- **적응 포트(verbatim 아님):** `common/CodeAccordian`(codicon 아이콘 span → lucide, styled-components `CodeBlock` 대신 `MarkdownBlock` 강조 경로 사용).
- **dispatch는 우리가 작성** — `ChatRow`를 못 베끼므로, 우리 `StructuredItem[]` 순서 보존 스트림(text/tool/usage/error/structured/separator + user)을 위 잎 컴포넌트에 매핑하는 dispatch(`StructuredTextView` 후신)를 직접 짠다. 레이아웃 배치도 우리 소관이다.
- **react-markdown 스택 도입** — Cline `MarkdownBlock`을 verbatim으로 쓰려면 `react-markdown` + `remark-gfm` + `rehype-highlight` + `unist-util-visit` + `marked`가 필요하다. 버튼은 `@radix-ui/react-slot` + `class-variance-authority`. (기존 lab 미니 markdown 파서는 대체.)
- **styled-components 도입 금지** — Cline `CodeBlock`/`MermaidBlock`이 끌어오나 ADR-0047 Tailwind 기조 위반이라 배제.
- **Apache-2.0 귀속 부착** — Cline은 Apache-2.0(저작권자 `Cline Bot Inc.`)이고 **NOTICE 파일이 없다**(감사 확인). 필요분: ① LICENSE 전문 사본(`LICENSES/cline-Apache-2.0.txt`) ② 복사/적응 파일마다 "Cline 원본 · Apache-2.0 · Modified by" 헤더(§4b). NOTICE는 Cline에 없으므로 불필요.

## 거부한 대안
- **CC 룩 네이티브 직접 구현(ADR-0047 원결정)** — 반복해서 근사에 그쳐 사용자가 반복 지적. 진짜 룩과 계속 어긋나 품질 미달. (이 ADR이 개정하는 대상.)
- **Cline `ChatRow`까지 전체 재구성** — 최상위 dispatch/레이아웃을 Cline 구조 기반으로 재건. verbatim이 아닌 대공사이고 VSCode gRPC를 전부 걷어내야 하며 근사로 회귀할 위험. 잎 포트로 실제 렌더 내부(markdown·thinking·code)를 이미 얻으므로 비용 대비 이득이 낮다.
- **Claude Code 확장 직접 복사** — 폐쇄소스(All Rights Reserved). 복사 불가(ADR-0047에서 확정).
- **styled-components 기반 Cline `CodeBlock` 복사** — Tailwind 기조(ADR-0047) 위반 + 런타임 비용. `MarkdownBlock`의 rehype-highlight 경로로 대체 가능.
- **기존 lab 미니 markdown 파서 유지** — 제목·문단·리스트·굵게/코드펜스만 지원(GFM 표·중첩·인용문 없음, 스파이크 품질). "실제 Cline" 목표와 어긋나고 렌더 완성도 부족.

## 근거
- **Cline 정찰·귀속 감사(서브에이전트)** — 복사 가능/적응/스킵 파일 분류, 각 파일 라이선스 헤더 유무(전부 무헤더), **Cline NOTICE 부재 확인**(핸드오프 미확인 항목 해소), LICENSE = Apache-2.0/`Cline Bot Inc.`, 의존성 델타(위 5+2개), StructuredItem→컴포넌트 props 매핑 초안.
- **사용자 결정(2026-07-05)** — "충실 포트": 잎 부품 verbatim + 우리 dispatch, react-markdown 스택 수용.
- **ADR-0047 재검토** — Tailwind 채택이 "이식=CSS 재작성" 마찰을 제거했고, 벤치마크(CC 룩 도달)를 사용자가 완화 → 0047의 Cline-이식 거부 근거가 소멸.
- **결정의 옳음은 이 ADR이 보장하지 않는다** — 포트 구현물은 후속 `/review code` + `/qa`(cdp 실측) 게이트로 검증한다.

## 영향 / 불변식
- **ADR-0047 개정** — "채팅 UI 룩 = 네이티브 직접 구현 / OSS 참조 한정(코드 복붙 아님 → 라이선스 트리거 없음)" 조항 폐기. 나머지(Tailwind v4 + shadcn/lucide 스타일링 채택·테마 CSS변수 유지)는 존속.
- **라이선스 트리거 발생** — 이제 실코드를 복사하므로 Apache-2.0 §4 준수 의무(LICENSE 사본 + 변경 파일 "Modified by" 헤더 + 저작권 표시 유지)가 강제된다. 어기면 라이선스 위반. (법무 확인은 사용자가 공개 시점에 — 현재 개인 로컬 사용으로 연기, 사용자 결정.)
- **데이터 모델 불변** — `StructuredItem` 순서 보존 스트림(ADR-0045)은 바꾸지 않는다. 포트는 dispatch/렌더만 교체하고 구독·누산·send 데이터흐름(ADR-0044/45/46)은 무변경.
- **의존성 추가(변경 보고 대상)** — `react-markdown`·`remark-gfm`·`rehype-highlight`·`unist-util-visit`·`marked`·`@radix-ui/react-slot`·`class-variance-authority`. `styled-components`는 도입하지 않는다.
- **앵커** — 포트/dispatch load-bearing 파일에 `// ADR-0048` 앵커. 복사 파일은 Cline 귀속 헤더도 함께.
