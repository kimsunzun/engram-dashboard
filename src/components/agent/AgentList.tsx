//! AgentList — 실행중 에이전트 ∪ 예약(Reserved) 프로필을 계층(트리)으로 그린다(ADR-0072 / ADR-0062 상태
//! 글리프 / ADR-0018 머지).
//!
//! ★react-arborist 부활(ADR-0072)★: 평면 목록에서 트리로 복귀 — parent_id 로 자식을 부모 밑에 중첩(1단).
//!   머지+계층화는 순수 함수 mergeTreeNodes(running ∪ reserved → forest)가 담당하고, 여기선 <Tree>가
//!   들여쓰기·접기/펼치기·드래그 재부모화를 준다. 각 행 = [토글][glyph][name]. 드래그로 onMove →
//!   reparentProfile command(§5 — 사람 드래그·LLM 호출이 같은 핸들). 낙관 갱신 없음: 백엔드 broadcast 가
//!   목록을 새로 그린다(rename 과 동형). 행 클릭·더블클릭·우클릭 메뉴·인라인 rename·in-flight 가드는 평면
//!   목록과 동일 로직을 NodeRenderer 안에서 그대로 쓴다(리그레션 금지).
//!
//! ★상태 = 색이 아니라 모양(글리프)★(ADR-0062): e-ink(흑백)에서도 상태가 구분되도록 색 리터럴 대신
//! statusGlyph 로 모양을 고른다. 5-glyph 어휘 중 백엔드가 실제 구분 가능한 3개(●/◻/✗)만 점등, ○(유휴)·
//! ◐(입력대기)는 어휘로만 정의(◐ 는 백엔드 신호 없어 절대 점등 안 함).
//!
//! ★표시명 = display_name override ?? cwd basename(프론트 파생, ADR-0061 리치화)★: 프로필 표시명 override
//! (display_name)가 있으면 그대로, 없으면 cwd 의 마지막 세그먼트를 쓴다(공용 basename 유틸 — PresetPalette
//! 표시명과 단일 출처). cwd 미노출(요구: name 만, cwd 표시 안 함). 이름 변경 = RenameProfile command.
//!
//! ★행(ROW) 우클릭 메뉴만 소유(ADR-0064)★: 옛 pane 배경 메뉴("에이전트 생성" 프리셋 픽커) + pane
//! stopPropagation 은 제거됐다 — pane 배경 우클릭은 상위 통합 슬롯 메뉴로 버블(agent_list 전용
//! "에이전트 생성" = agentlist.createAgent command + 공통 슬롯 ops). 행 우클릭만 stopPropagation 으로
//! 가로채 item-targeted 메뉴(활성화/열기/종료/이름변경/삭제)를 띄운다(VS Code view/item/context 결).
//! 조작은 agentClient / viewStore(백엔드 권위 invoke→emit)로만 흐른다 — raw invoke/ptyApi 없음(ADR-0011).
//! 이름변경 = RenameProfile command(ADR-0061 리치화, 인라인 편집). 재시작은 대응 command 부재 → "준비 중" 비활성.

import { useEffect, useRef, useState } from 'react'
import { Tree, type NodeRendererProps } from 'react-arborist'

import { agentClient } from '../../api/clientFactory'
import { useAgentStore } from '../../store/agentStore'
import { currentViewId, selectView, useViewStore } from '../../store/viewStore'
import { refreshProfiles } from '../../store/eventBus'
import { basename } from '../../util/basename'
import { mergeTreeNodes, type AgentTreeNode } from './mergeTreeNodes'
import { selectOpenTarget } from './selectOpenTarget'
import { t } from '../../i18n'

/**
 * 상태 → 글리프(모양) 매핑 — PURE(외부 의존 0, ADR-0062). 색이 아닌 모양이 상태를 담아 e-ink 에서도
 * 구분된다. 5-glyph 어휘를 전부 정의하되 현 백엔드가 구분 가능한 3개만 실제 점등한다.
 *
 * 매핑(ADR-0062):
 *   - Running               → ● (작업중)
 *   - Exiting/Exited/Killed  → ◻ (멈춤 — Exiting 은 terminal 직전 전이)
 *   - Failed                → ✗ (에러)
 *   - Reserved(프론트 합성)   → ○ (유휴/미spawn 깡통)
 *   - 그 외(미지 status)      → ○ (안전 degrade — 빈 칸 방지)
 *
 * ★◐(입력대기)는 어휘로만 존재 — 절대 점등하지 않는다★: 백엔드가 "입력 대기" 신호를 내지 않으므로
 *   이 함수는 ◐ 를 반환하는 분기가 없다(ADR-0062 — 미점등은 결함이 아니라 의도). 백엔드가 신호를 낼 때
 *   이 함수에 분기를 추가하는 것이 정규 경로.
 */
