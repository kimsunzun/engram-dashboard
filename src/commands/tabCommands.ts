// ADR-0055/0057: 사람 클릭(TabBar·Ctrl+Tab)과 LLM(__engramCmd)이 모두 이 동일 store 액션을 지난다
//   (§5 단일 제어 표면 — 새 상태 경로 0).
//
// ★window 해소★: command args 로 window 를 받되, 생략하면 이 웹뷰 창(readWindowLabelFromHash — main·팝업
//   label)으로 떨어진다. 그래서 Ctrl+Tab 같은 "포커스된 창" 소비자가 별도 label 계산 없이 부를 수 있다.

import { invoke } from '@tauri-apps/api/core'

import type { LayoutNode, SlotContent } from '../api/layoutTypes'
import { t } from '../i18n'
import { matchDeclaredSpelling } from './enumArg'
import { register } from './registry'
import { readWindowLabelFromHash, useViewStore } from '../store/viewStore'
import type { WindowTabsPayload } from '../store/viewStore'

// 빈 · 공백뿐 · 문자열 아닌 값을 fallback 으로 접지 않는다 — 빈 인자는 대개 안 채운 셸 변수라, 접으면 다른 창을
//   건드린다(버스 `layout/commands.rs` `text` 와 같은 조항). 없는 인자(undefined · null — optionalUuidArg 와
//   같은 가름)만 이 웹뷰 창으로 떨어진다.
function resolveWindow(cmd: string, args?: Record<string, unknown>): string {
  const w = args?.window
  if (w === undefined || w === null) return readWindowLabelFromHash()
  if (typeof w !== 'string' || w.trim().length === 0) {
    throw new Error(
      `[${cmd}] window 는 비어 있지 않은 문자열이어야 함 — 빈 인자는 대개 안 채운 셸 변수(받음: ${describeArg(w)})`,
    )
  }
  return w
}

// JSON.stringify 는 BigInt · 순환 객체에서 던진다 — 그러면 `[<cmd>]` 를 붙인 오류 대신 그 TypeError 가 나간다.
function describeArg(v: unknown): string {
  try {
    return JSON.stringify(v) ?? String(v)
  } catch {
    return typeof v === 'bigint' ? `${String(v)}n` : typeof v
  }
}

/**
 * 창 인자 `window` 의 탭 목록을 셸에서 받는다 — 셸이 버스와 같은 해석기로 그 인자를 푼다(window_id · 같은 실행 안 label
 * 을 받고 번호는 거절 — TRD S26-window-chrome §4-5). 답의 `label` 이 푼 창이다. ★웹 스토어(`windows`)로 고르지 말
 * 것★ — 그 키는 label 뿐이라 window_id 를 받으면 아무 창도 못 찾아 조용히 아무 일도 안 한다. 셸 거절(invoke 는 문자열로
 * 거절한다)은 명령 이름을 붙인 Error 로 던진다.
 */
async function listTabsOf(cmd: string, window: string): Promise<WindowTabsPayload> {
  try {
    return await invoke<WindowTabsPayload>('list_tabs', { window })
  } catch (e) {
    throw new Error(`[${cmd}] ${e instanceof Error ? e.message : String(e)}`)
  }
}

register({
  id: 'tab.create',
  title: t('tab.create'),
  category: 'tab',
  // name 은 tab.rename 과 같게 다듬고 공백뿐이면 invoke 전에 throw 한다 — 빠지면 자동 이름.
  run: args => {
    let name: string | undefined
    if (typeof args?.name === 'string') {
      name = args.name.trim()
      if (name.length === 0) throw new Error('[tab.create] name 은 공백일 수 없음')
    }
    return useViewStore.getState().createTab(resolveWindow('tab.create', args), name)
  },
})

register({
  id: 'tab.switch',
  title: t('tab.switch'),
  category: 'tab',
  run: args => {
    const view = args?.view
    if (typeof view !== 'string') throw new Error('[tab.switch] view(탭 id) 필요')
    return useViewStore.getState().switchTab(resolveWindow('tab.switch', args), view)
  },
})

