import { useEffect } from 'react'
import { HashRouter, Routes, Route } from 'react-router-dom'
import { themeManager } from './theme/ThemeManager'
import AppLayout from './components/layout/AppLayout'
import PopupPage from './pages/PopupPage'
import TreePage from './pages/TreePage'

function App() {
  useEffect(() => {
    themeManager.apply('dark')
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