export function statusGlyph(status: string): string {
  switch (status) {
    case 'Running':
      return '●' // 작업중
    case 'Exiting':
    case 'Exited':
    case 'Killed':
      return '◻' // 멈춤(종료/전이)
    case 'Failed':
      return '✗' // 에러
    case 'Reserved':
      return '○' // 유휴(미spawn 깡통)
    default:
      return '○' // 미지 status → 유휴로 degrade(빈 글리프 방지)
  }
}

/** 행 우클릭 메뉴 — react-arborist 가상화로 row 가 unmount 될 수 있어 NodeApi 대신 primitive snapshot 만 든다. */
type RowMenu = {
  x: number
  y: number
  agentId: string
  kind: 'running' | 'reserved'
}

/** react-arborist 행 높이·들여쓰기(px). 옛 AgentTree 와 동일 값(리그레션 없이 부활). */
const ROW_HEIGHT = 24
const INDENT = 12

export default function AgentList() {
  const rowMenuRef = useRef<HTMLDivElement>(null)
  const [rowMenu, setRowMenu] = useState<RowMenu | null>(null)
  // ★react-arborist(react-window)는 명시 width/height 를 요구★(가상화 스크롤). ResizeObserver 로 컨테이너
  //   실측을 추적하되, jsdom(테스트)엔 ResizeObserver 가 없어 콜백이 안 돌 수 있으므로 초기값을 비-0 으로
  //   둬 트리가 첫 렌더부터 행을 그린다(옛 AgentTree 동형 — 테스트가 data-agent-row 를 관측).
  const treeContainerRef = useRef<HTMLDivElement>(null)
  const [dimensions, setDimensions] = useState({ width: 240, height: 400 })
  // ★ref = 권위적 double-fire 가드, state(busyIds) = 시각(disabled/opacity)★ (PresetPalette 패턴 동형):
  //   useState 가드만으로는 re-render commit 전 두 번째 호출이 stale closure 로 busyIds 를 아직 비어있게 읽어
  //   둘 다 통과하는 창이 있다(빠른 더블클릭). ref 는 동기 mutable 이라 같은 tick 두 번째 호출도 즉시 차단한다.
  //   그래서 busyRef 가 실제 중복 발화를 막고, busyIds(state)는 순수 시각 표시(disabled/opacity)로만 병행한다.
  const busyRef = useRef<Set<string>>(new Set())
  const [busyIds, setBusyIds] = useState<Set<string>>(new Set())
  // 액션 실패 메시지 — 토스트/StatusBar 가 없어 행 옆 인라인 표시(AgentTree MAJOR-3 패턴).
  const [errorById, setErrorById] = useState<Record<string, string>>({})

  // ★인라인 편집 로컬 상태(프론트 전용 — 백엔드 권위 이름과 별개의 임시 draft, TabBar 패턴)★:
  //   editingId=편집 중 행 id(없으면 null), draft=입력 중 문자열. 확정(Enter/blur) 시에만 renameProfile.
  const [editingId, setEditingId] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  // ★안정 ref★: 편집 진입 시점에만 정확히 1회 select() — 인라인 콜백 ref 는 매 렌더 재부착돼 타이핑을 깬다(TabBar FIX 1).
  const inputRef = useRef<HTMLInputElement>(null)
  useEffect(() => {
    if (editingId !== null) inputRef.current?.select()
  }, [editingId])

  const agents = useAgentStore(s => s.agents)
  const profiles = useAgentStore(s => s.profiles)
  const selectedAgentId = useAgentStore(s => s.selectedAgentId)
  const setSelectedAgent = useAgentStore(s => s.setSelectedAgent)

  // 예약(프로필) ∪ 실행중(agents) 머지 + parent_id 계층화 — 순수 함수(ADR-0018 + ADR-0072). forest 반환.
  const forest: AgentTreeNode[] = mergeTreeNodes(profiles, agents)
  // 트리 존재/부재·메뉴 stale 판정용 평탄화(1단이라 루트 + 각 children). react-arborist 는 forest 를 직접 순회.
  const flatRows: AgentTreeNode[] = forest.flatMap(n => [n, ...n.children])

  // 컨테이너 실측 추적(react-window 가상화용). jsdom 은 ResizeObserver 미제공 → 초기 비-0 값이 폴백.
  useEffect(() => {
    const el = treeContainerRef.current
    if (!el || typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver(entries => {
      const { width, height } = entries[0].contentRect
      // 0 은 무시(레이아웃 전/언마운트 순간) — 마지막 유효 크기 유지.
      if (width > 0 && height > 0) setDimensions({ width: Math.floor(width), height: Math.floor(height) })
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  // 행 메뉴 바깥 클릭으로 닫기(자기 ref 밖 mousedown 이면 닫는다). 항목 클릭의 mousedown 이 먼저 메뉴를 닫아
  //   onClick 이 무산되는 것을 막기 위해 자기 컨테이너 내부 클릭은 예외(SlotContextMenu 가드 동형).
  useEffect(() => {
    if (!rowMenu) return
    const h = (e: MouseEvent) => {
      const t = e.target as Node
      if (rowMenuRef.current && !rowMenuRef.current.contains(t)) setRowMenu(null)
    }
    document.addEventListener('mousedown', h)
    return () => document.removeEventListener('mousedown', h)
  }, [rowMenu])

  // Escape 로 열린 행 메뉴 닫기 — 열려 있을 때만 리스너를 달고 닫힘/언마운트 시 해제(리스너 누수 방지).
  useEffect(() => {
    if (!rowMenu) return
    const h = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setRowMenu(null)
    }
    document.addEventListener('keydown', h)
    return () => document.removeEventListener('keydown', h)
  }, [rowMenu])

  // ★타깃 사라지면 메뉴 닫기★: rowMenu 상태는 대상 행보다 오래 산다 — 목록이 바뀌어(kill/deleteProfile/
  //   마지막 에이전트 제거로 empty 전이) 대상 agentId 가 트리에서 빠져도 rowMenu 는 그대로 남는다. 대상이
  //   flatRows 에서 사라지면 즉시 null 로 리셋해 떠난 agentId 를 겨눈 메뉴가 절대 렌더되지 않게 한다.
  useEffect(() => {
    if (rowMenu && !flatRows.some(r => r.id === rowMenu.agentId)) setRowMenu(null)
  }, [rowMenu, flatRows])

  // in-flight 시작 — busyRef(동기 권위 가드)로 같은 tick 재호출 즉시 차단, busyIds(state)는 시각용 병행.
  const beginInFlight = (id: string): boolean => {
    if (busyRef.current.has(id)) return false // 동기 권위 가드(re-render 전 두 번째 호출 차단)
    busyRef.current.add(id) // async 진입 전 동기 lock
    setBusyIds(prev => new Set(prev).add(id)) // 시각(disabled/opacity)용 — 가드 아님
    return true
  }
  const endInFlight = (id: string) => {
    busyRef.current.delete(id) // 성공·실패 무관 lock 해제(에러가 UI 영구 잠금 방지)
    setBusyIds(prev => {
      const next = new Set(prev)
      next.delete(id)
      return next
    })
  }
  const setError = (id: string, msg: string) => setErrorById(prev => ({ ...prev, [id]: msg }))
  const clearError = (id: string) =>
    setErrorById(prev => {
      if (!(id in prev)) return prev
      const next = { ...prev }
      delete next[id]
      return next
    })

  // 예약 노드 활성화(spawnProfile) — AgentTree 와 동일 restore UX(리그레션 금지). 성공 시 프로필 refetch.
  const activateReserved = (agentId: string) => {
    if (!beginInFlight(agentId)) return
    clearError(agentId)
    agentClient
      .spawnProfile(agentId, false)
      .then(() => refreshProfiles())
      .catch(e => {
        console.error('[spawnProfile]', e)
        setError(agentId, t('agent.activateFailed', { err: String(e) }))
      })
      .finally(() => endInFlight(agentId))
  }

  // "열기" = 이 에이전트를 main 활성 뷰의 대상 슬롯에 배정(기존 assign 경로 재사용 — AgentTree 와 동일).
  //   agent-tree 창(config)엔 자기 슬롯 캔버스가 없어 currentViewId() 가 main 폴백을 준다(AgentTree 주석 참조).
  //   ★대상 슬롯 = selectOpenTarget(순수 함수)★: 기본은 포커스 슬롯이지만, 포커스가 제어 슬롯(트리/팔레트)이거나
  //   focus=null 인 엣지 상태에선 트리/팔레트를 덮어쓰지 않고 첫 빈 슬롯으로 폴백한다(제어 슬롯 포커스 제외 —
  //   ViewLayoutRenderer click-to-focus 게이트의 안전망). 빈 슬롯도 없으면 배정 안 함(실패 토스트 — 클로버 금지).
  const openInFocusedSlot = (agentId: string) => {
    if (!beginInFlight(agentId)) return // 동기 가드 — 연타로 중복 assign 방지
    const vs = useViewStore.getState()
    const viewId = currentViewId()
    const view = selectView(vs, viewId)
    // 활성 뷰/캐시된 레이아웃 부재 → 조기 실패(활성 뷰 자체가 없음).
    if (!viewId || !view) {
      setError(agentId, t('agent.openFailedNoSlot'))
      endInFlight(agentId) // 조기 반환에도 lock 해제(영구 잠금 방지)
      return
    }
    const slotId = selectOpenTarget(view.layout, view.focusedSlotId)
    // 콘텐츠 슬롯(포커스 또는 빈 슬롯)이 하나도 없음 → 제어/타 에이전트 슬롯 임의 클로버 금지 → 실패 토스트.
    if (!slotId) {
      setError(agentId, t('agent.openFailedNoEmptySlot'))
      endInFlight(agentId)
      return
    }
    clearError(agentId)
    void vs
      .assignAgent(viewId, slotId, agentId)
      .catch(e => {
        console.error('[assignAgent]', e)
        setError(agentId, t('agent.openFailed', { err: String(e) }))
      })
      .finally(() => endInFlight(agentId))
  }

  // 종료(kill) — 동기 가드로 연타 중복 kill 방지. 성공·실패 무관 lock 해제.
  const killAgentGuarded = (agentId: string) => {
    if (!beginInFlight(agentId)) return
    clearError(agentId)
    agentClient
      .killAgent(agentId)
      .catch(e => {
        console.error('[kill]', e)
        setError(agentId, t('agent.killFailed', { err: String(e) }))
      })
      .finally(() => endInFlight(agentId))
  }

  // 예약 취소(삭제) — 예약 프로필을 데몬에서 제거(agentClient.deleteProfile). 다른 UI 로는 stale 예약 프로필을
  //   지울 수 없어 이 항목이 유일 경로(AgentTree reserved-row 의 "예약 취소" 리그레션 복원). 동기 가드 + 성공 시 refetch.
  const cancelReserved = (agentId: string) => {
    if (!beginInFlight(agentId)) return
    clearError(agentId)
    agentClient
      .deleteProfile(agentId)
      .then(() => refreshProfiles())
      .catch(e => {
        console.error('[deleteProfile]', e)
        setError(agentId, t('agent.cancelReservedFailed', { err: String(e) }))
      })
      .finally(() => endInFlight(agentId))
  }

  // ★드래그 재부모화(ADR-0072 §5 — 사람 드래그·LLM 호출이 같은 reparentProfile 핸들)★: react-arborist onMove
  //   가 dropParentId(null=루트) 를 준다. 낙관 갱신 없음 — 백엔드 ReparentProfile → ProfileListUpdated
  //   broadcast 가 forest 를 새로 그린다(rename 동형). invalid move(cycle/self/2단/존재하지 않는 parent)는
  //   백엔드가 Error 로 거부 → 트리는 옛 배치 유지 + 인라인 에러. 동기 가드로 연타 중복 발화 차단.
  const reparent = (childId: string, parentId: string | null) => {
    // no-op 억제: 이미 그 부모(또는 이미 루트)면 발화하지 않는다(불필요 command).
    const node = flatRows.find(r => r.id === childId)
    const currentParentId = forest.find(root => root.children.some(c => c.id === childId))?.id ?? null
    if (!node || currentParentId === parentId) return
    if (!beginInFlight(childId)) return
    clearError(childId)
    agentClient
      .reparentProfile(childId, parentId)
      .then(() => refreshProfiles())
      .catch(e => {
        console.error('[reparentProfile]', e)
        setError(childId, t('agent.reparentFailed', { err: String(e) }))
      })
      .finally(() => endInFlight(childId))
  }

  // 표시명 = display_name override ?? cwd basename(ADR-0061 리치화 — 트리 rename). PresetPalette 와 동일
  //   precedence(override 우선, 없으면 basename 파생). rename 시작·확정 시 이 값을 draft 시드/미변경 판정에 쓴다.
  const displayNameOf = (node: AgentTreeNode): string => node.displayName ?? basename(node.cwd)

  // 편집 진입: 현재 표시명을 draft 로 시드(우클릭 "이름 변경"). 행 메뉴는 호출부에서 닫는다.
  const beginEdit = (node: AgentTreeNode) => {
    setEditingId(node.id)
    setDraft(displayNameOf(node))
  }
  const cancelEdit = () => setEditingId(null)
  // 확정: trim 후 비었거나 현재 표시명과 같으면 no-op(revert), 아니면 renameProfile. 어느 경우든 편집 종료.
  // ★멱등★: editingId 가 이 행이 아니면 즉시 return — Enter 가 언마운트→blur→commitEdit 재발화를 막는다(TabBar 동형).
  //   동기 중복 발화 차단은 beginInFlight(busyRef).
  // ★성공 시 refreshProfiles() — spawnProfile/deleteProfile 과 대칭★: 표시명 반영은 원칙적으로 백엔드
  //   RenameProfile → ProfileListUpdated broadcast 로 온다(낙관 갱신 X). 그러나 rename 만 이 refetch 안전망이
  //   없으면 broadcast 를 놓쳤을 때(재연결 창·이벤트 유실) 이 창만 stale 표시명으로 남는다 — activateReserved/
  //   cancelReserved 는 이미 성공 시 refreshProfiles 로 권위 목록을 다시 끌어와 대칭을 맞춘다. rename 도 같은
  //   안전망을 달아 세 경로를 일관되게 만든다(broadcast 가 정상 도달하면 같은 전체 목록 재적용이라 무해·멱등).
  const commitEdit = (node: AgentTreeNode) => {
    if (editingId !== node.id) return
    const trimmed = draft.trim()
    setEditingId(null)
    if (trimmed.length === 0 || trimmed === displayNameOf(node)) return // 미변경·빈값 → 발화 안 함
    if (!beginInFlight(node.id)) return
    clearError(node.id)
    agentClient
      .renameProfile(node.id, trimmed)
      .then(() => refreshProfiles())
      .catch(e => {
        console.error('[renameProfile]', e)
        setError(node.id, t('agent.renameFailed', { err: String(e) }))
      })
      .finally(() => endInFlight(node.id))
  }

  // 행 우클릭 메뉴 항목 — kind 로 분기. reserved 는 활성화/이름변경/삭제, running 은 열기/종료/이름변경/재시작.
  //   이름변경 = RenameProfile command(ADR-0061 리치화 — 인라인 편집 진입). 재시작은 백엔드 command 부재 →
  //   disabled "준비 중"(날조 금지). disabled 시각 판정은 busyIds(state), 실제 중복 발화 차단은 busyRef 동기 가드.
  //   ★대상 node 조회★: 메뉴는 agentId 만 들고 있으므로 flatRows 에서 찾아 rename 진입에 넘긴다(타깃-gone 은 effect 가 닫음).
  const menuNode = rowMenu ? flatRows.find(r => r.id === rowMenu.agentId) : undefined
  const rowMenuItems: Array<{ label: string; disabled: boolean; action: () => void }> = !rowMenu
    ? []
    : rowMenu.kind === 'reserved'
      ? [
          {
            label: t('agent.rowActivate'),
            disabled: busyIds.has(rowMenu.agentId),
            action: () => activateReserved(rowMenu.agentId),
          },
          // 이름 변경(RenameProfile) — reserved 프로필도 표시명 override 가능(트리 rename, ADR-0061).
          {
            label: t('agent.rowRename'),
            disabled: busyIds.has(rowMenu.agentId) || !menuNode,
            action: () => menuNode && beginEdit(menuNode),
          },
          // ★예약 취소(삭제) — AgentTree reserved-row 리그레션 복원★: 이 항목 외엔 stale 예약 프로필을 지울
          //   UI 가 없다(deleteProfile 유일 경로). 동기 가드는 cancelReserved 내부 busyRef.
          {
            label: t('agent.rowCancelReserved'),
            disabled: busyIds.has(rowMenu.agentId),
            action: () => cancelReserved(rowMenu.agentId),
          },
        ]
      : [
          { label: t('agent.rowOpen'), disabled: busyIds.has(rowMenu.agentId), action: () => openInFocusedSlot(rowMenu.agentId) },
          {
            label: t('agent.rowKill'),
            disabled: busyIds.has(rowMenu.agentId),
            action: () => killAgentGuarded(rowMenu.agentId),
          },
          // 이름 변경(RenameProfile) — ad-hoc(프로필 없는 running)은 rename 대상 부재라 disabled(menuNode 는
          //   있으나 백엔드 프로필이 없어 RenameProfile 이 Error → 발화 자체를 막는 게 아니라, 프로필 있으면
          //   override 저장). 여기선 항목을 열되 백엔드가 no-profile 이면 Error 를 인라인 표시한다(날조 아님).
          {
            label: t('agent.rowRename'),
            disabled: busyIds.has(rowMenu.agentId) || !menuNode,
            action: () => menuNode && beginEdit(menuNode),
          },
          // ★준비 중(백엔드 command 없음)★: 재시작 전용 command 가 protocolClient 에 없다(kill→re-spawn 조합뿐).
          //   날조 금지 — 실제 command 가 생기면 배선한다(ADR-0011).
          { label: t('agent.rowRestart'), disabled: true, action: () => {} },
        ]

  // NodeRenderer — 컴포넌트 안에 두어 select/busy/error/편집 핸들러를 클로저로 접근(옛 AgentTree 동형).
  //   react-arborist 가 style(가상화 위치·높이)·node(NodeApi)·dragHandle 을 준다. 각 행 = [토글][glyph][name].
  const NodeRenderer = ({ node, style, dragHandle }: NodeRendererProps<AgentTreeNode>) => {
    const data = node.data
    const isReserved = data.kind === 'reserved'
    const isBusy = busyIds.has(data.id)
    const err = errorById[data.id]
    // 1단이라 자식 유무 = isInternal(부모). 부모만 토글(펼치기/접기)을 그린다.
    const hasChildren = data.children.length > 0
    return (
      <div
        ref={dragHandle}
        data-agent-row={data.id}
        style={{
          ...style, // react-arborist 가상화 위치/높이(top/height). 들여쓰기는 아래 paddingLeft 로 level 반영.
          display: 'flex',
          alignItems: 'center',
          gap: '6px',
          // level*INDENT + 기본 padding. 토글이 없는 leaf 도 부모와 글리프 정렬이 맞게 토글 폭만큼 고정 확보.
          paddingLeft: `${8 + node.level * INDENT}px`,
          paddingRight: '8px',
          cursor: isBusy ? 'wait' : 'pointer',
          background:
            selectedAgentId === data.id
              ? 'color-mix(in srgb, var(--accent) 15%, transparent)'
              : 'transparent',
          fontFamily: 'var(--font-ui)',
          fontSize: '12px',
          // 예약(깡통)은 흐리게 + 이탤릭 — 미spawn 시각 구분(색 리터럴 없이 muted 변수·기울임).
          color: isReserved ? 'var(--text-muted)' : 'var(--text)',
          fontStyle: isReserved ? 'italic' : 'normal',
          opacity: isBusy ? 0.6 : 1,
          userSelect: 'none',
        }}
        onClick={() => setSelectedAgent(data.id)}
        // 더블클릭: 예약 행 → 활성화(spawn). 실행중 행 → no-op(AgentTree 동작 유지).
        onDoubleClick={() => {
          if (data.kind === 'reserved') activateReserved(data.id)
        }}
        // ★인라인 편집 키(Enter 확정 / Escape 취소)는 입력이 아니라 이 행 div 에서 처리★: react-window
        //   가상화 안에서 조건부로 mount 되는 <input> 의 onKeyDown 은 React 19 이벤트 위임이 2번째 이후
        //   재-mount 되는 root 에 keydown 리스너를 붙이지 못해 synthetic 이 안 뜬다(react-window+jsdom 상호작용
        //   — click/blur/change 는 정상, keydown 만). 항상 렌더되는 이 행 div 에 핸들러를 두면 초기 mount 부터
        //   keydown 이 등록돼 안정적으로 동작한다(input 키는 버블로 여기 올라옴). 편집 중일 때만 stopPropagation
        //   으로 react-arborist 키 네비게이션·전역 키바인딩 누수를 막는다(TabBar 버블 차단과 동형).
        onKeyDown={e => {
          if (editingId !== data.id) return
          e.stopPropagation()
          if (e.key === 'Enter') commitEdit(data)
          else if (e.key === 'Escape') cancelEdit() // 취소(revert — renameProfile 안 부름).
        }}
        title={err ?? (isReserved ? t('agent.doubleClickToActivate') : data.cwd)}
        onContextMenu={e => {
          e.preventDefault()
          e.stopPropagation() // ★행 메뉴가 이긴다(ADR-0064)★: 상위 통합 슬롯 메뉴가 안 뜨게 여기서 멈춘다.
          setSelectedAgent(data.id)
          setRowMenu({ x: e.clientX, y: e.clientY, agentId: data.id, kind: data.kind })
        }}
      >
        {/* 접기/펼치기 토글 — 부모(자식 보유)만 클릭 가능한 ▸/▾. leaf 는 같은 폭의 빈 칸(글리프 정렬 유지). */}
        <span
          data-agent-toggle={hasChildren ? '1' : undefined}
          onClick={e => {
            if (!hasChildren) return
            e.stopPropagation() // 토글은 선택/편집으로 새지 않는다.
            node.toggle()
          }}
          style={{
            width: '10px',
            flexShrink: 0,
            textAlign: 'center',
            fontSize: '9px',
            color: 'var(--text-muted)',
            cursor: hasChildren ? 'pointer' : 'default',
          }}
        >
          {hasChildren ? (node.isOpen ? '▾' : '▸') : ''}
        </span>
        {/* 상태 = 글리프 모양(색 아님, ADR-0062). muted 변수로만 렌더 — 모양이 상태를 담는다. */}
        <span data-agent-glyph="1" style={{ fontSize: '11px', color: 'var(--text-muted)', flexShrink: 0 }}>
          {statusGlyph(data.status)}
        </span>
        {/* 표시명 = display_name override ?? cwd basename(프론트 파생, ADR-0061 리치화). 편집 중이면 인라인 input.
            cwd 는 노출 안 함(title 로만). */}
        {editingId === data.id ? (
          <input
            data-agent-rename-input={data.id}
            value={draft}
            autoFocus
            ref={inputRef}
            onChange={e => setDraft(e.target.value)}
            // ★Enter/Escape 라우팅은 행 div onKeyDown 이 소유★(위 주석 — react-window+React19 keydown 위임
            //   회피). 여기선 라우팅하지 않고, 키는 버블로 행 div 에 올라간다. blur 확정만 입력이 직접 소유.
            onBlur={() => commitEdit(data)}
            onClick={e => e.stopPropagation()}
            onDoubleClick={e => e.stopPropagation()}
            style={{
              // 내용 폭에 맞춤 — TabBar rename input 동형. minWidth/maxWidth 로 상·하한.
              flex: 1,
              minWidth: '3ch',
              maxWidth: '180px',
              font: 'inherit',
              color: 'var(--text)',
              background: 'var(--bg)',
              border: '1px solid var(--accent)',
              borderRadius: '2px',
              padding: '0 2px',
              outline: 'none',
            }}
          />
        ) : (
          <span
            data-agent-name="1"
            // flex:1+minWidth:0 = 행 폭을 채워 ellipsis 가 제대로 걸리게. paddingRight 2px = italic overhang 여유.
            style={{
              flex: 1,
              minWidth: 0,
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
              paddingRight: '2px',
            }}
          >
            {displayNameOf(data)}
          </span>
        )}
        {err && (
          <span style={{ marginLeft: 'auto', color: 'var(--text-muted)', fontSize: '10px', flexShrink: 0 }}>
            {t('agent.rowFailedBadge')}
          </span>
        )}
      </div>
    )
  }

  return (
    // ★구조 = 라벨(고정) 바깥 + 트리만 아래 영역★(PresetPalette/AgentMonitoringPicker 동형, ADR-0053):
    //   라벨은 스크롤 밖 flex 형제로 올려 상단 고정. react-arborist(react-window)는 자기 가상화 스크롤을
    //   소유하므로 ScrollArea 로 감싸지 않는다(스크롤 소유 충돌 회피 — 옛 flat 목록과 다른 점).
    //   ★pane 배경 마커 = 이 바깥 flex 컨테이너(data-agent-list)★: pane 배경 우클릭은 상위 통합 슬롯 메뉴로
    //   버블(ADR-0064 — 옛 자체 배경 메뉴/stopPropagation 제거). 행 우클릭만 행 핸들러가 stopPropagation.
    <div
      data-agent-list="1"
      data-testid="agent-list"
      style={{
        flex: 1,
        minHeight: 0,
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        background: 'var(--bg-secondary)',
      }}
    >
      {/* 슬롯 콘텐츠 라벨(사용자 요청) — 이 슬롯 = 에이전트 트리. 스크롤 밖 flex 형제라 항상 상단 고정. 변수-only. */}
      <div
        data-slot-label="agent-list"
        style={{
          flexShrink: 0,
          padding: '6px 8px',
          borderBottom: '1px solid var(--border)',
          background: 'var(--bg-secondary)',
          color: 'var(--text-muted)',
          fontFamily: 'var(--font-ui)',
          fontSize: '11px',
          fontWeight: 600,
          letterSpacing: '0.03em',
        }}
      >
        {t('agent.treeLabel')}
      </div>
      {/* ★빈 목록 = 트리 대신 flex-1 센터링 div★: 그릴 노드가 없으면 react-arborist 를 마운트하지 않고 안내
          문구를 세로 중앙 정렬(옛 FIX-A 취지 유지 — 빈 상태엔 스크롤/가상화 표면 없음). */}
      {flatRows.length === 0 ? (
        <div
          style={{
            flex: 1,
            minHeight: 0,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            color: 'var(--text-muted)',
            fontFamily: 'var(--font-ui)',
            fontSize: '12px',
          }}
        >
          {t('agent.emptyList')}
        </div>
      ) : (
        // 트리 컨테이너 = 실측 대상(ResizeObserver). react-arborist 가 이 폭·높이 안에서 가상화 스크롤한다.
        <div ref={treeContainerRef} data-testid="agent-tree" style={{ flex: 1, minHeight: 0, overflow: 'hidden' }}>
          <Tree<AgentTreeNode>
            data={forest}
            idAccessor="id"
            childrenAccessor="children"
            width={dimensions.width}
            height={dimensions.height}
            rowHeight={ROW_HEIGHT}
            indent={INDENT}
            openByDefault
            // ★rename 은 우클릭→인라인 input 자체 구현★: react-arborist 내장 edit(onRename) 대신 우리 draft
            //   상태를 쓰므로 내장 편집은 끈다(중복 편집 UI 방지). 선택 다중선택도 불필요 → 단일선택 유지.
            disableEdit
            disableMultiSelection
            // ★드래그 불가 = 프로필 없는 노드(ADR-0072 드롭 가드)★: ad-hoc(SpawnByCwd, 프로필 없는 running)은
            //   reparent 대상 프로필이 없어 백엔드가 no-profile 을 Error 로 거부한다 → 애초에 드래그를 막아
            //   불필요한 실패 커맨드·에러 토스트를 없앤다(reserved·프로필 있는 running 은 드래그 가능).
            disableDrag={(data: AgentTreeNode) => !data.hasProfile}
            // ★1단 상한 + 프로필 유무 + 부모-불변 UI 가드(ADR-0072 · review FIX)★: UI 에서 막는 3가지 —
            //   ① 부모가 루트 아닌 곳(=자식 밑)으로의 드롭, 또는 이미 자식을 가진 노드를 남의 밑으로 드롭(둘 다
            //      2단이 됨). leaf 도 children:[] 라 react-arborist 가 isInternal 로 봐 중앙 드롭이 걸리므로 필요.
            //   ② 프로필 없는 노드(ad-hoc)를 부모로 하는 드롭 — 그 위엔 자식 못 붙임(reparent 대상 프로필 부재).
            //   ③ 현재 부모와 동일한 부모로의 드롭(루트↔루트 reorder 포함) — 백엔드 정렬은 created_at 기반이라
            //      reorder 미지원이다. react-arborist 는 same-parent reorder 를 유효 드롭(커서 표시)으로 주지만
            //      드롭해도 아무 일 없어 혼란 → 유효 드롭처럼 보이지 않게 비활성(onMove 도 이미 no-op 억제).
            //   ★루트 드롭 정규화★: canDrop 은 루트 드롭 때 parentNode 를 내부 루트 pseudo-node(isRoot,
            //     level=-1, data={id:ROOT_ID})로 준다(null 이 아님). 그래서 ②·③ 은 실 부모(isRoot 아님)에만
            //     적용하고, 루트-대상 비교는 dropParentId(루트=null)로 정규화해 판정한다.
            disableDrop={({ parentNode, dragNodes }) => {
              const isRootDrop = parentNode == null || parentNode.isRoot
              // dropParentId: 루트 드롭이면 null, 아니면 실 부모 id(reparent 가 받는 것과 동일 정규화).
              const dropParentId = isRootDrop ? null : parentNode.id
              // 현재 부모 id: 루트 레벨 노드의 parent 는 내부 루트(isRoot)라 null 로 정규화.
              const dragParent = dragNodes[0]?.parent
              const currentParentId = dragParent && !dragParent.isRoot ? dragParent.id : null
              return (
                // ① 실 부모가 루트 아닌 곳(=자식 밑) → 2단 방지.
                (!isRootDrop && parentNode.level > 0) ||
                // ② 실 부모가 프로필 없는 노드(ad-hoc) → 그 위엔 자식 못 붙임.
                (!isRootDrop && !parentNode.data.hasProfile) ||
                // ① 이미 자식을 가진 노드를 남의 밑으로 → 2단 방지.
                dragNodes.some(n => Array.isArray(n.data.children) && n.data.children.length > 0) ||
                // ③ 부모가 안 바뀌는 드롭(same-parent reorder / 루트↔루트) → 유효 드롭처럼 안 보이게.
                currentParentId === dropParentId
              )
            }}
            // ★드래그 재부모화 = onMove(§5)★. dropParentId(null=루트)로 reparentProfile 발화(낙관 갱신 X).
            onMove={({ dragIds, parentId }) => {
              // disableMultiSelection 으로 단일 드래그 전제 — [0]만 처리(다중선택 재활성 시 이 라인 재검토).
              if (dragIds.length !== 1) return
              const childId = dragIds[0]
              if (childId) reparent(childId, parentId)
            }}
          >
            {NodeRenderer}
          </Tree>
        </div>
      )}

      {/* ── 행 우클릭 메뉴 ─────────────────────────────────────────────── */}
      {rowMenu && (
        <div ref={rowMenuRef} style={MENU_STYLE(rowMenu.x, rowMenu.y)}>
          {rowMenuItems.map(item => (
            <div
              key={item.label}
              style={MENU_ITEM_STYLE(item.disabled)}
              onMouseEnter={e => {
                if (!item.disabled) e.currentTarget.style.background = 'color-mix(in srgb, var(--accent) 20%, transparent)'
              }}
              onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
              onClick={e => {
                e.stopPropagation()
                if (!item.disabled) {
                  item.action()
                  setRowMenu(null)
                }
              }}
            >
              {item.label}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

// 메뉴 공통 스타일(SlotContextMenu·AgentTree 인라인 메뉴와 동형 — 변수-only).
function MENU_STYLE(x: number, y: number): React.CSSProperties {
  return {
    position: 'fixed',
    top: y,
    left: x,
    background: 'var(--bg-secondary)',
    border: '1px solid var(--border)',
    borderRadius: '4px',
    zIndex: 1000,
    minWidth: '150px',
    boxShadow: '0 2px 8px rgba(0,0,0,0.3)',
    fontFamily: 'var(--font-ui)',
    fontSize: '12px',
  }
}
function MENU_ITEM_STYLE(disabled: boolean): React.CSSProperties {
  return {
    padding: '6px 12px',
    cursor: disabled ? 'default' : 'pointer',
    color: disabled ? 'var(--text-muted)' : 'var(--text)',
    opacity: disabled ? 0.5 : 1,
  }
}
