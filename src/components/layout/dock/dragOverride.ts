// TRD S21-slot-docking §2-4 · §4-6: 출처 창의 「바꿔 그리기」 — 끄는 동안 커서가 자기 탭바 구역 밖이면 끄는 탭 대신 남은 탭을
//   보이는 웹 표시(R15 · 모델은 그대로)와, 놓은 뒤 그 표시 · 탭바 순서 이동 모습(transform)을 언제 거두나를 정하는 순수 리듀서.
//   원래 선택 되돌리기(`switch_tab_if(initial, expect = 끄는 탭)`)를 낼지도 여기서 정한다 — 내는 것은 부르는 쪽(출처 훅)이다.
// ★시한이 없다★ — 푸는 계기는 넷뿐이다: ① 그 끌기의 `tab_drop:done`(자기 invoke 오류면 곧바로) ② dragend 뒤 끄는 탭을 활성으로
//   두지 않는 `window:tabs-updated` ③ 그 창의 다음 pointerdown · dragstart ④ 셸이 끝을 정하지 않는 출처 dragend(같은 웹뷰 제자리
//   놓기 · 엔진이 접은 끌기). 시한으로 풀면 `Drop` 적용 전에 끄는 탭이 한두 프레임 보인다.
// 부르는 쪽 약속: 사건은 그 창 스토어에 적용한 **뒤의** 탭 상태(`WindowModel`)와 함께 넣는다. dragend 는 그 끌기의 `DragEnd`
//   invoke 를 보내기 **전에** 넣는다 — 그래야 dragend 전에 닿은 `done` 이 받은 창의 `Drop` 몫이라고 가를 수 있다.

import type { DropOutcome } from '../../../../src-tauri/bindings/DropOutcome'
import type { EndEffect } from '../../../../src-tauri/bindings/EndEffect'
import { succession } from '../tabStrip/succession'

/** 그 창의 탭 상태 — 스토어 `windows[label]` 의 탭 id 목록과 활성(모르면 `null`). */
export interface WindowModel {
  tabs: readonly string[]
  active: string | null
}

/** 출처 창이 쥐는 끌기 하나. `idle` = 바꿔 그리기 없음(그리는 탭 = 모델 활성 · transform 0). */
export type OverrideState =
  | { phase: 'idle' }
  /**
   * 되돌리기(`Revert`)를 냈고 그 활성이 모델에 닿기를 기다린다 — 그동안 `to` 를 그린다(transform 0). 풀면 곧바로 모델
   * 활성(아직 끄는 탭)을 그려 `to` → 끄는 탭 → `to` 로 번쩍인다(메인 결정 2026-10-20). 되돌리기가 끝내 닿지 않으면 ② · ③ 이 푼다
   * — 셸이 조건이 어긋나 안 바꿨으면 그사이 활성을 바꾼 쪽의 알림이 ② 다.
   */
  | { phase: 'reverting'; drag: string; view: string; to: string }
  | {
      /** `live` = dragstart ~ dragend · `ended` = dragend 뒤 풀 계기를 기다림. */
      phase: 'live' | 'ended'
      /** 출처 창 label — 되돌리기를 보낼 창. */
      window: string
      drag: string
      view: string
      initial: string | null
      /** 구역 밖에서 보일 남은 탭 — 셸이 놓을 때 고르는 `successor` 와 같다(TRD §3-3). `null` = 탭 하나뿐(바꿔 그림 없음). */
      remaining: string | null
      /** dragstart 때 그 창 탭 순서 — 이 순서가 그대로인 동안만 순서 이동 모습을 그린다. */
      startTabs: readonly string[]
      /** 커서가 자기 탭바 구역 밖이다(창 밖 포함). dragend 뒤에는 그때 값에 머문다. */
      out: boolean
      /** 자기 dragend 의 `dropEffect` — `live` 동안 `null`. */
      effect: EndEffect | null
      /** 이 끌기의 `done{applied}` 를 받았다 — 탭 목록이 dragstart 때와 달라지면 푼다. */
      applied: boolean
    }

export type OverrideEvent =
  /** 출처 dragstart — 앞 끌기는 버린다(③). `window` = 출처 창 label · `initial` = 누르기 전 그 창 모델 활성. */
  | { type: 'start'; window: string; drag: string; view: string; initial: string | null }
  /** 커서가 자기 탭바 구역에 들어옴(`out: false`) · 벗어남(`out: true`) — dragend 전까지만 듣는다. */
  | { type: 'zone'; out: boolean }
  /**
   * 출처 dragend. `local` = 이 끌기의 끝을 셸에 보내지 않아 여기서 푼다(④ · 되돌리지 않음 — 누른 탭이 활성으로 남아 클릭과 같다):
   * 이 웹뷰의 drop 이 같은 창 같은 자리라 보내지 않았다 · dragstart 와 같은 task 에서 난 dragend = 엔진이 OS 끌기를 열지 못하고
   * 접었다(옛 HTML5 끌기 실측 — dragstart 뒤 0–1 ms · 메인 결정 2026-10-20 — 셸에 아무것도 보내지 않는다).
   */
  | { type: 'end'; effect: EndEffect; local: boolean }
  | { type: 'done'; drag: string; outcome: DropOutcome }
  /** 자기 `tab_drop` invoke 가 오류로 끝남. */
  | { type: 'sendFailed'; drag: string }
  /** 그 창의 `window:tabs-updated` 를 스토어에 적용함. */
  | { type: 'tabs' }
  /** 그 창의 pointerdown. */
  | { type: 'pointerdown' }

