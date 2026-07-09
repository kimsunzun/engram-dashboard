import { useState } from 'react'
import { Allotment } from 'allotment'
import 'allotment/dist/style.css'
import Sidebar from './Sidebar'
import WindowLayout from './WindowLayout'
import DiffPanel from '../diff/DiffPanel'
import StatusBar from './StatusBar'
import { MAIN_WINDOW_LABEL } from '../../store/viewStore'

export default function AppLayout() {
  const [sidebarOpen, setSidebarOpen] = useState(true)
  const [diffOpen, setDiffOpen] = useState(false)
  // ★탭 소유 모델(ADR-0057)★: main 창의 슬롯 영역은 WindowLayout("main") 이 소유한다 — 탭바 + 활성 탭
  //   슬롯 캔버스(keep-alive). 창 크롬(Sidebar/DiffPanel/StatusBar)만 AppLayout 이 감싸고, 슬롯 영역만
  //   교체한다(§7-1). main·팝업이 같은 WindowLayout 을 마운트해 D-2 "동일 코드경로"를 만든다.
  //
  // ★단일 레이아웃 권위(Brick 1 / ADR-0035·0057)★: 옛 프론트 전용 slotStore/LayoutRenderer 는 제거됐다.
  //   메인 캔버스·슬롯 우클릭 메뉴(SlotContextMenu)·트리 배정 모두 viewStore(=백엔드 ViewManager 미러)로
  //   단일화됐다. 전역 activeViewId 는 창별 windows[label].active 로 대체됐다(ADR-0057).

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
              <WindowLayout label={MAIN_WINDOW_LABEL} />
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
