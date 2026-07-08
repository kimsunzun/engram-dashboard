import { useRef, useEffect, useState } from 'react'
import { Tree, type NodeRendererProps } from 'react-arborist'
import { useAgentStore } from '../../store/agentStore'
import { useViewStore, selectActiveView } from '../../store/viewStore'
import { agentClient } from '../../api/clientFactory'
import { refreshProfiles } from '../../store/eventBus'
import { mergeTreeNodes, type AgentTreeNode } from './mergeTreeNodes'

// ★self-배치 가드 개념 메모(옛 sourceSlotId prop 제거 — Brick 1)★: in-slot 트리(슬롯 안에 뜬 트리)를
// 되살릴 땐 viewStore focused slot(UUID) 기준 self-배치 가드를 *재도입*해야 한다 — 트리가 자기 슬롯을
// 터미널로 덮어 트리가 증발하는 자기파괴 루프 방지. 지금은 트리가 사이드바/전용 창에만 뜨고 유일 caller 였던
// 옛 LayoutRenderer 가 삭제돼 이 가드가 필요 없어 prop 을 뺐다.

// 트리 노드 데이터 = 머지 결과(running ∪ reserved). mergeTreeNodes 와 동일 형태.
type AgentNodeData = AgentTreeNode

/**
 * 우클릭 메뉴 상태. react-arborist는 가상화로 row가 unmount될 수 있으므로
 * NodeApi 객체를 들고 있지 않고 primitive snapshot만 저장한다(consult).
 */
type NodeMenu = {
  x: number
  y: number
  agentId: string
  name: string
  kind: 'running' | 'reserved'
  canInterrupt: boolean
}

function statusColor(status: string): string {
  switch (status) {
    case 'Running': return 'var(--accent)'
    case 'Exiting': return '#f5a623'
    case 'Failed':  return '#ff4444'
    case 'Reserved': return 'var(--text-muted)' // 깡통: 흐릿한 회색(미spawn)
    default:        return 'var(--text-muted)'
  }
}

