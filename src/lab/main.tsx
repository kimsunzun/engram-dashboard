// 출력 실험실(lab) 진입점 — 터미널 모드(xterm) / JSON 모드(RichSlot) 토글 + 폭 토글.
// Tauri/데몬 없이 순수 브라우저. 컬러·레이아웃·resize 전파를 격리 실험한다(영구 활용).
//
// 폭 토글(100/50/30%)은 좌우 split 을 시뮬레이션 — 폭을 줄이면 TerminalView 의
// ResizeObserver 가 발화해 fit→onResize 가 도는지(= PTY 전파, gotty 패턴) 눈으로 확인된다.

import { createRoot } from 'react-dom/client'
import { useState } from 'react'
import { RichSlot } from './richslot/RichSlot'
import { parseStreamJson } from './richslot/parse'
import { TerminalView } from './terminal/TerminalView'
import { ansiWelcomeSample } from './terminal/fixtures'
import textFixture from './richslot/fixtures/text.jsonl?raw'
import toolFixture from './richslot/fixtures/tool.jsonl?raw'
import partialFixture from './richslot/fixtures/partial.jsonl?raw'

const JSON_FIXTURES: Record<string, string> = {
  text: textFixture,
  tool: toolFixture,
  partial: partialFixture,
}
const WIDTHS = ['100%', '50%', '30%']

function Lab() {
  const [mode, setMode] = useState<'terminal' | 'json'>('terminal') // 터미널 모드 먼저
  const [width, setWidth] = useState('100%')
  const [jsonFix, setJsonFix] = useState('tool')
  const [lastResize, setLastResize] = useState('—')

  const btn = (active: boolean) => ({ fontWeight: active ? 700 : 400 })

  return (
    <div style={{ height: '100vh', display: 'flex', flexDirection: 'column' }}>
      <div
        style={{
          padding: 8,
          background: '#111',
          color: '#ccc',
          display: 'flex',
          gap: 16,
          alignItems: 'center',
          flexWrap: 'wrap',
          fontFamily: 'system-ui, sans-serif',
          fontSize: 13,
        }}
      >
        <span style={{ display: 'flex', gap: 4 }}>
          {(['terminal', 'json'] as const).map((m) => (
            <button key={m} onClick={() => setMode(m)} style={btn(mode === m)}>
              {m === 'terminal' ? 'Terminal' : 'JSON'}
            </button>
          ))}
        </span>
        <span style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
          <span style={{ color: '#888' }}>width:</span>
          {WIDTHS.map((w) => (
            <button key={w} onClick={() => setWidth(w)} style={btn(width === w)}>
              {w}
            </button>
          ))}
        </span>
        {mode === 'json' && (
          <span style={{ display: 'flex', gap: 4 }}>
            {Object.keys(JSON_FIXTURES).map((k) => (
              <button key={k} onClick={() => setJsonFix(k)} style={btn(jsonFix === k)}>
                {k}
              </button>
            ))}
          </span>
        )}
        {mode === 'terminal' && (
          <span style={{ color: '#888', marginLeft: 'auto' }}>
            onResize → PTY: <b style={{ color: '#4a9eff' }}>{lastResize}</b>
          </span>
        )}
      </div>
      <div style={{ flex: 1, overflow: 'hidden' }}>
        <div style={{ width, height: '100%', borderRight: '2px dashed #444', overflow: 'hidden' }}>
          {mode === 'terminal' ? (
            <TerminalView
              output={ansiWelcomeSample}
              onResize={(c, r) => setLastResize(`${c}×${r}`)}
            />
          ) : (
            <RichSlot messages={parseStreamJson(JSON_FIXTURES[jsonFix])} />
          )}
        </div>
      </div>
    </div>
  )
}

createRoot(document.getElementById('root')!).render(<Lab />)
