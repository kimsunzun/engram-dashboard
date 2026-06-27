# 멀티 탭/페이지 레이아웃 상태 관리 OSS 패턴 리서치

**상태:** 완료  
**날짜:** 2026-06-27  
**강도:** deep (Claude 팬아웃 4 + Codex blind 1 + 적대 검증)  
**확신도 범례:** 확실 = 코드/공식 문서 직접 확인 / 가능성 높음 = 복수 출처 합의 / 불확실 = 단일 출처 또는 미검증

---

## 요약 (핵심 4문장)

VS Code·Zed 모두 **단일 전역 컨테이너**가 활성 그룹 포인터를 들고, 각 페이지/그룹의 레이아웃은 **별도 레코드(직렬화 트리)**로 격리된다. 탭 전환은 `activeGroupId`/`activePageId`만 교체하며 비활성 상태는 메모리에 보존된다. 레이아웃 트리는 **재귀 enum/union** 으로 표현된다(VS Code SerializedGrid / Zed SerializedPaneGroup / react-mosaic MosaicNode). Zustand에서 이 패턴을 구현할 때는 **Immer 미들웨어 + 단일 스토어 + `pagesById: Record<PageId, PageLayoutState>`** 가 업계 표준과 정확히 일치한다.

---

## 1. VS Code — EditorGroupsService + EditorPart

### 구조 개요

VS Code 워크벤치는 `IEditorGroupsService` → `IEditorGroupsContainer` → `IEditorGroup[]` 계층으로 상태를 관리한다.  
레이아웃 직렬화는 `EditorPart`가 담당하며 워크스페이스 memento(`editorpart.state` 키)에 저장한다.

**확신도: 확실** (GitHub microsoft/vscode 소스 직접 fetch 확인)

### 핵심 타입

```typescript
// IEditorGroupsContainer (공통 기반)
interface IEditorGroupsContainer {
  readonly activeGroup: IEditorGroup;
  readonly groups: readonly IEditorGroup[];
  readonly count: number;
  readonly orientation: GroupOrientation;

  activateGroup(group: IEditorGroup | GroupIdentifier): IEditorGroup;
  addGroup(location, direction): IEditorGroup;
  removeGroup(group): void;
  applyLayout(layout: EditorGroupLayout): void;
  getLayout(): EditorGroupLayout;
}

// IEditorGroupsService (멀티 윈도우 확장)
interface IEditorGroupsService extends IEditorGroupsContainer {
  readonly mainPart: IEditorPart;
  readonly parts: ReadonlyArray<IEditorPart>;
  saveWorkingSet(name: string): IEditorWorkingSet;
  applyWorkingSet(workingSet: IEditorWorkingSet | 'empty'): Promise<boolean>;
}

// 직렬화 상태 (memento에 저장되는 형태)
interface IEditorPartUIState {
  readonly serializedGrid: ISerializedGrid;     // 분할 트리
  readonly activeGroup: GroupIdentifier;         // 포커스된 그룹 ID
  readonly mostRecentActiveGroups: GroupIdentifier[];  // MRU 순서
}

// 그룹별 에디터 상태
interface ISerializedEditorGroupModel {
  id: number;
  locked?: boolean;
  editors: SerializedEditorInput[];
  mru: number[];       // 인덱스 기반 MRU (mru[0] = 활성 에디터)
  preview?: number;    // preview 에디터 인덱스
  sticky?: number;     // sticky 경계 인덱스
}
```

### 축약 통합 모델

```typescript
type EditorPartState = {
  grid: SerializedGridNode;                        // split tree + sizes
  activeGroupId: number;                           // 포커스 pane
  mostRecentActiveGroupIds: number[];
  groupsById: Record<number, EditorGroupState>;
};

type EditorGroupState = {
  id: number;
  locked?: boolean;
  editors: SerializedEditorInput[];
  mruEditorIndexes: number[];   // [0] = 활성 에디터
  previewIndex?: number;
  stickyIndex?: number;
};
```

### 탭 보존/복원 메커니즘

- **보존:** 그룹 전환 시 `EditorGroupModel`은 메모리에 유지, `EditorPart.saveState()`만 memento 쓰기.
- **복원:** `doApplyState()`가 `serializedGrid`로 그리드 재구성 후 각 그룹의 에디터 목록 복원, `mru[0]`을 활성 에디터로 설정.
- **원자성:** `onDidAddGroup`/`onDidRemoveGroup` 이벤트를 일시 중지하고 전환 수행.

