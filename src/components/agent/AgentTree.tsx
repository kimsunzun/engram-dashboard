import { useRef, useEffect, useState } from 'react'
import { Tree, type NodeRendererProps } from 'react-arborist'
import { useAgentStore } from '../../store/agentStore'
import { useSlotStore } from '../../store/slotStore'
import { ptyApi } from '../../api/ptyApi'

interface AgentTreeProps {
  /**
   * 이 트리가 들어있는 슬롯 id. 있으면 그 슬롯(자기 자신)에는 배치를 금지한다 —
   * 슬롯 안 트리에서 클릭이 자기 슬롯을 터미널로 덮어 트리가 증발하는 자기파괴 루프 방지(consult).
   */
  sourceSlotId?: number
}

type AgentNodeData = {
  id: string
  name: string
  status: string
  canInterrupt: boolean
}

/**
 * 우클릭 메뉴 상태. react-arborist는 가상화로 row가 unmount될 수 있으므로
 * NodeApi 객체를 들고 있지 않고 primitive snapshot만 저장한다(consult).
 */
type NodeMenu = { x: number; y: number; agentId: string; name: string; canInterrupt: boolean }

function statusColor(status: string): string {
  switch (status) {
    case 'Running': return 'var(--accent)'
    case 'Exiting': return '#f5a623'
    case 'Failed':  return '#ff4444'
    default:        return 'var(--text-muted)'
  }
}

export default function AgentTree({ sourceSlotId }: AgentTreeProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const menuRef = useRef<HTMLDivElement>(null)
  const [dimensions, setDimensions] = useState({ width: 200, height: 400 })
  const [menu, setMenu] = useState<NodeMenu | null>(null)
  const agents = useAgentStore(s => s.agents)
  const selectedAgentId = useAgentStore(s => s.selectedAgentId)
  const setSelectedAgent = useAgentStore(s => s.setSelectedAgent)
  // 배치는 store 액션 직접 호출이 아니라 dispatch(단일 제어 표면, §5)를 거친다.
  const dispatch = useSlotStore(s => s.dispatch)
  const focusedSlotId = useSlotStore(s => s.focusedSlotId)

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

  const treeData: AgentNodeData[] = agents.map(a => ({
    id: a.id,
    name: a.name || a.id.slice(0, 8),
    status: a.status.type,
    canInterrupt: a.capabilities?.control?.interrupt ?? false,
  }))

  // NodeRenderer를 컴포넌트 안에 두어 dispatch/selectedAgent/메뉴 핸들러를 클로저로 접근.
  const NodeRenderer = ({ node, style }: NodeRendererProps<AgentNodeData>) => (
    <div
      style={{
        ...style,
        display: 'flex',
        alignItems: 'center',
        gap: '4px',
        padding: '0 8px',
        cursor: 'pointer',
        background: selectedAgentId === node.data.id ? 'color-mix(in srgb, var(--accent) 15%, transparent)' : 'transparent',
        fontFamily: 'var(--font-ui)',
        fontSize: '12px',
        color: 'var(--text)',
        userSelect: 'none',
      }}
      // 클릭 = 선택만(selectOnly). 배치/종료/중단은 우클릭 메뉴로.
      onClick={() => setSelectedAgent(node.data.id)}
      onContextMenu={e => {
        e.preventDefault()
        e.stopPropagation()
        setSelectedAgent(node.data.id)
        setMenu({
          x: e.clientX,
          y: e.clientY,
          agentId: node.data.id,
          name: node.data.name,
          canInterrupt: node.data.canInterrupt,
        })
      }}
    >
      <span style={{ color: statusColor(node.data.status), fontSize: '10px', marginLeft: '4px' }}>●</span>
      <span>{node.data.name}</span>
      <span style={{ marginLeft: 'auto', color: 'var(--text-muted)', fontSize: '10px' }}>{node.data.status}</span>
    </div>
  )

  const menuItems = menu
    ? [
        {
          label: '포커스 슬롯에 배치',
          // 자기 슬롯에 배치하면 트리가 터미널로 덮여 사라진다 → disable.
          disabled: sourceSlotId !== undefined && focusedSlotId === sourceSlotId,
          action: () => dispatch({ kind: 'assignAgent', slotId: focusedSlotId, agentId: menu.agentId }),
        },
        {
          label: '중단(작업만 멈춤)',
          disabled: !menu.canInterrupt, // capability 미지원이면 회색 처리(§2 capability 분기)
          action: () => ptyApi.interruptAgent(menu.agentId).catch(e => console.error('[interrupt]', e)),
        },
        {
          label: '종료',
          disabled: false,
          action: () => ptyApi.killAgent(menu.agentId).catch(e => console.error('[kill]', e)),
        },
      ]
    : []

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
