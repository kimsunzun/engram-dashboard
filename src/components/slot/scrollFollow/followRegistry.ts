// ADR-0242: 슬롯 id → 스크롤 따라가기 손잡이 — 이 창(웹뷰) 안의 모듈 맵이다. 창 안 command
//   (`commands/scrollCommands.ts`)가 여기서 손잡이를 찾는다. 새 전역 핸들을 만들지 않는다(CLAUDE.md 「LLM-우선 제어」).

/** 한 슬롯의 스크롤 따라가기 손잡이 — 훅(`useScrollFollow`)이 만들고 슬롯·command 가 부른다. */
export interface FollowHandle {
  readonly pinned: boolean
  /** 바닥에 붙이고 바닥으로 쓴다. 숨은 탭이면 붙음만 세우고 쓰기는 보일 때 한다. */
  pin(): void
  /** 따라가기를 놓는다 — 위치는 그대로 둔다. */
  unpin(): void
}

const handles = new Map<string, FollowHandle>()

/**
 * 올리고 내리는 함수를 돌려준다. ★내리기는 그 자리가 아직 자기 손잡이일 때만 지운다★ — 같은 슬롯에 새
 * 마운트가 먼저 올렸을 수 있다(StrictMode 이중 마운트 · 렌더 모드 교체 · 정체성 key 재마운트).
 */
export function registerFollow(slotId: string, handle: FollowHandle): () => void {
  handles.set(slotId, handle)
  return () => {
    if (handles.get(slotId) === handle) handles.delete(slotId)
  }
}

/** 없으면 `undefined` — 이 창에 그 슬롯의 대화·텍스트 뷰가 마운트돼 있지 않다. */
export function getFollow(slotId: string): FollowHandle | undefined {
  return handles.get(slotId)
}
