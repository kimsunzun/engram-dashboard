import { useState } from 'react'

import AgentList from '../agent/AgentList'
import { agentClient } from '../../api/clientFactory'
import { refreshProfiles } from '../../store/eventBus'

interface SidebarProps {
  onToggle: () => void
}

const BTN: React.CSSProperties = {
  background: 'none',
  border: 'none',
  color: 'var(--text-muted)',
  cursor: 'pointer',
  fontSize: '11px',
  padding: '0 2px',
}

const INPUT: React.CSSProperties = {
  background: 'var(--bg)',
  border: '1px solid var(--border)',
  color: 'var(--text)',
  fontFamily: 'var(--font-ui)',
  fontSize: '11px',
  padding: '2px 4px',
  width: '100%',
}

export default function Sidebar({ onToggle }: SidebarProps) {
  const handleDetach = () => {
    window.open('index.html#/tree', '_blank')
    onToggle()
  }

  // 깡통(예약) 생성 인라인 폼(ADR-0018). Tauri WebView 에서 window.prompt 는 신뢰 못 하니 인라인 input.
  const [creating, setCreating] = useState(false)
  const [name, setName] = useState('')
  const [cwd, setCwd] = useState('')
  const [busy, setBusy] = useState(false)
  const [json, setJson] = useState(false)
  // 생성 실패 인라인 메시지 — 토스트/StatusBar 시스템이 없어 폼 안에 표시(MAJOR-3).
  const [error, setError] = useState<string | null>(null)

  const reset = () => {
    setCreating(false)
    setName('')
    setCwd('')
    setBusy(false)
    setJson(false)
    setError(null)
  }

  const submit = () => {
    const n = name.trim()
    const c = cwd.trim()
    if (!n || !c || busy) return
    setBusy(true)
    setError(null)
    // auto_restore=false: 부팅 자동 spawn 제외(ADR-0018 결정 4) — 재부팅해도 깡통으로 남는다.
    // [임시/테스트] JSON(StreamJson) 스폰 — 정식은 §5 커맨드화(백로그: M2 spawn UI json 노출)로 대체 예정.
    agentClient
      .createClaudeProfile(n, c, [], [], false, json ? 'StreamJson' : 'Terminal')
      .then(() => refreshProfiles())
      .then(reset)
      .catch(e => {
        console.error('[createClaudeProfile]', e)
        setError(`예약 실패: ${String(e)}`) // 폼 유지 + 사용자에게 실패 표시
        setBusy(false)
      })
  }

  return (
    <div style={{
      height: '100%',
      background: 'var(--bg-secondary)',
      borderRight: '1px solid var(--border)',
      display: 'flex',
      flexDirection: 'column',
    }}>
      <div style={{
        padding: '0 8px',
        height: '28px',
        borderBottom: '1px solid var(--border)',
        display: 'flex',
        justifyContent: 'space-between',
        alignItems: 'center',
        fontFamily: 'var(--font-ui)',
        fontSize: '11px',
        color: 'var(--text-muted)',
        flexShrink: 0,
        gap: '4px',
      }}>
        <span>Agent Tree</span>
        <div style={{ display: 'flex', gap: '2px' }}>
          <button onClick={() => setCreating(v => !v)} style={BTN} title="새 Claude 프로필 예약">+</button>
          <button onClick={handleDetach} style={BTN} title="트리 분리">↗</button>
          <button onClick={onToggle} style={BTN} title="접기">◀</button>
        </div>
      </div>
      {creating && (
        <div style={{
          padding: '6px 8px',
          borderBottom: '1px solid var(--border)',
          display: 'flex',
          flexDirection: 'column',
          gap: '4px',
          flexShrink: 0,
        }}>
          <input
            autoFocus
            style={INPUT}
            placeholder="이름"
            value={name}
            onChange={e => setName(e.target.value)}
            onKeyDown={e => { if (e.key === 'Enter') submit(); if (e.key === 'Escape') reset() }}
          />
          <input
            style={INPUT}
            placeholder="작업 디렉터리(cwd)"
            value={cwd}
            onChange={e => setCwd(e.target.value)}
            onKeyDown={e => { if (e.key === 'Enter') submit(); if (e.key === 'Escape') reset() }}
          />
          <label style={{ fontFamily: 'var(--font-ui)', fontSize: '11px', color: 'var(--text-muted)', display: 'flex', alignItems: 'center', gap: '4px', cursor: 'pointer' }}>
            <input type="checkbox" checked={json} disabled={busy} onChange={e => setJson(e.target.checked)} />
            JSON 모드 (StreamJson)
          </label>
          <div style={{ display: 'flex', gap: '4px', justifyContent: 'flex-end' }}>
            <button style={BTN} onClick={reset} disabled={busy}>취소</button>
            <button
              style={{ ...BTN, color: name.trim() && cwd.trim() && !busy ? 'var(--accent)' : 'var(--text-muted)' }}
              onClick={submit}
              disabled={!name.trim() || !cwd.trim() || busy}
            >예약</button>
          </div>
          {error && (
            <div style={{ color: '#ff4444', fontFamily: 'var(--font-ui)', fontSize: '10px', wordBreak: 'break-word' }}>
              {error}
            </div>
          )}
        </div>
      )}
      <AgentList />
    </div>
  )
}