/**
 * 원래 선택 되돌리기(R12 · R15) — `switch_tab_if(window, to, expect)` 로 낸다: 셸은 그 창 활성이 아직 `expect`(누름이 활성으로
 * 한 끄는 탭)일 때만 `to` 로 바꾼다. 그래서 되돌리기가 닿기 전에 사용자 · LLM 이 고른 탭을 덮지 않는다(웹 스토어가 늦어도).
 */
export interface Revert {
  window: string
  to: string
  expect: string
}

export interface OverrideStep {
  state: OverrideState
  /** 낼 되돌리기 — 끌기 하나에 많아야 한 번이다. */
  revert: Revert | null
}

export const IDLE: OverrideState = { phase: 'idle' }

const released = (): OverrideStep => ({ state: IDLE, revert: null })

export function stepOverride(state: OverrideState, event: OverrideEvent, model: WindowModel): OverrideStep {
  if (event.type === 'start') {
    const { window, drag, view, initial } = event
    const remaining =
      initial !== null && initial !== view && model.tabs.includes(initial) ? initial : succession(model.tabs, view)
    return {
      state: {
        phase: 'live',
        window,
        drag,
        view,
        initial,
        remaining,
        startTabs: [...model.tabs],
        out: false,
        effect: null,
        applied: false,
      },
      revert: null,
    }
  }
  if (state.phase === 'idle') return { state, revert: null }
  const same: OverrideStep = { state, revert: null }

  if (state.phase === 'reverting') {
    if (event.type === 'pointerdown') return released()
    // 되돌리기가 닿음(활성 = `to`) · LLM 이 다른 탭으로 바꿈 · 끄는 탭이 떠남 — 셋 다 활성이 끄는 탭이 아니게 된 것이다(②).
    if (event.type === 'tabs' && model.active !== state.view) return released()
    return same
  }

  switch (event.type) {
    case 'zone':
      return state.phase === 'live' && state.out !== event.out ? { state: { ...state, out: event.out }, revert: null } : same
    case 'end':
      if (state.phase !== 'live') return same
      return event.local ? released() : { state: { ...state, phase: 'ended', effect: event.effect }, revert: null }
    case 'pointerdown':
      return released()
    case 'sendFailed':
      return event.drag === state.drag ? released() : same
    case 'done': {
      if (event.drag !== state.drag) return same
      if (event.outcome === 'rejected') return released()
      if (event.outcome === 'applied') {
        // 탭 하나뿐인 창은 바꿔 그림이 없어 목록 바뀜을 기다릴 것이 없다(옮겨진 팝아웃은 목록이 그대로다).
        const next = { ...state, applied: true }
        return state.remaining === null || orderChanged(next, model) ? released() : { state: next, revert: null }
      }
      // 취소 · 무시. dragend 전에 닿은 것은 받은 창의 `Drop` 몫이다 — 출처 dragend 는 `move` 다. `move` 면 받은 곳의 `Drop` 결과가
      //   뒤따르거나 그 창이 보내지 않았다(같은 창 같은 자리) — 이것으로 풀면 `Drop` 적용 전에 끄는 탭이 보이고, 되돌리면 클릭과
      //   달라진다.
      if (state.phase === 'live' || state.effect === 'move') return same
      const to = revertTarget(state, model)
      return to === null
        ? released()
        : {
            state: { phase: 'reverting', drag: state.drag, view: state.view, to },
            revert: { window: state.window, to, expect: state.view },
          }
    }
    case 'tabs':
      // 끄는 탭이 그 창을 떠났다 — 어느 때든 푼다(메인 결정 2026-10-20). 옮겨 나감이면 모델 활성이 곧 남은 탭이고, 끄는 중
      //   닫혔으면 떨어진 머리의 dragend 가 오지 않는다(TRD §4-7 (c) 와 같은 까닭).
      if (!model.tabs.includes(state.view)) return released()
      if (state.applied && orderChanged(state, model)) return released()
      return state.phase === 'ended' && model.active !== state.view ? released() : same
  }
}

/** 그 창이 지금 그릴 탭 — 바꿔 그리는 동안은 남은 탭 · 되돌리기를 기다리는 동안은 되돌릴 탭, 아니면 모델 활성. */
export function shownTab(state: OverrideState, model: WindowModel): string | null {
  if (state.phase === 'reverting') return model.tabs.includes(state.to) ? state.to : model.active
  if (state.phase !== 'idle' && state.out && state.remaining !== null && model.tabs.includes(state.remaining)) {
    return state.remaining
  }
  return model.active
}

/**
 * 탭바 순서 이동 모습(transform)을 그려도 되나 — 끌기가 풀리지 않았고 그 창 탭 순서가 dragstart 때 그대로일 때만. 모델 순서가
 * 바뀐 그 프레임에 거둬야 놓은 탭이 튀지 않는다(TRD §2-4 · §8-3 ④).
 */
export function stripHeld(state: OverrideState, model: WindowModel): boolean {
  return (state.phase === 'live' || state.phase === 'ended') && sameOrder(state.startTabs, model.tabs)
}

function orderChanged(state: { startTabs: readonly string[] }, model: WindowModel): boolean {
  return !sameOrder(state.startTabs, model.tabs)
}

function sameOrder(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((t, i) => t === b[i])
}

/**
 * 취소 · 무시 뒤 되돌릴 탭(TRD §4-6) — 누름의 `switch_tab` 이 바꾼 활성을 누르기 전으로. 건너뜀 = `initial` 없음 · 끄는 탭 자신 ·
 * 그 창 탭이 아님 · 그 창 활성이 이미 끄는 탭이 아님(그사이 사용자 · LLM 이 바꿨다 — 덮지 않는다).
 */
function revertTarget(state: { view: string; initial: string | null }, model: WindowModel): string | null {
  const { view, initial } = state
  if (initial === null || initial === view || !model.tabs.includes(initial) || model.active !== view) return null
  return initial
}
