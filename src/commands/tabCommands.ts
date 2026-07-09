// ADR-0055/0057: 탭/창 command 어댑터 — register 로 기존 store 액션(viewStore)에 라우팅만 한다(새 상태
//   경로 0). import 부수효과로 등록되므로 부팅 경로(App.tsx)에서 side-effect import 한다. 사람 클릭(TabBar·
//   Ctrl+Tab)·LLM(__engramCmd)·window.__engramLayout 이 모두 이 동일 store 액션을 지난다(§5 단일 제어 표면).
//
// ★window 해소★: command args 로 window 를 받되, 생략하면 이 웹뷰 창(readWindowLabelFromHash — main·팝업
//   label)으로 떨어진다. 그래서 Ctrl+Tab 같은 "포커스된 창" 소비자가 별도 label 계산 없이 부를 수 있다.

import { invoke } from '@tauri-apps/api/core'

import { register } from './registry'
import { readWindowLabelFromHash, useViewStore } from '../store/viewStore'

/** args.window(있으면) 또는 이 웹뷰 창 label. */
function resolveWindow(args?: Record<string, unknown>): string {
  const w = args?.window
  return typeof w === 'string' && w.length > 0 ? w : readWindowLabelFromHash()
}

/**
 * ★타깃 창(window)의 active 탭★(S4-F1). view 를 생략한 창별 command(close/switch/next)는 *지정된 창*의
 * active 탭을 대상으로 삼아야 한다 — 현재 웹뷰의 active(currentViewId)가 아니다. window 가 이 웹뷰 창과
 * 다른데 currentViewId 로 채우면 (window, view) 쌍이 어긋나 백엔드가 ViewNotFound 로 거부한다.
 * 그 창 상태가 아직 안 왔으면 null(호출자가 미확정 처리).
 */
function activeViewOf(window: string): string | null {
  return useViewStore.getState().windows[window]?.active ?? null
}

register({
  id: 'tab.create',
  title: '새 탭',
  category: 'tab',
  // args.window(생략 시 이 창) 에 빈-슬롯 탭 추가. name 옵션.
  run: args => {
    const name = typeof args?.name === 'string' ? args.name : undefined
    return useViewStore.getState().createTab(resolveWindow(args), name)
  },
})

register({
  id: 'tab.switch',
  title: '탭 전환',
  category: 'tab',
  // args.view(필수) 로 그 창 active 탭 교체.
  run: args => {
    const view = args?.view
    if (typeof view !== 'string') throw new Error('[tab.switch] view(탭 id) 필요')
    return useViewStore.getState().switchTab(resolveWindow(args), view)
  },
})

register({
  id: 'tab.close',
  title: '탭 닫기',
  category: 'tab',
  // args.view(생략 시 ★그 창의★ active 탭, S4-F1) 를 닫는다.
  run: args => {
    const window = resolveWindow(args)
    // ★S4-F1★: view 생략 시 지정된 창(window)의 active 를 쓴다 — 현재 웹뷰 active 가 아니다. window 가
    //   다른 창을 가리키면 (window, view) 어긋남 → 백엔드 ViewNotFound.
    const view = typeof args?.view === 'string' ? args.view : activeViewOf(window)
    if (!view) throw new Error('[tab.close] 닫을 탭 id 미확정')
    return useViewStore.getState().closeTab(window, view)
  },
})

register({
  id: 'tab.next',
  title: '다음 탭(순환)',
  category: 'tab',
  keybinding: 'Ctrl+Tab',
  // ★Ctrl+Tab(D-8)★: 포커스된 창의 탭을 순환한다 = switch_tab(이 창, 다음 탭). 사람 키·클릭과 동일 command
  //   경로(§5). 탭이 없거나 1개면 no-op. 방향은 오른쪽(마지막이면 첫 탭으로 wrap).
  run: args => {
    const window = resolveWindow(args)
    const win = useViewStore.getState().windows[window]
    if (!win || win.tabs.length <= 1) return
    const idx = win.tabs.findIndex(t => t.id === win.active)
    // active 를 못 찾으면(비정상) 첫 탭으로. 찾으면 다음(오른쪽 순환).
    const nextIdx = idx < 0 ? 0 : (idx + 1) % win.tabs.length
    return useViewStore.getState().switchTab(window, win.tabs[nextIdx].id)
  },
})

register({
  id: 'window.create',
  title: '새 창',
  category: 'window',
  // 빈 새 창(빈 탭 1개) 생성 → label 반환(D-6).
  run: () => useViewStore.getState().createWindow(),
})

register({
  id: 'window.close',
  title: '창 닫기',
  category: 'window',
  // args.window(생략 시 이 창) 닫기. main 은 백엔드가 거부(hide only, 불변식 4).
  run: args => useViewStore.getState().closeWindow(resolveWindow(args)),
})

/** UUID(8-4-4-4-12 hex) 형식 검사 — tab/slot 는 백엔드 ViewId/slot id(Uuid) 라 형식이 맞아야 한다. */
const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i

/**
 * ★spawn_into 인자 정규화(FIX 5)★: tab/slot 는 "있으면 UUID 문자열, 없으면 미지정(null)"이다. 이전엔
 * 잘못된 타입(숫자 등)을 조용히 null 로 강등해 "스폰은 됐는데 엉뚱한 곳에 배치"로 이어졌다 → present-but-invalid
 * 는 invoke **전에** throw 해 side-effecting 스폰을 막는다. 부재(undefined/null)는 정상("미지정").
 */
function optionalUuidArg(v: unknown, name: string): string | null {
  if (v === undefined || v === null) return null // 미지정 — 정상.
  if (typeof v !== 'string' || !UUID_RE.test(v)) {
    throw new Error(`[agent.spawnInto] ${name} 는 UUID 문자열이어야 함(받음: ${JSON.stringify(v)})`)
  }
  return v
}

register({
  id: 'agent.spawnInto',
  title: '스폰 + 배치',
  category: 'agent',
  // ★spawn_into(D-7, TRD §6 G9)★: 스폰(데몬) + 탭 생성(필요 시) + 슬롯 배정을 한 방으로 조립하는 백엔드
  //   합성 command 를 직접 invoke 한다(store 상태 없이 backend 권위 — ADR-0057). 성공 시 새 AgentId 반환.
  //   args: window(생략 시 이 창)·cwd(필수)·tab?·slot?·backend?. 슬롯 정책·실패 가시성은 backend 가 강제.
  //   ★FIX 5★: tab/slot 은 invoke 전에 UUID 형식 검증(잘못된 값 → 스폰 전 throw, 오배치 방지).
  run: args => {
    const cwd = args?.cwd
    if (typeof cwd !== 'string' || cwd.length === 0) throw new Error('[agent.spawnInto] cwd 필요')
    const tab = optionalUuidArg(args?.tab, 'tab')
    const slot = optionalUuidArg(args?.slot, 'slot')
    const backend = typeof args?.backend === 'string' ? args.backend : null
    return invoke<string>('spawn_into', {
      window: resolveWindow(args),
      tab,
      slot,
      backend,
      cwd,
    })
  },
})