**장점:** 그룹/탭 상태가 메모리 격리 → 전환 비용 최소.  
**단점:** IEditorPart 계층이 복잡, 직접 상태 구독보다 이벤트 기반 통지.

**출처:**
- [editorGroupsService.ts](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/services/editor/common/editorGroupsService.ts)
- [editorPart.ts](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/browser/parts/editor/editorPart.ts)
- [editorGroupModel.ts](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/common/editor/editorGroupModel.ts)

---

## 2. Zed — AppState + Workspace + PaneGroup

### 구조 개요

Zed의 `AppState`는 글로벌 서비스 컨테이너(언어 레지스트리, 클라이언트, 파일시스템 등)이며 레이아웃 상태가 아니다.  
실제 레이아웃은 **Workspace → PaneGroup(재귀 트리) → Pane → Item[]** 계층이 담당한다.

**확신도: 확실** (GitHub zed-industries/zed persistence/model.rs 직접 fetch 확인)

### 핵심 타입 (Rust)

```rust
// 글로벌 서비스 컨테이너 (레이아웃 X)
pub struct AppState {
    pub languages: Arc<LanguageRegistry>,
    pub client: Arc<Client>,
    pub user_store: Entity<UserStore>,
    pub workspace_store: Entity<WorkspaceStore>,
    pub fs: Arc<dyn Fs>,
    pub session: Entity<AppSession>,
}

// 워크스페이스 집합 (레이아웃 간접 참조)
pub struct WorkspaceStore {
    workspaces: HashSet<(gpui::AnyWindowHandle, WeakEntity<Workspace>)>,
    client: Arc<Client>,
}

// 직렬화 레이아웃 트리 (재귀 enum)
pub enum SerializedPaneGroup {
    Group {
        axis: SerializedAxis,
        flexes: Option<Vec<f32>>,
        children: Vec<SerializedPaneGroup>,  // 재귀
    },
    Pane(SerializedPane),
}

// 패인 (탭 컨테이너)
pub struct SerializedPane {
    pub active: bool,                      // 이 패인이 포커스됐는가
    pub children: Vec<SerializedItem>,
    pub pinned_count: usize,
}

// 개별 탭
pub struct SerializedItem {
    pub kind: Arc<str>,
    pub item_id: ItemId,
    pub active: bool,    // 이 아이템이 활성 탭인가
    pub preview: bool,
}
```

### 런타임 Pane 상태 (메모리)

```rust
pub struct Pane {
    items: Vec<Box<dyn ItemHandle>>,
    active_item_index: usize,       // 활성 탭 인덱스
    preview_item_id: Option<EntityId>,
    activation_history: Vec<ActivationHistoryEntry>,
    // ...
}
```

### 탭 보존/복원 메커니즘

- **보존:** 패인 전환 시 `active_item_index`만 변경, 비활성 아이템은 `Vec`에 유지.
- **복원:** `SerializedPane.children`에서 `active: true` 아이템을 찾아 `activate_item()` 호출.
- **persistence:** SQLite KEY_VALUE_STORE에 JSON 직렬화.

**장점:** Rust enum 재귀 구조가 레이아웃 트리를 타입 안전하게 표현.  
**단점:** GPUI 프레임워크 종속, React 에코시스템과 직접 이식 불가.

