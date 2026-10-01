// ADR-0244: 채팅 칸에서 끊기를 보낸 뒤 그 턴이 끝날 때까지의 「중단하는 중」 — 에이전트마다 그 끊기 요청을 쥔다.
//   프론트 전용 · 인메모리 · 창마다 따로다(zustand 모듈 스토어 — 창마다 자기 인스턴스를 갖는다).
//   ★세우는 곳은 창 명령 `agent.interrupt`(`commands/agentCommands.ts`) 하나다★ — 사람 Esc 와 LLM 이 같은 길을 지난다
//   (CLAUDE.md 「LLM-우선 제어」). 두 번째 길을 만들지 말 것. 걷는 곳 —
//   ① 그 요청이 거절됐을 때(명령)
//   ② 대화 뷰(RichSlot)가 라이브로 받은 턴 경계(MessageDone · TurnEnd — 결말 무관) · 처음 따라잡았는데 열린 턴이 없을 때 ·
//      대기 표시가 켜짐 → 꺼짐일 때 · 새 화신(비우기)일 때 · 에이전트가 없어졌거나 연결이 끊겼을 때
//   ③ 그 에이전트의 마지막 대화 뷰가 풀릴 때(아래 `watch`) — 구독이 멈춘 뷰(`detached` · `error`)는 풀린 것으로 센다.
//   ★이 창에 그 에이전트의 대화 뷰가 하나도 없으면 세우지 않는다(`watch`)★ — 턴 끝을 보고 걷을 쪽이 없어, 세우면 그 뒤
//   이 창의 끊기 명령이 전부 「이미 보냈다」로 답하고 보내지 않는다. 같은 까닭으로 마지막 뷰가 사라질 때 걷는다.
//   버스로 곧장 보낸 끊기(`engram agent.interrupt …`)는 이 창 명령을 지나지 않아 여기 서지 않는다 — 의도된 범위다.

import { create } from 'zustand'

export interface InterruptState {
  /** 끊기를 보내고 그 턴이 끝나기를 기다리는 에이전트 → 그 끊기 요청. 칸이 없으면 기다리지 않는다. */
  pending: Record<string, Promise<void>>
  /** 에이전트 → 이 창에 지금 마운트된 그 에이전트의 대화 뷰 수. 칸이 없으면 0 이다. */
  views: Record<string, number>
  /**
   * 그 에이전트의 대화 뷰가 마운트됐다. @returns 해제 함수 — 마운트 해제(정리)에서 부른다. 두 번 불러도 한 번만 센다.
   *   마지막 뷰가 풀리면 기다림도 걷는다.
   */
  watch(agentId: string): () => void
  /** 그 에이전트가 기다리는 중이 된다. 이 창에 그 에이전트의 대화 뷰가 없으면 아무것도 안 한다(머리 주석). */
  begin(agentId: string, request: Promise<void>): void
  /**
   * 기다림을 걷는다. `request` 를 주면 지금 쥔 것이 그 요청일 때만 걷는다 — 늦게 온 옛 요청의 거절이 그 뒤 새 요청의
   * 기다림을 지우지 않게. 없으면 누구의 것이든 걷는다(턴 끝). 기다리는 중이 아니면 아무것도 안 한다.
   */
  end(agentId: string, request?: Promise<void>): void
}

// 에이전트 id 는 밖(LLM)에서도 온다 — 맨 인덱스로 읽으면 `'constructor'` 같은 프로토타입 이름이 칸으로 읽힌다.
function own<T>(record: Record<string, T>, key: string): T | undefined {
  return Object.prototype.hasOwnProperty.call(record, key) ? record[key] : undefined
}

function without<T>(record: Record<string, T>, key: string): Record<string, T> {
  const next = { ...record }
  delete next[key]
  return next
}

/** 그 에이전트가 기다리는 끊기 요청 — 없으면 `undefined`(중단하는 중이 아니다). */
export function pendingInterrupt(
  state: Pick<InterruptState, 'pending'>,
  agentId: string,
): Promise<void> | undefined {
  return own(state.pending, agentId)
}

export const useInterruptStore = create<InterruptState>((set, get) => ({
  pending: {},
  views: {},
  watch: agentId => {
    set(s => ({ views: { ...s.views, [agentId]: (own(s.views, agentId) ?? 0) + 1 } }))
    let released = false
    return () => {
      if (released) return
      released = true
      const left = (own(get().views, agentId) ?? 1) - 1
      if (left > 0) {
        set(s => ({ views: { ...s.views, [agentId]: left } }))
        return
      }
      set(s => ({ views: without(s.views, agentId), pending: without(s.pending, agentId) }))
    }
  },
  begin: (agentId, request) => {
    if ((own(get().views, agentId) ?? 0) === 0) return
    set(s => ({ pending: { ...s.pending, [agentId]: request } }))
  },
  end: (agentId, request) => {
    const current = own(get().pending, agentId)
    if (current === undefined || (request !== undefined && current !== request)) return
    set(s => ({ pending: without(s.pending, agentId) }))
  },
}))