register({
  id: 'tab.close',
  title: t('tab.closeCmd'),
  category: 'tab',
  // view 생략 = 지정된 창(window)의 활성 탭 — 이 웹뷰의 활성이 아니다(S4-F1).
  run: async args => {
    const window = resolveWindow('tab.close', args)
    const view = args?.view
    if (typeof view === 'string') return useViewStore.getState().closeTab(window, view)
    const tabs = await listTabsOf('tab.close', window)
    return useViewStore.getState().closeTab(tabs.label, tabs.active)
  },
})

register({
  id: 'tab.next',
  title: t('tab.next'),
  category: 'tab',
  keybinding: 'Ctrl+Tab',
  // ★생략 시 무엇이 바뀌나를 정확히 적는다★: 버스 호출자에겐 「자기 창」이 없고, 실제 대상은 셸이 고른
  //   목적지 창(보통 main)이다. 아래 resolveWindow 의 fallback 은 그 목적지 창의 URL 해시를 읽는다 — 즉
  //   생략하면 사실상 main 이다. LLM 이 window 를 넘길지 말지를 정하는 근거가 이 문장뿐이라 「이 창」 같은
  //   말을 쓰지 않는다.
  help: {
    summary: '그 창의 활성 탭을 다음 탭으로 옮긴다(탭이 하나뿐이면 아무 일도 안 한다).',
    effect: 'write',
    args: {
      window: {
        type: 'string',
        description:
          '대상 창 — window.list 의 window_id(재시작을 넘어 같은 창) 또는 같은 실행 안 label. 번호는 받지 않는다(다시 쓰인다 — 주면 거절). 생략하면 사실상 main 이다 — 버스 호출에는 "부른 창"이 없어 셸이 고른 목적지 창의 해시를 읽고, 그 값이 main 으로 떨어진다.',
      },
    },
  },
  // ★Ctrl+Tab(D-8)★: 사람 키·클릭과 동일 command 경로(§5). 활성 · 탭 차례는 셸 답(list_tabs) 기준이다.
  run: async args => {
    const tabs = await listTabsOf('tab.next', resolveWindow('tab.next', args))
    if (tabs.tabs.length <= 1) return
    const idx = tabs.tabs.findIndex(t => t.id === tabs.active)
    const next = tabs.tabs[idx < 0 ? 0 : (idx + 1) % tabs.tabs.length]
    return useViewStore.getState().switchTab(tabs.label, next.id)
  },
})

register({
  id: 'tab.rename',
  title: t('tab.rename'),
  category: 'tab',
  // ★rename(ADR-0057)★: 사람 더블클릭 인라인 편집 = LLM 한 표면(§5). 빈/공백 name 은 invoke 전에 throw
  //   (백엔드로 흘려보내지 않음 — TabBar 경계와 동일 정규화). view 는 백엔드 view_owner 파생이라 window 불필요.
  run: args => {
    const view = args?.view
    if (typeof view !== 'string' || view.length === 0) throw new Error('[tab.rename] view(탭 id) 필요')
    const name = args?.name
    if (typeof name !== 'string') throw new Error('[tab.rename] name(문자열) 필요')
    const trimmed = name.trim()
    if (trimmed.length === 0) throw new Error('[tab.rename] name 은 공백일 수 없음')
    return useViewStore.getState().renameTab(view, trimmed)
  },
})

register({
  id: 'window.create',
  title: t('window.create'),
  category: 'window',
  // 빈 새 창(빈 탭 1개) 생성(D-6) → {window, window_id, number} 반환(셸 버스 `window.create` 의 답과 같은 셋 — TRD
  //   S26-window-chrome §5-3).
  run: () => useViewStore.getState().createWindow(),
})

register({
  id: 'window.close',
  title: t('window.close'),
  category: 'window',
  // main 은 백엔드가 거부(hide only, 불변식 4).
  run: args => useViewStore.getState().closeWindow(resolveWindow('window.close', args)),
})

/** tab/slot 는 백엔드 ViewId/slot id(Uuid) 라 형식이 맞아야 한다. */
const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i