export default function AgentTree() {
  const containerRef = useRef<HTMLDivElement>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const [dimensions, setDimensions] = useState({ width: 200, height: 400 })
  const [menu, setMenu] = useState<NodeMenu | null>(null)
  // activate/delete in-flight 가드 — 같은 id 진행 중이면 재호출(더블클릭 연타·메뉴 재실행) 무시(MAJOR-2).
  const [busyIds, setBusyIds] = useState<Set<string>>(new Set())
  // 액션 실패 메시지 — 토스트/StatusBar 시스템이 없어 노드 옆 인라인 표시로 사용자 피드백(MAJOR-3).
  const [errorById, setErrorById] = useState<Record<string, string>>({})
  const agents = useAgentStore(s => s.agents)
  const profiles = useAgentStore(s => s.profiles)
  const selectedAgentId = useAgentStore(s => s.selectedAgentId)
  const setSelectedAgent = useAgentStore(s => s.setSelectedAgent)

  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    const ro = new ResizeObserver(entries => {
      const { width, height } = entries[0].contentRect
      setDimensions({ width: Math.floor(width), height: Math.floor(height) })
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [])

  // 메뉴 열렸을 때 바깥 클릭으로 닫기. 메뉴 내부 클릭은 닫지 않는다(SlotContextMenu와 동일 가드).
  // 가드가 없으면 항목 클릭의 mousedown이 먼저 메뉴를 닫아 onClick 액션이 실행되지 않는다(리뷰 [높음]).
  useEffect(() => {
    if (!menu) return
    const h = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) setMenu(null)
    }
    document.addEventListener('mousedown', h)
    return () => document.removeEventListener('mousedown', h)
  }, [menu])

  // 예약(프로필) ∪ 실행중(agents) 머지 — 순수 함수로 추출(ADR-0018, mergeTreeNodes.test).
  const treeData: AgentNodeData[] = mergeTreeNodes(profiles, agents)

  // in-flight 마킹 헬퍼 — 진행 중이면 false 반환(재호출 무시), 시작 가능하면 마킹하고 true.
  const beginInFlight = (agentId: string): boolean => {
    if (busyIds.has(agentId)) return false
    setBusyIds(prev => new Set(prev).add(agentId))
    return true
  }
  const endInFlight = (agentId: string) => {
    setBusyIds(prev => {
      const next = new Set(prev)
      next.delete(agentId)
      return next
    })
  }
  const setError = (agentId: string, msg: string) =>
    setErrorById(prev => ({ ...prev, [agentId]: msg }))
  const clearError = (agentId: string) =>
    setErrorById(prev => {
      if (!(agentId in prev)) return prev
      const next = { ...prev }
      delete next[agentId]
      return next
    })

  // 예약 노드 활성화: spawnProfile 후 프로필 목록 refetch(실행중 전환은 agent-list-updated 가 처리).
  const activateReserved = (agentId: string) => {
    if (!beginInFlight(agentId)) return // in-flight 가드(MAJOR-2)
    clearError(agentId)
    agentClient
      .spawnProfile(agentId, false)
      .then(() => refreshProfiles())
      .catch(e => {
        console.error('[spawnProfile]', e)
        setError(agentId, `활성화 실패: ${String(e)}`) // 사용자 피드백(MAJOR-3)
      })
      .finally(() => endInFlight(agentId))
  }

  // 예약 취소(깡통 삭제) = deleteProfile 후 refetch.
  const deleteReserved = (agentId: string) => {
    if (!beginInFlight(agentId)) return // in-flight 가드(MAJOR-2)
    clearError(agentId)
    agentClient
      .deleteProfile(agentId)
      .then(() => refreshProfiles())
      .catch(e => {
        console.error('[deleteProfile]', e)
        setError(agentId, `삭제 실패: ${String(e)}`) // 사용자 피드백(MAJOR-3)
      })
      .finally(() => endInFlight(agentId))
  }

  // NodeRenderer를 컴포넌트 안에 두어 dispatch/selectedAgent/메뉴 핸들러를 클로저로 접근.
  const NodeRenderer = ({ node, style }: NodeRendererProps<AgentNodeData>) => {
    const isReserved = node.data.kind === 'reserved'
    const isBusy = busyIds.has(node.data.id)
    const err = errorById[node.data.id]
    return (
      <div
        style={{
          ...style,
          display: 'flex',
          alignItems: 'center',
          gap: '4px',
          padding: '0 8px',
          cursor: isBusy ? 'wait' : 'pointer',
          background: selectedAgentId === node.data.id ? 'color-mix(in srgb, var(--accent) 15%, transparent)' : 'transparent',
          fontFamily: 'var(--font-ui)',
          fontSize: '12px',
          // 예약(깡통)은 흐리게 — 미spawn 시각 구분(ADR-0018).
          color: err ? '#ff4444' : isReserved ? 'var(--text-muted)' : 'var(--text)',
          fontStyle: isReserved ? 'italic' : 'normal',
          opacity: isBusy ? 0.6 : 1, // in-flight 시각 표시(MAJOR-2)
          userSelect: 'none',
        }}
        // 클릭 = 선택만(selectOnly). 배치/종료/중단은 우클릭 메뉴로.
        onClick={() => setSelectedAgent(node.data.id)}
        // 더블클릭: 예약 노드 → 활성화(spawn). 실행중 노드 → no-op(기존 동작 유지).
        // 진행 중이면 activateReserved 내부 가드가 무시한다.
        onDoubleClick={() => {
          if (node.data.kind === 'reserved') activateReserved(node.data.id)
        }}
        title={err ?? (isReserved ? '더블클릭으로 활성화(spawn)' : undefined)}
        onContextMenu={e => {
          e.preventDefault()
          e.stopPropagation()
          setSelectedAgent(node.data.id)
          setMenu({
            x: e.clientX,
            y: e.clientY,
            agentId: node.data.id,
            name: node.data.name,
            kind: node.data.kind,
            canInterrupt: node.data.canInterrupt,
          })
        }}
      >
        <span style={{ color: statusColor(node.data.status), fontSize: '10px', marginLeft: '4px' }}>{isReserved ? '○' : '●'}</span>
        <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{node.data.name}</span>
        <span style={{ marginLeft: 'auto', color: err ? '#ff4444' : 'var(--text-muted)', fontSize: '10px', flexShrink: 0 }}>
          {err ? '실패' : isBusy ? '...' : isReserved ? '대기' : node.data.status}
        </span>
      </div>
    )
  }

  // 예약(깡통)과 실행중은 가능한 동작이 다르다 → 메뉴 항목을 kind 로 분기.
  const menuItems = !menu
    ? []
    : menu.kind === 'reserved'
    ? [
        {
          // 진행 중(spawn/delete)이면 회색 — 중복 호출 방지(MAJOR-2). 내부 가드도 이중 방어.
          label: '활성화(spawn)',
          disabled: busyIds.has(menu.agentId),
          action: () => activateReserved(menu.agentId),
        },
        {
          label: '예약 취소(삭제)',
          disabled: busyIds.has(menu.agentId),
          action: () => deleteReserved(menu.agentId),
        },
      ]
    : [
        {
          // ADR-0035: 이 액션은 이제 살아있는 viewStore(백엔드 권위) 경로를 쓴다. 활성 뷰의 포커스
          // 슬롯에 assign_agent invoke → emit → 캔버스 렌더. (옛 slotStore dispatch는 죽은 경로였음.)
          label: '포커스 슬롯에 배치',
          disabled: false, // 트리는 항상 활성 뷰의 포커스 슬롯에 배치(in-slot self-배치 가드는 위 메모 참조)
          action: () => {
            const vs = useViewStore.getState()
            const viewId = vs.activeViewId
            const slotId = selectActiveView(vs)?.focusedSlotId
            const aid = menu.agentId               // menu 는 곧 닫히니 async 전에 캡처
            if (!viewId || !slotId) {
              setError(aid, '배치 실패: 활성 뷰/포커스 슬롯 없음')
              return
            }
            clearError(aid)
            void vs.assignAgent(viewId, slotId, aid).catch(e => {
              console.error('[assignAgent]', e)
              setError(aid, `배치 실패: ${String(e)}`)   // 사용자 피드백(MAJOR-3 패턴)
            })
          },
        },
        {
          label: '중단(작업만 멈춤)',
          disabled: !menu.canInterrupt, // capability 미지원이면 회색 처리(§2 capability 분기)
          action: () => agentClient.interruptAgent(menu.agentId).catch(e => console.error('[interrupt]', e)),
        },
        {
          label: '종료',
          disabled: false,
          action: () => agentClient.killAgent(menu.agentId).catch(e => console.error('[kill]', e)),
        },
      ]

  return (
    <div ref={containerRef} style={{ flex: 1, overflow: 'hidden', minHeight: 0, height: '100%' }}>
      {treeData.length === 0 ? (
        <div style={{ height: '100%', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          <span style={{ color: 'var(--text-muted)', fontFamily: 'var(--font-ui)', fontSize: '12px' }}>
            에이전트 없음
          </span>
        </div>
      ) : (
        <Tree<AgentNodeData>
          data={treeData}
          width={dimensions.width}
          height={dimensions.height}
          rowHeight={24}
          indent={12}
          disableEdit
          disableDrag
          disableDrop
        >
          {NodeRenderer}
        </Tree>
      )}
      {menu && (
        <div
          ref={menuRef}
          style={{
            position: 'fixed',
            top: menu.y,
            left: menu.x,
            background: 'var(--bg-secondary)',
            border: '1px solid var(--border)',
            borderRadius: '4px',
            zIndex: 1000,
            minWidth: '150px',
            boxShadow: '0 2px 8px rgba(0,0,0,0.3)',
            fontFamily: 'var(--font-ui)',
            fontSize: '12px',
          }}
        >
          {menuItems.map(item => (
            <div
              key={item.label}
              style={{
                padding: '6px 12px',
                cursor: item.disabled ? 'default' : 'pointer',
                color: item.disabled ? 'var(--text-muted)' : 'var(--text)',
                opacity: item.disabled ? 0.5 : 1,
              }}
              onMouseEnter={e => { if (!item.disabled) e.currentTarget.style.background = 'color-mix(in srgb, var(--accent) 20%, transparent)' }}
              onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
              onClick={e => { e.stopPropagation(); if (!item.disabled) { item.action(); setMenu(null) } }}
            >{item.label}</div>
          ))}
        </div>
      )}
    </div>
  )
}
