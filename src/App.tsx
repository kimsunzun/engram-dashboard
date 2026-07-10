import { useEffect } from 'react'
import { HashRouter, Routes, Route } from 'react-router-dom'
import { themeManager } from './theme/ThemeManager'
import AppLayout from './components/layout/AppLayout'
import TreePage from './pages/TreePage'
import PopoutPage from './pages/PopoutPage'
import { initEventBus, refreshProfiles, refreshPresets } from './store/eventBus'
import { agentClient, bootstrapDaemonIfNeeded } from './api/clientFactory'
import { useAgentStore } from './store/agentStore'
// ADR-0055/0064: command + 슬롯 메뉴 기여 등록 매니페스트 단일 import — 부팅 시 모든 register(...)·
//   registerSlotMenu(...) 가 실행돼 레지스트리·슬롯 메뉴 기여부가 채워진다(산발 import 일원화, ADR-0064 §4).
import './commands/contributions'
import { installKeybindings } from './commands/keybindings'

function App() {
  useEffect(() => {
    themeManager.apply('dark')
  }, [])

  // ADR-0055: 전역 키바인딩 설치 — 언마운트/HMR 시 disposer 로 리스너 제거(중복 누적 방지).
  useEffect(() => installKeybindings(), [])

  useEffect(() => {
    // ADR-0021 §1: 부팅 시 명시 ensure 1회 — daemon 모드면 데몬을 띄운다(명령의 부수효과가 아니라
    // 명시 시작). 명령 경로(ensureReady)는 attach-only 라 이게 없으면 부팅 때 데몬이 안 뜬다.
    // start 가 캐시(host:port)를 채운 뒤에야 이후 getAgents/구독의 attach 가 붙으므로 먼저 await.
    void (async () => {
      await bootstrapDaemonIfNeeded()
      void initEventBus()
      agentClient
        .getAgents()
        .then(agents => useAgentStore.getState().setAgents(agents))
        .catch(err => console.warn('[App] getAgents failed:', err))
      // 깡통(예약) 프로필 초기 로드(ADR-0018) — 트리가 예약 노드를 그리려면 필요.
      void refreshProfiles()
      // 프리셋 초기 로드(ADR-0061) — PresetPalette 가 목록을 그리려면 필요(refreshProfiles 미러).
      void refreshPresets()
    })()
  }, [])

  return (
    <HashRouter>
      <div style={{ height: '100vh' }}>
        <Routes>
          <Route path="/" element={<AppLayout />} />
          <Route path="/tree" element={<TreePage />} />
          {/* 런타임 창(팝업 분리·빈 창 생성) — ?window=<label> 의 탭 가진 창(WindowLayout, ADR-0057). */}
          <Route path="/popup" element={<PopoutPage />} />
        </Routes>
      </div>
    </HashRouter>
  )
}

export default App
