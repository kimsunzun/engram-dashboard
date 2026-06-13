import { Allotment } from 'allotment'
import type { LayoutNode } from '../../store/slotStore'
import SlotPane from '../slot/SlotPane'
import TerminalSlot from '../slot/TerminalSlot'
import AgentTree from '../agent/AgentTree'

function nodeKey(node: LayoutNode): string {
  if (node.type === 'slot') return `s${node.id}`
  return `p[${node.children.map(nodeKey).join(',')}]`
}

export default function LayoutRenderer({ node }: { node: LayoutNode }) {
  if (node.type === 'slot') {
    return (
      <SlotPane slotId={node.id}>
        {node.content.kind === 'terminal'
          ? <TerminalSlot agentId={node.content.agentId} />
          : <AgentTree sourceSlotId={node.id} />}
      </SlotPane>
    )
  }
  return (
    <div style={{ height: '100%' }}>
      <Allotment vertical={node.dir === 'vertical'}>
        {node.children.map((child) => (
          <Allotment.Pane key={nodeKey(child)}>
            <LayoutRenderer node={child} />
          </Allotment.Pane>
        ))}
      </Allotment>
    </div>
  )
}
