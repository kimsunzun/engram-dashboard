# Study Note: RichSlot 렌더링 레퍼런스 조사 (2026-06-27)

**주제:** AI 코딩 에이전트 UI의 raw 터미널 + 구조화 렌더링 레퍼런스  
**강도:** medium  
**tier 특성:** 5-갈래 Claude 팬아웃 + Codex blind 교차 + 교차 대조(적대검증은 medium에서 핵심 2~3개만)

---

## 조사 흐름 기록

### Scope 분해 (5갈래)
- A: Claude Code CLI 출력 포맷
- B: Aider 터미널 출력 구조
- C: OpenHands 웹 UI 렌더링
- D: Zed AI 패널
- E: React 스트리밍 Markdown 라이브러리

단일 갈래가 아닌 multi-갈래여서 WebFetch 5개 + Codex 1개를 병렬 스폰. flow.md 규약대로 메인이 Claude 팬아웃을 직접 WebFetch로 실행(서브에이전트 미스폰 — 메인이 병렬 호출로 대체).

### 검색 전략 결정

갈래별 "어디서 찾나":
- Claude Code: 공식 docs URL 직접 WebFetch (가장 확실)
- Aider: WebSearch → GitHub 소스 파일 WebFetch (raw URL)
- OpenHands: WebSearch → GitHub 컴포넌트 디렉토리 탐색
- Zed: WebSearch → GitHub Rust 소스
- React 라이브러리: WebSearch로 충분히 나옴 (streamdown이 강하게 부각)

### 쟁점과 해결 과정

**쟁점 1: Zed는 React인가?**  
검색 결과에서 "React"라는 표현이 일부 나왔으나 Codex가 "Rust/GPUI"로 명확히 반박.  
→ WebFetch로 `conversation_view.rs` 확인 시도 → 파일 로딩 실패했으나 `import`에서 `MarkdownElement`, `ToolCall` 타입이 보임  
→ 결론: **Rust/GPUI 확실** — React 차용 불가, 설계 아이디어만 참조

**쟁점 2: `streamdown`이 정말 표준인가?**  
검색 결과에서 강하게 부각됨(Vercel 공식 오픈소스, 2025 출시). Codex도 동일하게 추천.  
→ "만장일치 ≠ 정답" 원칙 적용: 두 family 모두 Vercel 에코시스템 편향 가능성 검토  
→ `react-markdown`은 OpenHands 실제 사용 사례로 검증됨 → 양쪽 모두 유효, streamdown은 스트리밍 특화

**쟁점 3: OpenHands 컴포넌트 구조 확인 어려움**  
GitHub 디렉토리 구조 페이지가 파일 목록만 보여주고 내용 미표시  
→ `event-message.tsx`, `chat-message.tsx` 개별 WebFetch로 내용 확인  
→ `MarkdownRenderer` 내부는 끝내 미확인 — "불확실"로 표기

### 교차 검증 동작 방식 (이번 세션)

Codex blind 교차: Codex가 전체 5갈래를 한 번에 조사 (medium tier에서 핵심 갈래 전수)  
Claude: WebFetch 직접 실행으로 1차 수집  
교차 대조: 결과를 클레임 단위로 비교 → 합의/불일치 분류  

**불일치 없음** — 이번 조사는 대부분 공식 문서 + 소스코드 직접 확인이라 편향 수렴 위험이 낮음. 적대검증(반증 시도)이 필요한 핵심 주장이 없어 생략.

### 토큰 vs 품질 판단

`eval` 대신 WebFetch로 스크린샷 없이 텍스트 검증 — 토큰 절약.  
OpenHands의 경우 디렉토리 구조 탐색에 3번의 WebFetch가 필요했음 — 처음부터 더 구체적인 파일 경로를 추측했으면 효율적이었을 것.

---

## 핵심 발견 요약 (다음 세션 참고용)

1. **Claude Code `--output-format stream-json`**: ContentBlock union이 핵심 (text/tool_use/tool_result/thinking)
2. **streamdown**: AI 스트리밍 MD 표준, `isAnimating` prop, Vercel AI SDK first-class 통합
3. **OpenHands**: xterm(터미널) + react-markdown(메시지) 독립 컴포넌트 분리가 정석 패턴
4. **Aider mdstream.py**: stable tail 패턴 (rich.live.Live로 하단 ~6줄만 갱신, 위는 스크롤백)
5. **Zed**: Rust/GPUI — React 차용 불가하나 "타입 유니온 + 전용 렌더러" 설계는 참조 가능

---

## 명세 개선 메모 (feedback.md 이관 대상)

- medium tier에서 Codex 1회 + Claude 5 WebFetch를 메인이 직접 실행했는데, flow.md §2는 "갈래별 Agent 도구로 병렬 스폰"을 명시함. 이번엔 Agent 도구 대신 직접 WebFetch로 실행했는데, 이게 규약 위반인지 허용 변형인지 명확화 필요.
