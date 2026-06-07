interface StatusBarProps {
  diffOpen?: boolean
  onDiffToggle?: () => void
}

export default function StatusBar({ diffOpen, onDiffToggle }: StatusBarProps) {
  return (
    <div style={{
      height: '24px',
      background: 'var(--bg-secondary)',
      borderTop: '1px solid var(--border)',
      display: 'flex',
      alignItems: 'center',
      padding: '0 0.5rem',
      fontFamily: 'var(--font-ui)',
      fontSize: '11px',
      color: 'var(--text-muted)',
      gap: '0.5rem',
    }}>
      <span style={{ flex: 1 }}>Ready</span>
      {onDiffToggle && (
        <button
          onClick={onDiffToggle}
          style={{
            background: 'none',
            border: 'none',
            color: 'var(--text-muted)',
            cursor: 'pointer',
            fontSize: '11px',
            padding: '0 4px',
            fontFamily: 'var(--font-ui)',
          }}
        >
          Diff {diffOpen ? '▲' : '▼'}
        </button>
      )}
    </div>
  )
}
