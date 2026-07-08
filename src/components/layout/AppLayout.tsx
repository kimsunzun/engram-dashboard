import { useState } from 'react'
import { Allotment } from 'allotment'
import 'allotment/dist/style.css'
import Sidebar from './Sidebar'
import ViewLayoutRenderer from './ViewLayoutRenderer'
import DiffPanel from '../diff/DiffPanel'
import StatusBar from './StatusBar'
import { selectActiveView, useViewStore } from '../../store/viewStore'

export default function AppLayout() {
  const [sidebarOpen, setSidebarOpen] = useState(true)
  const [diffOpen, setDiffOpen] = useState(false)
  // 메인 캔버스는 ★항상 백엔드 권위 레이아웃(ADR-0035)★ — active view 의 캐시 항목만 그린다(active-only).
  // 부팅 직후 init(list_views/get_view) 전엔 active 캐시가 비어 null → 빈 화면이고, init 이 기본 View 1
  // (빈 슬롯 1개)을 캐시에 넣으면 곧장 그려진다.
  //
  // ★단일 레이아웃 권위(Brick 1)★: 옛 프론트 전용 slotStore/LayoutRenderer 는 제거됐다. 메인 캔버스·
  //   슬롯 우클릭 메뉴(SlotContextMenu)·트리 배정 모두 viewStore(=백엔드 ViewManager 미러)로 단일화됐다.
  const activeView = useViewStore(selectActiveView)

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
              {activeView && (
                <ViewLayoutRenderer
                  node={activeView.layout}
                  focusedSlotId={activeView.focusedSlotId}
                />
              )}
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