/**
 * ★spawn_into 인자 정규화(FIX 5)★: tab/slot 는 "있으면 UUID 문자열, 없으면 미지정(null)"이다. 이전엔
 * 잘못된 타입(숫자 등)을 조용히 null 로 강등해 "스폰은 됐는데 엉뚱한 곳에 배치"로 이어졌다 → present-but-invalid
 * 는 invoke **전에** throw 해 side-effecting 스폰을 막는다. 부재(undefined/null)는 정상("미지정").
 */
function optionalUuidArg(v: unknown, name: string): string | null {
  if (v === undefined || v === null) return null
  if (typeof v !== 'string' || !UUID_RE.test(v)) {
    throw new Error(`[agent.spawnInto] ${name} 는 UUID 문자열이어야 함(받음: ${JSON.stringify(v)})`)
  }
  return v
}

// SlotContent 유니온 태그 화이트리스트(set_slot_content 인자 검증 — ADR-0060/0063).
// ★순서가 곧 선언 철자다★ — 호출자가 어떤 대소문자로 쓰든 백엔드로 나가는 태그는 이 배열의 원소로
//   접힌다(사용자 결정 2026-09-23 — 같은 낱말이 CLI 로는 되고 이 표면으로는 안 되던 어긋남 제거).
const SLOT_CONTENT_TYPES = ['empty', 'agent', 'agent_list', 'preset_palette', 'usage'] as const

const USAGE_SHOW_FIELDS = ['show_claude', 'show_codex'] as const

function findSlotContent(node: LayoutNode, slotId: string): SlotContent | null {
  if (node.type === 'slot') return node.id === slotId ? node.content : null
  return findSlotContent(node.a, slotId) ?? findSlotContent(node.b, slotId)
}

/**
 * 사용량 슬롯의 빠진 show 칸 = 그 슬롯이 이미 사용량이면 지금 값(TRD §3 #20). 버스의 `layout.setSlotContent` 는 셸이 락
 * 안에서 같은 병합을 하므로, 이 길도 같게 두어야 두 LLM 길이 같은 결과를 낸다. 이미 사용량이 아니거나 그 뷰를 아직
 * 모르면 빠진 채로 넘긴다 — 셸 역직렬화가 빠진 칸을 true 로 읽는다(`SlotContent::Usage` serde 기본값).
 *
 * ★기준은 웹뷰가 받아 둔 레이아웃이다★ — 셸의 권위 값과 그사이 어긋날 수 있다(방송이 닿기 전의 연속 호출). 락 안의
 *   원자 병합은 이 길엔 없고 셸의 `set_usage_slot`(버스와 사용량 토글 `usageSlot.toggle*` 가 탄다)에 있다. ★그래서
 *   이 길(`__engramCmd`·cdp)로 사용량 content 를 보내는 LLM 은 show_claude · show_codex 를 둘 다 준다★ — 빠진 칸을
 *   지금 값으로 지키려면 버스의 `layout.setSlotContent` 를 쓴다. 이 안내를 `prompts/engram-help.md` 에 두지 않는 이유 =
 *   그 화면의 독자는 창 JS 를 못 부르는 CLI 에이전트다.
 */
function mergeUsageShows(viewId: string, slotId: string, content: Record<string, unknown>): Record<string, unknown> {
  const layout = useViewStore.getState().layouts[viewId]?.layout
  const current = layout ? findSlotContent(layout, slotId) : null
  if (current?.type !== 'usage') return content
  const merged = { ...content }
  for (const field of USAGE_SHOW_FIELDS) if (merged[field] === undefined) merged[field] = current[field]
  return merged
}

/**
 * ★SlotContent variant 형태 검증(FIX LOW)★: 태그(type)만 화이트리스트로 걸면 `{type:'agent'}` 처럼
 * agent_id 가 빠진 malformed 값이 레지스트리를 통과해 Rust 역직렬화에서야 늦게 터진다(오배치 진단 지연).
 * variant 별 필수 필드를 레지스트리 경계에서 미리 검증해 잘못된 값은 invoke **전에** throw 한다
 * (agent.spawnInto 의 optionalUuidArg 와 동형 — side-effecting invoke 전 loud fail). tag 는 이미
 * SLOT_CONTENT_TYPES 로 검증된 상태로 들어온다.
 */
