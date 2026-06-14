import { useEffect } from 'react'
import { HashRouter, Routes, Route } from 'react-router-dom'
import { themeManager } from './theme/ThemeManager'
import AppLayout from './components/layout/AppLayout'
import PopupPage from './pages/PopupPage'
import TreePage from './pages/TreePage'
import { initEventBus } from './store/eventBus'
import { agentClient } from './api/clientFactory'
import { useAgentStore } from './store/agentStore'

function App() {
  useEffect(() => {
    themeManager.apply('dark')
  }, [])

  useEffect(() => {
    // Tauri 이벤트 버스 초기화 + 에이전트 초기 목록 로드
    void initEventBus()
    agentClient
      .getAgents()
      .then(agents => useAgentStore.getState().setAgents(agents))
      .catch(err => console.warn('[App] getAgents failed:', err))
  }, [])

  return (
    <HashRouter>
      <div style={{ height: '100vh' }}>
        <Routes>
          <Route path="/" element={<AppLayout />} />
          <Route path="/popup" element={<PopupPage />} />
          <Route path="/tree" element={<TreePage />} />
        </Routes>
      </div>
    </HashRouter>
  )
}

export default App
