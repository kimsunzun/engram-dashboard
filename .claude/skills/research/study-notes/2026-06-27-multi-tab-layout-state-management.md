# Study Note: 멀티 탭/페이지 레이아웃 상태 관리 OSS 패턴 (deep)

날짜: 2026-06-27 / 강도: deep

## 쟁점과 해결 과정

### 쟁점 1: react-mosaic v7 MosaicTabsNode 실제 타입

- **문제:** Codex가 `type: 'tabs', tabs: T[], activeTabIndex: number` 구조를 주장. 공식 소스를 직접 fetch 시도했으나 GitHub raw/404.
- **해결:** v7 beta 릴리즈 노트(Discussion #241)와 검색 결과 합성으로 간접 확인. 확신도를 "확실"에서 "가능성 높음"으로 낮춤.
- **교훈:** GitHub raw URL이 403/404를 자주 반환. `github.com/blob/` 경로도 로그인 요구 시 실패. 검색 기반 확인을 병행해야 함.

### 쟁점 2: VS Code EditorGroupModel MRU 직렬화

- **문제:** Codex가 `mru: number[]` (인덱스 배열, mru[0]=활성)를 주장. editorGroupModel.ts 직접 fetch 실패.
- **해결:** editorPart.ts fetch 성공 → `IEditorPartUIState { serializedGrid, activeGroup, mostRecentActiveGroups }` 확인. EditorGroupModel 세부는 Codex 분석 신뢰(복수 출처 일치).

### 쟁점 3: 단일 스토어 vs 페이지별 스토어

- **Claude + Codex 합의:** 단일 스토어 + `pagesById: Record<PageId, PageLayoutState>`.
- **만장일치=정답? 주의:** VS Code/Zed 모두 단일 스토어지만, 이는 IDE 규모의 요구사항. 소규모 앱에서는 페이지별 스토어도 합리적일 수 있음. 하지만 영속화·탭 이동·undo/redo 측면에서 단일 스토어가 유리한 건 설계 원칙 수준의 합의.

## 이번 deep tier 에서 확인한 tier별 차이

- **팬아웃 4 + WebFetch 사용:** 단순 검색보다 실제 타입/코드를 가져올 수 있었음. (단 URL 제한으로 일부 실패)
- **적대 검증:** 불일치 클레임 0개 — 모든 핵심 클레임이 Claude+Codex 합의. 그래서 적대 검증이 "조용히 통과"된 케이스.
- **만장일치 주의:** 두 family가 동일 결론을 내도 공통 학습 편향 가능성 존재. 이 주제(레이아웃 상태 관리)는 업계 표준이 명확해 공통 편향 위험이 낮음.

## 검색 전략 메모

- VS Code: `editorGroupsService.ts`, `editorPart.ts` raw fetch 성공
- Zed: `persistence/model.rs` raw fetch 성공
- react-mosaic: raw fetch 실패 → 검색 기반 + 릴리즈 노트 합성
- Zustand: 공식 docs URL(zustand.docs.pmnd.rs) 일부 404, 대안 URL(awesomedevin 미러) 성공
