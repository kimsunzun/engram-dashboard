import { useRef, useEffect, useState } from 'react'
import { Tree, type NodeRendererProps } from 'react-arborist'
import { useAgentStore } from '../../store/agentStore'
import { useSlotStore } from '../../store/slotStore'

type AgentNodeData = {
  id: string
  name: string
  status: string
}

function statusColor(status: string): string {
  switch (status) {
    case 'Running': return 'var(--accent)'
    case 'Exiting': return '#f5a623'
    case 'Failed':  return '#ff4444'
    default:        return 'var(--text-muted)'
  }
}

function NodeRenderer({ node, style }: NodeRendererProps<AgentNodeData>) {
  const setSelectedAgent = useAgentStore(s => s.setSelectedAgent)
  const selectedAgentId = useAgentStore(s => s.selectedAgentId)
  const { focusedSlotId, assignAgent } = useSlotStore()

  return (
    <div
      style={{
        ...style,
        display: 'flex',
        alignItems: 'center',
        gap: '4px',
        padding: '0 8px',
        cursor: 'pointer',
        background: selectedAgentId === node.id ? 'color-mix(in srgb, var(--accent) 15%, transparent)' : 'transparent',
        fontFamily: 'var(--font-ui)',
        fontSize: '12px',
        color: 'var(--text)',
        userSelect: 'none',
      }}
      onClick={() => {
        setSelectedAgent(node.id)
        assignAgent(focusedSlotId, node.id)
      }}
    >
      <span style={{ color: statusColor(node.data.status), fontSize: '10px', marginLeft: '4px' }}>●</span>
      <span>{node.data.name}</span>
      <span style={{ marginLeft: 'auto', color: 'var(--text-muted)', fontSize: '10px' }}>
        {node.data.status}
      </span>
    </div>
  )
}

export default function AgentTree() {
  const containerRef = useRef<HTMLDivElement>(null)
  const [dimensions, setDimensions] = useState({ width: 200, height: 400 })
  const agents = useAgentStore(s => s.agents)

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

  const treeData: AgentNodeData[] = agents.map(a => ({
    id: a.id,
    name: a.id.slice(0, 8),
    status: a.status.type,
  }))

  if (treeData.length === 0) {
    return (
      <div
        ref={containerRef}
        style={{
          flex: 1,
          overflow: 'hidden',
          minHeight: 0,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
        }}
      >
        <span style={{ color: 'var(--text-muted)', fontFamily: 'var(--font-ui)', fontSize: '12px' }}>
          에이전트 없음
        </span>
      </div>
    )
  }

  return (
    <div ref={containerRef} style={{ flex: 1, overflow: 'hidden', minHeight: 0 }}>
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
    </div>
  )
}
