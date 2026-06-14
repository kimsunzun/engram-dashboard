# ADR-0011: agentClient 제어 표면 facade (데몬 대비)

- 상태: 확정 (프론트 통합, S12 대비)
- 관련: CLAUDE.md §5·프론트 파일 구조 · `api/{agentClient,embeddedClient,clientFactory}.ts`

## 맥락
데몬화 시 프론트가 `invoke`/Channel을 직접 호출하면 전송 경로(in-process ↔ WebSocket)를 갈아끼울 수 없다. 또 §5(LLM-우선 제어)는 모든 동작이 LLM이 닿는 단일 제어 표면을 갖길 요구한다.

## 결정
컴포넌트·스토어는 **`agentClient` 인터페이스만** 의존한다(ptyApi 직접 호출 금지). 현재 구현은 `embeddedClient`(in-process, invoke/Channel 캡슐화). `clientFactory`가 싱글톤을 만들고 `window.__ENGRAM_AGENT__`로 노출(§5 제어 표면). 데몬 단계의 `DaemonClient`(WS)는 **동일 인터페이스**로 추가.

## 거부한 대안
- **ptyApi invoke 직접 호출 산재** — 데몬 전환 시 모든 호출처를 전면 수정. facade swap 불가.

## 근거
facade 한 곳만 교체하면 in-process ↔ daemon 전환 흡수(데몬화=facade swap, 갈아엎기 X). window 노출로 사람 클릭과 LLM 호출이 같은 진입점.

## 영향 / 불변식
- **컴포넌트·스토어에서 ptyApi 직접 호출 금지** — 반드시 agentClient 경유.
- DaemonClient는 새 인터페이스를 만들지 말고 agentClient를 구현한다.
