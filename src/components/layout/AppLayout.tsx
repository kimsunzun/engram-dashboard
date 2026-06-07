import { useState } from 'react'
import { Allotment } from 'allotment'
import 'allotment/dist/style.css'
import Sidebar from './Sidebar'
import SlotPane from '../slot/SlotPane'
import TerminalSlot from '../slot/TerminalSlot'
import DiffPanel from '../diff/DiffPanel'
import StatusBar from './StatusBar'

export default function AppLayout() {
  const [sidebarOpen, setSidebarOpen] = useState(true)
  const [diffOpen, setDiffOpen] = useState(false)

  return (
    <div style={{ height: '100%', position: 'relative' }}>
      {!sidebarOpen && (
        <button
          onClick={() => setSidebarOpen(true)}
          style={{
            position: 'absolute',
            top: '50%',
            left: 0,
            transform: 'translateY(-50%)',
            zIndex: 10,
            background: 'var(--bg-secondary)',
            border: '1px solid var(--border)',
            color: 'var(--text-muted)',
            cursor: 'pointer',
            padding: '4px 2px',
            fontSize: '12px',
            borderRadius: '0 4px 4px 0',
          }}
        >▶</button>
      )}
      <Allotment>
        <Allotment.Pane preferredSize={200} minSize={120} visible={sidebarOpen}>
          <Sidebar onToggle={() => setSidebarOpen(false)} />
        </Allotment.Pane>
        <Allotment.Pane>
          <Allotment vertical>
            <Allotment.Pane>
              <Allotment>
                <Allotment.Pane>
                  <SlotPane slotId={1}><TerminalSlot /></SlotPane>
                </Allotment.Pane>
                <Allotment.Pane>
                  <SlotPane slotId={2}><TerminalSlot /></SlotPane>
                </Allotment.Pane>
              </Allotment>
            </Allotment.Pane>
            <Allotment.Pane preferredSize={300} minSize={300} maxSize={300} visible={diffOpen}>
              <DiffPanel />
            </Allotment.Pane>
            <Allotment.Pane preferredSize={24} minSize={24} maxSize={24}>
              <StatusBar diffOpen={diffOpen} onDiffToggle={() => setDiffOpen(v => !v)} />
            </Allotment.Pane>
          </Allotment>
        </Allotment.Pane>
      </Allotment>
    </div>
  )
}
