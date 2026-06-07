import { DiffEditor, loader } from '@monaco-editor/react'
import * as monaco from 'monaco-editor'

loader.config({ monaco })

const original = `function hello() {\n  console.log("hello")\n}`
const modified = `function hello(name: string) {\n  console.log(\`hello \${name}\`)\n}`

const BTN: React.CSSProperties = {
  padding: '2px 10px',
  fontSize: '11px',
  background: 'var(--bg)',
  color: 'var(--text)',
  border: '1px solid var(--border)',
  borderRadius: '3px',
  cursor: 'pointer',
  fontFamily: 'var(--font-ui)',
}

export default function DiffPanel() {
  return (
    <div style={{ height: '300px', display: 'flex', flexDirection: 'column', borderTop: '1px solid var(--border)' }}>
      <div style={{
        display: 'flex',
        gap: '6px',
        alignItems: 'center',
        padding: '4px 8px',
        background: 'var(--bg-secondary)',
        borderBottom: '1px solid var(--border)',
        flexShrink: 0,
      }}>
        <span style={{ fontFamily: 'var(--font-ui)', fontSize: '11px', color: 'var(--text-muted)', flex: 1 }}>Diff</span>
        <button style={BTN} onClick={() => console.log('Accept')}>Accept</button>
        <button style={BTN} onClick={() => console.log('Revert')}>Revert</button>
      </div>
      <div style={{ flex: 1, minHeight: 0 }}>
        <DiffEditor
          original={original}
          modified={modified}
          language="typescript"
          theme="vs-dark"
          height="100%"
          options={{ readOnly: true, renderSideBySide: true, minimap: { enabled: false } }}
        />
      </div>
    </div>
  )
}
