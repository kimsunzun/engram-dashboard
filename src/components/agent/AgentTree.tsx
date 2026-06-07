import { useRef, useEffect, useState } from 'react'
import { Tree, type NodeRendererProps } from 'react-arborist'
import { useAgentStore, dummyAgents, dummyGroups } from '../../store/agentStore'
import { useSlotStore } from '../../store/slotStore'

type AgentNodeData = {
  id: string
  name: string
  isGroup: boolean
  memberCount?: number
  status?: string
  cost?: string
  children?: AgentNodeData[]
}

const treeData: AgentNodeData[] = dummyGroups.map(g => ({
  id: g.id,
  name: g.name,
  isGroup: true,
  memberCount: g.members.length,
  children: g.members
    .map(mid => dummyAgents.find(a => a.id === mid))
    .filter((a): a is typeof dummyAgents[0] => Boolean(a))
    .map(a => ({ id: a.id, name: a.name, isGroup: false, status: a.status, cost: a.cost })),
}))

function NodeRenderer({ node, style }: NodeRendererProps<AgentNodeData>) {
  const setSelectedAgent = useAgentStore(s => s.setSelectedAgent)
  const selectedAgentId = useAgentStore(s => s.selectedAgentId)
  const { focusedSlotId, assignAgent } = useSlotStore()

  const statusColor =
    node.data.status === 'running' ? 'var(--accent)' :
    node.data.status === 'error'   ? '#ff4444' :
    'var(--text-muted)'

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
        if (node.data.isGroup) {
          node.toggle()
        } else {
          setSelectedAgent(node.id)
          assignAgent(focusedSlotId, node.id)
        }
      }}
    >
      {node.data.isGroup ? (
        <>
          <span style={{ color: 'var(--text-muted)', fontSize: '10px' }}>{node.isOpen ? '▾' : '▸'}</span>
          <span>{node.data.name}</span>
          <span style={{
            marginLeft: 'auto',
            background: 'var(--bg)',
            border: '1px solid var(--border)',
            borderRadius: '8px',
            padding: '0 5px',
            fontSize: '10px',
            color: 'var(--text-muted)',
          }}>{node.data.memberCount}</span>
        </>
      ) : (
        <>
          <span style={{ color: statusColor, fontSize: '10px', marginLeft: '12px' }}>●</span>
          <span>{node.data.name}</span>
          <span style={{ marginLeft: 'auto', color: 'var(--text-muted)', fontSize: '10px' }}>{node.data.cost}</span>
        </>
      )}
    </div>
  )
}

export default function AgentTree() {
  const containerRef = useRef<HTMLDivElement>(null)
  const [dimensions, setDimensions] = useState({ width: 200, height: 400 })

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

  return (
    <div ref={containerRef} style={{ flex: 1, overflow: 'hidden', minHeight: 0 }}>
      <Tree<AgentNodeData>
        data={treeData}
        width={dimensions.width}
        height={dimensions.height}
        rowHeight={24}
        indent={12}
        initialOpenState={{ g1: true }}
        disableEdit
        disableDrag
        disableDrop
      >
        {NodeRenderer}
      </Tree>
    </div>
  )
}
