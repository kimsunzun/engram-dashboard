# ADR-0050: 채팅 렌더 = 자체 구현 (Cline 포트 제거) + Claude Code VSCode 확장 시각 벤치마크

- 상태: 확정 (2026-07-06, 근거: 사용자 벤치마크 재교정 결정 + 구현물 /review code·/qa(cdp 실측) 게이트 통과)
- 관련: Supersedes ADR-0048 (Cline 잎 포트 + Apache-2.0 귀속) · ADR-0047(Tailwind 채택 존속, 자체 구현 전제 복원) · `src/components/slot/chat/`(신규 렌더러) · `src/components/slot/StructuredTextView.tsx`(dispatch) · `src/components/slot/RichSlot.tsx`(전송 Enter-only) · `src/components/slot/structuredAccumulator.ts`(StructuredItem 모델·무변경) · step-log 2026-07-06

## 맥락
ADR-0048은 채팅 렌더를 "Cline 잎 컴포넌트 verbatim 포트 + 우리 dispatch + react-markdown 스택 + Apache-2.0 귀속"으로 정했다. 그 전제는 (a) Cline 코드를 실제로 복사해 룩을 얻고 (b) 그 대가로 Apache-2.0 §4 귀속 의무(LICENSE 사본 + 파일 헤더 + THIRD_PARTY_NOTICES)를 진다는 것이었다.

착수 후 벤치마크가 스크린샷 비교로 재교정됐다(사용자 결정 2026-07-06): 사용자가 원하던 룩은 **Cline이 아니라 Claude Code VSCode 확장**임이 확정됐다. 이로써 Cline 포트의 존재 이유가 소멸했다 — Cline 코드를 끌고 다녀도 목표 룩(Claude Code 확장)에 가까워지지 않고, 오직 Apache-2.0 귀속 부담만 남는다. Claude Code 확장은 폐쇄소스(All Rights Reserved)라 코드 포트가 불가능하므로, 룩 도달 경로는 **스크린샷 실측 벤치마킹**뿐이다.

## 결정
채팅 렌더를 **Cline 포트를 완전히 제거한 자체 구현**으로 재작성하고, 시각 목표를 **Claude Code VSCode 확장(스샷 실측 벤치마크)**으로 고정한다.

- **신규 `src/components/slot/chat/` 자체 구현:**
  - `Markdown.tsx` — `react-markdown` + `remark-gfm` + `remark-math` + `rehype-highlight` + `rehype-katex`(+katex css). zero-width(U+200B류) 소독(스트리밍이 펜스 앞에 흘림), 단일 `ReactMarkdown`(블록 쪼개기 금지), 언어 별칭 정규화, `pre` hover 복사버튼, 래퍼 `.chat-markdown`.
  - `ThoughtRow.tsx` — 접힘 "Thought" 라벨 + 셰브런. **내용이 비어도(암호화 thinking) 라벨 행 표시**(비상호작용, tooltip "내용 비공개"). 내용 있으면 토글, streaming이면 pulse.
  - `CopyButton.tsx` — lucide Copy→Check. `ui/button`·cva·radix **무의존**(순수 button).
  - `chat.css` — 1차 근사 타이포/간격값(확장 정밀 매칭 전 base).
- **Cline 포트·귀속 완전 제거:** `slot/cline/*`(5파일) · `ui/button.tsx` · 귀속 3종(`THIRD_PARTY_NOTICES.md` · `LICENSES/cline-Apache-2.0.txt` · 복사 파일 헤더) 삭제.
- **KaTeX 수식 렌더 추가**(remark-math + rehype-katex) — `$$…$$` 생문자열 노출 해소.
- **전송 버튼 제거 → Enter-only.** RichSlot 입력바 전폭, "메시지 입력 (Enter 전송 · Shift+Enter 줄바꿈)".
- **진행 방식 = 1차 근사 후 반복.** base(엔진·기능)를 먼저 세워 커밋하고, 확장 룩 정밀 매칭은 사용자 스샷 반복으로 후속 조정한다.

