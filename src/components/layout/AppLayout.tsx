import { useState } from 'react'
import { Allotment } from 'allotment'
import 'allotment/dist/style.css'
import { useSlotStore } from '../../store/slotStore'
import Sidebar from './Sidebar'
import LayoutRenderer from './LayoutRenderer'
import DiffPanel from '../diff/DiffPanel'
import StatusBar from './StatusBar'

export default function AppLayout() {
  const [sidebarOpen, setSidebarOpen] = useState(true)
  const [diffOpen, setDiffOpen] = useState(false)
  const layout = useSlotStore(s => s.layout)

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
              <LayoutRenderer node={layout} />
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