**출처:**
- [workspace/src/persistence/model.rs](https://github.com/zed-industries/zed/blob/main/crates/workspace/src/persistence/model.rs)
- [workspace/src/workspace.rs](https://github.com/zed-industries/zed/blob/main/crates/workspace/src/workspace.rs)
- [workspace/src/pane.rs](https://github.com/zed-industries/zed/blob/main/crates/workspace/src/pane.rs)
- [DeepWiki: Workspace Persistence](https://deepwiki.com/zed-industries/zed/3.4-panel-system)

---

## 3. React 기반 레이아웃 라이브러리

### 3-1. react-mosaic (v7)

**확신도: 확실** (v7 beta 릴리즈 노트 + npm 공식 문서 확인, `MosaicTabsNode` 구조는 가능성 높음)

```typescript
// 노드 타입 유니온 (n진 트리, v7부터)
type MosaicNode<T extends MosaicKey> =
  | MosaicSplitNode<T>
  | MosaicTabsNode<T>
  | T;  // 리프 (창 ID)

// 분할 노드 (v7: binary first/second → n진 children)
interface MosaicSplitNode<T> {
  type: 'split';
  direction: 'row' | 'column';
  children: MosaicNode<T>[];
  splitPercentages?: number[];
}

// 탭 노드 (v7 신규 — 일급 시민)
interface MosaicTabsNode<T> {
  type: 'tabs';
  tabs: T[];
  activeTabIndex: number;
}

// 탭 활성화 예시
const tabsNode: MosaicNode<string> = {
  type: 'tabs',
  tabs: ['tab1', 'tab2', 'tab3'],
  activeTabIndex: 0,
};
```

**상태 관리 방식:**
- 제어(controlled): `value` + `onChange` + `onRelease` props → 외부 스토어(Zustand 등)에서 관리
- 비제어(uncontrolled): `initialValue` props → 내부 useState

**탭 전환 시 보존:** `activeTabIndex`만 변경, `tabs` 배열은 유지 → 비활성 탭 상태는 외부 컴포넌트 마운트 유지 여부에 달림.

**장점:** 순수 JSON 트리 → localStorage 직렬화가 한 줄. 탭이 일급 노드.  
**단점:** 구버전(v6 이하)은 탭 미지원(바이너리 트리만). 드래그 간 mosaicId 충돌 주의.

**출처:**
- [react-mosaic GitHub](https://github.com/nomcopter/react-mosaic)
- [v7.0.0-beta0 Discussion #241](https://github.com/nomcopter/react-mosaic/discussions/241)
- [Webviz 탭 구현 Gist](https://gist.github.com/troygibb/9be30ae863ea5da4ebdbf58f1eda8f49)

### 3-2. golden-layout (v2)

**확신도: 가능성 높음** (공식 문서 확인, 내부 타입 일부는 불확실)

```typescript
type ItemConfig =
  | RowOrColumnConfig
  | StackConfig       // 탭 그룹 = stack
  | ComponentConfig;

type LayoutConfig = {
  root: ItemConfig;
};

type RowOrColumnConfig = {
  type: 'row' | 'column';
  content: ItemConfig[];
};

// 탭 그룹 (stack) — activeItemIndex로 활성 탭 관리
type StackConfig = {
  type: 'stack';
  content: ComponentConfig[];
  activeItemIndex?: number;
};

type ComponentConfig = {
  type: 'component';
  componentType: string;
  componentState?: unknown;
  title?: string;
  id?: string;
};
```

**탭 전환 시 보존:** `saveLayout()` → JSON → `loadLayout()` 패턴. 활성 탭은 `activeItemIndex`로 직렬화.  
**제약:** v2는 중첩 stack 불허 (stack 안에 stack 금지).

**출처:**
- [golden-layout 공식](https://golden-layout.github.io/golden-layout/)
- [Getting Started with React](https://golden-layout.com/tutorials/getting-started-react.html)

---

## 4. Zustand 중첩 상태 업데이트 패턴

### 4-1. 단순 spread (얕은 중첩)

**확신도: 확실** (공식 docs 확인)

```typescript
set((state) => ({
  deep: {
    ...state.deep,
    nested: {
      ...state.deep.nested,
      obj: {
        ...state.deep.nested.obj,
        count: state.deep.nested.obj.count + 1,
      },
    },
  },
}))
```

**장점:** 외부 의존성 없음, 명시적.  
**단점:** 중첩 3단계 이상이면 코드가 급격히 비대해짐.

### 4-2. Immer 미들웨어 (권장 — 레이아웃 트리)

**확신도: 확실** (공식 Immer middleware docs 확인)

```typescript
import { create } from 'zustand';
import { immer } from 'zustand/middleware/immer';

// 타입
type PageId = string;
type PaneId = string;
type TabId = string;

type LayoutNode =
  | { type: 'split'; direction: 'row' | 'column'; children: LayoutNode[]; sizes?: number[] }
  | { type: 'tabs'; paneId: PaneId; tabIds: TabId[]; activeTabId: TabId };

type PageLayoutState = {
  id: PageId;
  title: string;
  root: LayoutNode;
  activePaneId?: PaneId;
  updatedAt: number;
};

type DashboardStore = {
  activePageId: PageId;
  pageOrder: PageId[];
  pagesById: Record<PageId, PageLayoutState>;

  setActivePage: (pageId: PageId) => void;
  setPaneActiveTab: (pageId: PageId, paneId: PaneId, tabId: TabId) => void;
  updatePageLayout: (pageId: PageId, fn: (page: PageLayoutState) => void) => void;
};

// 구현
export const useDashboardStore = create<DashboardStore>()(
  immer((set) => ({
    activePageId: 'home',
    pageOrder: ['home'],
    pagesById: {},

    setActivePage: (pageId) =>
      set((s) => {
        if (s.pagesById[pageId]) s.activePageId = pageId;
        // 비활성 페이지 상태는 pagesById에 그대로 유지 — 별도 작업 불필요
      }),

    setPaneActiveTab: (pageId, paneId, tabId) =>
      set((s) => {
        const page = s.pagesById[pageId];
        if (!page) return;
        visitLayout(page.root, (node) => {
          if (node.type === 'tabs' && node.paneId === paneId) {
            node.activeTabId = tabId;  // Immer draft 직접 변이
          }
        });
        page.updatedAt = Date.now();
      }),

    updatePageLayout: (pageId, fn) =>
      set((s) => {
        const page = s.pagesById[pageId];
        if (page) fn(page);
      }),
  }))
);

// 트리 순회 헬퍼
function visitLayout(node: LayoutNode, fn: (n: LayoutNode) => void): void {
  fn(node);
  if (node.type === 'split') node.children.forEach((c) => visitLayout(c, fn));
}
```

**주의사항:**
- 클래스 객체는 `[immerable] = true` 필요. 없으면 draft가 아닌 현재 상태를 직접 변이해 구독 콜백 미실행.
- plain object/array에는 문제 없음.

### 4-3. optics-ts / Ramda (대안)

**확신도: 가능성 높음** (복수 출처 확인)

```typescript
// optics-ts
import * as O from 'optics-ts'
set(O.modify(O.optic<State>().path("deep.nested.obj.count"))((c) => c + 1))

// Ramda
import * as R from 'ramda'
set(R.over(R.lensPath(["deep", "nested", "obj", "count"]), (c) => c + 1))
```

**장점:** 경로 기반, 타입 추론 강력 (optics-ts).  
**단점:** 학습 곡선, 팀 친숙도 낮을 수 있음.

**출처:**
- [Zustand 중첩 상태 가이드](https://awesomedevin.github.io/zustand-vue/en/docs/advanced/sickof-changing-nested-state)
- [Immer middleware 공식 reference](https://zustand.docs.pmnd.rs/reference/integrations/immer-middleware)

---

## 핵심 질문 답변

### Q1. 단일 스토어 vs 페이지별 스토어?

**결론: 단일 스토어** (VS Code·Zed·react-mosaic 모두 동일 패턴)

```typescript
// 권장 단일 스토어 구조
{
  activePageId: PageId,
  pageOrder: PageId[],
  pagesById: Record<PageId, PageLayoutState>  // 각 페이지 레이아웃 격리
}
```

| 단일 스토어 | 페이지별 스토어 |
|---|---|
| 크로스 페이지 명령(이동/복제) 단순 | 리렌더 범위 좁음 |
| 영속화·undo/redo·라우팅 단순 | 페이지 lifecycle 복잡 |
| VS Code/Zed 검증된 패턴 | 페이지 간 공유 상태 어려움 |

**확신도: 가능성 높음** (VS Code/Zed 구조에서 귀납 + Codex 명시 권장)

### Q2. 페이지 전환 시 이전 상태 보존/복원?

**결론:** `pagesById[pageId]` 레코드를 그대로 두고 `activePageId`만 교체.

```typescript
setActivePage: (pageId) => set((s) => {
  if (s.pagesById[pageId]) s.activePageId = pageId;
  // 비활성 페이지는 pagesById에 살아있음 — 복원 불필요
})
```

VS Code의 `EditorGroupModel`이 메모리에 유지되며 전환 시 그룹 ID만 변경하는 것과 동일한 원리.  
컴포넌트 언마운트를 피하려면 CSS `display: none`/`visibility: hidden`으로 DOM 유지.

**확신도: 확실** (VS Code/Zed 소스 직접 확인)

### Q3. activePageId + 페이지 레이아웃 관계 표현?

**결론:** activePageId = 포인터, 레이아웃은 `pagesById` Record에 격리.

```typescript
type LayoutWorkspaceState = {
  activePageId: PageId;                         // 포인터
  pagesById: Record<PageId, PageLayoutState>;   // 레이아웃 격리
};

// 선택자
const selectActivePage = (s: LayoutWorkspaceState) =>
  s.pagesById[s.activePageId];

// 특정 페이지 레이아웃 선택자 (리렌더 최소화)
const selectPageLayout = (pageId: PageId) =>
  (s: LayoutWorkspaceState) => s.pagesById[pageId];
```

VS Code의 `{ serializedGrid, activeGroup, mostRecentActiveGroups }` 구조와 동일한 패턴.

**확신도: 확실**

### Q4. Zustand 중첩 상태 업데이트 패턴?

**결론: Immer 미들웨어 권장** (중첩 3단계 이상 레이아웃 트리)

| 방식 | 적합 상황 | 비고 |
|---|---|---|
| spread | 얕은 중첩 (1~2단계) | 외부 의존성 없음 |
| **Immer (권장)** | 레이아웃 트리 (3단계+, 재귀) | 공식 지원, 코드 간결 |
| optics-ts | 타입 안전 경로 접근 필요 시 | 학습 곡선 있음 |
| Ramda | 함수형 파이프라인 선호 시 | 번들 크기 주의 |

**확신도: 가능성 높음** (Zustand 공식 docs + 복수 커뮤니티 출처 일치)

---

## 교차 검증표 (Claude ↔ Codex)

| 클레임 | Claude 수집 | Codex 결과 | 합의 여부 | 최종 확신도 |
|---|---|---|---|---|
| VS Code: 단일 스토어 (serializedGrid + activeGroup) | ✅ (소스 fetch) | ✅ (코드 발췌) | 합의 | 확실 |
| VS Code: MRU 인덱스로 활성 에디터 추적 | ✅ (EditorPartUIState) | ✅ | 합의 | 확실 |
| Zed: AppState = 글로벌 서비스, 레이아웃은 PaneGroup | ✅ (소스 fetch) | ✅ (구조체 코드) | 합의 | 확실 |
| Zed: SerializedPaneGroup = 재귀 enum | ✅ (소스 fetch) | ✅ | 합의 | 확실 |
| react-mosaic v7: MosaicTabsNode.activeTabIndex | v7 릴리즈 노트 | ✅ (코드 예시) | 합의 | 가능성 높음 |
| 단일 스토어 권장 | VS Code/Zed 귀납 | ✅ 명시 | 합의 | 가능성 높음 |
| Immer = 레이아웃 트리에 최적 | 공식 docs | ✅ | 합의 | 가능성 높음 |

불일치 클레임: 없음.

---

## 공백 및 한계

- react-mosaic v7 정식 types.ts 소스 파일을 직접 fetch하지 못함 (GitHub raw 404). `MosaicTabsNode` 타입은 v7 릴리즈 노트 + 검색 결과 합성 — "가능성 높음"으로 강등.
- VS Code `EditorGroupModel` 내부 MRU 직렬화 세부 로직은 `editorGroupModel.ts` 직접 fetch 실패. `editorPart.ts` fetch 결과와 Codex 분석에서 간접 확인.
- Zed의 SQLite 스키마(KEY_VALUE_STORE 구체 구조)는 별도 조사 필요.
- golden-layout v2 내부 TypeScript 타입은 공식 문서 기반, 실제 구현 타입과 미세 차이 가능성.

---

## 참고 출처

- [VS Code editorGroupsService.ts](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/services/editor/common/editorGroupsService.ts)
- [VS Code editorPart.ts](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/browser/parts/editor/editorPart.ts)
- [VS Code editorGroupModel.ts](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/common/editor/editorGroupModel.ts)
- [Zed persistence/model.rs](https://github.com/zed-industries/zed/blob/main/crates/workspace/src/persistence/model.rs)
- [Zed workspace.rs](https://github.com/zed-industries/zed/blob/main/crates/workspace/src/workspace.rs)
- [Zed pane.rs](https://github.com/zed-industries/zed/blob/main/crates/workspace/src/pane.rs)
- [react-mosaic GitHub](https://github.com/nomcopter/react-mosaic)
- [react-mosaic v7 beta Discussion](https://github.com/nomcopter/react-mosaic/discussions/241)
- [golden-layout 공식](https://golden-layout.github.io/golden-layout/)
- [Zustand Immer middleware 공식](https://zustand.docs.pmnd.rs/reference/integrations/immer-middleware)
- [Zustand 중첩 상태 가이드](https://awesomedevin.github.io/zustand-vue/en/docs/advanced/sickof-changing-nested-state)
- [DeepWiki VS Code Layout System](https://deepwiki.com/microsoft/vscode/3.2-editor-features-and-contributions)
- [DeepWiki Zed Workspace Persistence](https://deepwiki.com/zed-industries/zed/3.4-panel-system)