function validateSlotContent(content: { type: string } & Record<string, unknown>): void {
  switch (content.type) {
    case 'agent':
      if (typeof content.agent_id !== 'string' || content.agent_id.length === 0) {
        throw new Error(
          `[layout.setSlotContent] agent variant 는 string agent_id 필요(받음: ${JSON.stringify(content)})`,
        )
      }
      break
    case 'empty':
    case 'agent_list':
    case 'preset_palette':
      // 추가 필드 없는 unit variant — tag 만 맞으면 통과(여분 필드는 백엔드가 무시).
      break
    case 'usage':
      // 빠진 칸은 여기서 채우지 않는다 — 호출부가 그 슬롯의 지금 값으로 채운다(`mergeUsageShows`).
      for (const field of USAGE_SHOW_FIELDS) {
        if (content[field] !== undefined && typeof content[field] !== 'boolean') {
          throw new Error(
            `[layout.setSlotContent] usage variant 의 ${field} 는 boolean 이어야 함(받음: ${JSON.stringify(content)})`,
          )
        }
      }
      break
    // default 는 도달 불가(호출부에서 SLOT_CONTENT_TYPES 로 이미 걸러짐) — 방어만.
  }
}

register({
  id: 'layout.setSlotContent',
  title: t('slot.setContent'),
  category: 'layout',
  // ★set_slot_content(ADR-0063)★: 사람 우클릭 = LLM 한 표면(§5).
  run: args => {
    const viewId = args?.viewId
    const slotId = args?.slotId
    const raw = args?.content
    if (typeof viewId !== 'string') throw new Error('[layout.setSlotContent] viewId 필요')
    if (typeof slotId !== 'string') throw new Error('[layout.setSlotContent] slotId 필요')
    const type = matchDeclaredSpelling((raw as { type?: unknown } | undefined)?.type, SLOT_CONTENT_TYPES)
    if (!type) {
      throw new Error(
        `[layout.setSlotContent] content.type 이 SlotContent variant 여야 함(받음: ${JSON.stringify(raw)})`,
      )
    }
    // 접은 태그를 도로 얹는다 — 여분 필드(agent_id 등)는 호출자가 준 그대로 통과시키고 태그만 갈아낀다.
    const content = { ...(raw as Record<string, unknown>), type }
    validateSlotContent(content)
    const sent = type === 'usage' ? mergeUsageShows(viewId, slotId, content) : content
    return useViewStore.getState().setSlotContent(viewId, slotId, sent as SlotContent)
  },
})

register({
  id: 'agent.spawnInto',
  title: t('agent.spawnInto'),
  category: 'agent',
  // ★spawn_into(D-7, TRD §6 G9)★: 스폰(데몬) + 탭 생성(필요 시) + 슬롯 배정을 한 방으로 조립하는 백엔드
  //   합성 command 를 직접 invoke 한다(store 상태 없이 backend 권위 — ADR-0057). 성공 시 새 AgentId 반환.
  //   슬롯 정책·실패 가시성은 backend 가 강제.
  //   ★FIX 5★: tab/slot 은 invoke 전에 UUID 형식 검증(잘못된 값 → 스폰 전 throw, 오배치 방지).
  run: args => {
    const cwd = args?.cwd
    if (typeof cwd !== 'string' || cwd.length === 0) throw new Error('[agent.spawnInto] cwd 필요')
    const tab = optionalUuidArg(args?.tab, 'tab')
    const slot = optionalUuidArg(args?.slot, 'slot')
    const backend = typeof args?.backend === 'string' ? args.backend : null
    return invoke<string>('spawn_into', {
      window: resolveWindow('agent.spawnInto', args),
      tab,
      slot,
      backend,
      cwd,
    })
  },
})
