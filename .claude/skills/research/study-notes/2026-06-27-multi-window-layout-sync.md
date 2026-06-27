# study-note: 멀티 창 레이아웃 동기화 — 2026-06-27

주제: 멀티 창 간 레이아웃 상태 동기화 OSS 구현 패턴
강도: deep

## 쟁점과 해결 과정

### 쟁점 1: BroadcastChannel이 Tauri에서 작동하는가?
초기 가정은 "웹 표준이라 작동 안 할 수 있다"였다. state-sync 비교표 확인 결과 zustand-sync-tabs(BroadcastChannel 기반)이 Tauri에서 실제로 동작하지만, Rust 백엔드가 상태 변화를 인식하지 못하는 한계가 있음이 확인됐다. → "작동하지만 한계 있음"으로 수정.

### 쟁점 2: 독립 레이아웃 트리 vs 공유 트리 서브뷰 — Tauri에서 실제 선택지?
조사 전에는 두 방식이 모두 가능한 선택지라고 생각했다. 그러나 Tauri는 `window.open`으로 서브 창을 생성하면 별도 JS 컨텍스트가 강제로 분리되므로 "공유 JS 메모리 트리" 방식이 아예 불가능함이 확인됐다. → 실질적 선택지는 A(독립 트리 + Rust authority 동기화) vs B(Rust가 트리 전체 보유)로 좁혀진다.

### 쟁점 3: Tauri unlisten 자동 정리 가능 여부
Claude가 Issue #15583을 발견(2026-06-25 등록). Codex는 이를 미언급. 직접 GitHub 확인으로 "미해결 버그"임을 확인. → cleanup 패턴 설계에 중요한 실무 영향.

## 검색 전략 메모
- 초반 WebSearch로 넓게 → WebFetch로 공식 문서·GitHub issue 직접 확인 순서가 효과적.
- state-sync 라이브러리 비교표(777genius.github.io)가 프레임워크 간 비교에 매우 유용했음.
- Codex가 VS Code 소스코드 패턴(storageIpc.ts, sharedProcess.ts)을 직접 인용해 구체적 코드 패턴 확보에 기여.

## deep vs medium 차이 체감
- deep: WebFetch로 실제 GitHub issue·공식 문서를 직접 확인해 버그(#15583)와 주의사항(#5288)을 발견. medium이었으면 "onCloseRequested 쓰면 된다"로 끝났을 것.
- 적대 검증 덕에 BroadcastChannel 한계와 revision gate 패턴의 구체적 메커니즘까지 확인.