## 거부한 대안
- **Cline 포트 유지 + 이름만 우리 것으로 변경** — 실질 3~400줄 렌더 코드에 Apache-2.0 귀속(LICENSE 사본·파일 헤더·NOTICES)을 영구히 끌고 다닐 가치가 없다. 벤치마크가 Cline→Claude Code 확장으로 바뀌어 Cline 포트의 존재 이유 자체가 소멸했다. (이 ADR이 폐기하는 ADR-0048의 결정.)
- **Claude Code 확장 코드 직접 포트** — 폐쇄소스(All Rights Reserved), 복사 불가. 그래서 스샷 실측 벤치마킹만이 룩 도달 경로다.
- **marked / styled-components 재도입** — Cline 스택이 끌어오던 것이나 자체 구현엔 불필요. `marked`는 react-markdown 경로로 대체되어 제거, styled-components는 Tailwind 기조(ADR-0047) 위반이라 배제.
- **확장 룩을 스샷 벤치마크 없이 "느낌"으로 근사** — ADR-0047 네이티브 직접 구현이 반복 근사에 그친 실패 모드. 소스가 없으니 스샷 실측을 벤치마크로 고정해 어긋남을 잡는다.

## 근거
- **사용자 벤치마크 재교정(2026-07-06)** — 오전 Cline 충실도 작업(82a8108·8b844ed) 후 스샷 비교로 "목표 룩 = Claude Code 확장"을 확정. Cline 포트 완전 재작성 + 귀속 삭제를 사용자가 직접 선택.
- **구현물 게이트 통과** — `/review code`(doc-aware) PASS(가드 무손상·테스트 non-vacuous) + `/qa`: vitest 242/242 · `tsc --noEmit` 0 · BOM clean · **cdp 실측**(마크다운·KaTeX 박스·코드 하이라이트·hover 복사버튼·전송버튼 제거 라이브 확인).
- **결정의 옳음은 이 ADR이 보장하지 않는다** — 시각 정밀 매칭은 미완(1차 근사). 확장 룩 도달은 후속 스샷 반복으로 검증한다.

## 영향 / 불변식
- **ADR-0048 전체 폐기** — Cline verbatim 포트 + Apache-2.0 귀속 결정 소멸. **라이선스 트리거 해소** — Cline 실코드가 코드베이스에 없으므로 Apache-2.0 §4 의무가 사라지고 귀속 3종을 삭제한다.
- **ADR-0047 관계** — 0048이 0047의 "네이티브 직접 구현 / OSS 참조 한정" 조항을 개정했으나, 0048 폐기로 그 개정이 되돌려져 **0047의 자체 구현 전제가 복원**된다. 이 ADR은 거기에 "Claude Code 확장 = 명시적 스샷 벤치마크"를 더한다. (0047의 Tailwind v4 + shadcn/lucide·테마 CSS변수 채택은 존속.)
- **데이터 모델 불변** — `StructuredItem` 순서 보존 스트림(ADR-0045)·구독·누산·send 데이터흐름(ADR-0044/45/46) 무변경. `structuredAccumulator`(특히 `Date.now` 금지 — replay 멱등 불변식) 불가침. 이 라운드는 dispatch/렌더만 교체.
- **빈(암호화) thinking 행 표시** — opus-4-8의 thinking은 암호화(평문 빈 문자열)라는 업스트림 동작이라, 내용이 비어도 "Thought" 라벨 행을 표시한다(비상호작용). sonnet은 평문 정상. (재디버깅 금지 — 업스트림 동작.)
- **전송 = Enter-only(버튼 제거)** — RichSlot의 Enter/Shift+Enter/IME 가드(`isComposing||229`)·`send()` await-catch는 load-bearing 불변(회귀테스트가 Enter 경로로 이 동작을 지킨다). 버튼 제거가 이 로직을 바꾸지 않는다.
- **의존성 델타(변경 보고 대상)** — 추가 `katex`·`remark-math`·`rehype-katex` / 제거 `class-variance-authority`·`@radix-ui/react-slot`·`unist-util-visit`·`marked`.
- **앵커** — 채팅 렌더 dispatch/leaf load-bearing 파일에 `// ADR-0050` 앵커. `StructuredTextView.tsx`·그 테스트에 mislabel돼 있던 `// ADR-0049`(dispatch는 0048→0050 소관, 0049는 백엔드 thinking-token 주입)를 0050으로 교정. (ThoughtRow의 "암호화 thinking 근거절" 0049 인용은 정당 — 유지.)
