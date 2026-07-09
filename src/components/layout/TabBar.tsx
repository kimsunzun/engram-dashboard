// TabBar — 한 창(label)의 탭 목록 + 활성 탭 표시 + [+] 새 탭 + 탭별 닫기(ADR-0057, §7-2).
//
// ★§5 손발/두뇌 분리★: 사람 클릭(탭 전환/추가/닫기)은 viewStore 액션(switchTab/createTab/closeTab)만
// 부른다 — LLM(window.__engramLayout·__engramCmd)이 같은 command 를 흔드는 것과 물리적으로 동일 표면.
// 실제 상태 변경은 백엔드 ViewManager(권위)가 하고 window:tabs-updated emit 으로 반영된다(낙관 갱신 X).
//
// 스타일: shadcn 탭 풍(창 상단 가로 바). 순수 내부라 위치/스타일은 메인 재량 — CSS 변수 테마 토큰 사용.

import type { ViewMeta } from '../../api/layoutTypes'

interface TabBarProps {
  /** 이 탭바가 속한 창 label(main·slot-popup-N). 모든 액션이 이 label 을 백엔드에 넘긴다. */
  label: string
  /** 그 창의 탭 목록(좌→우 순서, window:tabs-updated 미러). */
  tabs: ViewMeta[]
  /** 그 창의 활성 탭 view id(강조 표시 대상). */
  active: string
  onSwitch: (viewId: string) => void
  onCreate: () => void
  onClose: (viewId: string) => void
}

export default function TabBar({ label, tabs, active, onSwitch, onCreate, onClose }: TabBarProps) {
  return (
    <div
      data-testid="tab-bar"
      data-window-label={label}
      style={{
        display: 'flex',
        alignItems: 'stretch',
        height: '28px',
        flexShrink: 0,
        background: 'var(--bg-secondary)',
        borderBottom: '1px solid var(--border)',
        fontFamily: 'var(--font-ui)',
        fontSize: '12px',
        userSelect: 'none',
        overflowX: 'auto',
      }}
    >
      {tabs.map(tab => {
        const isActive = tab.id === active
        return (
          <div
            key={tab.id}
            data-testid="tab"
            data-view-id={tab.id}
            data-active={isActive ? 'true' : 'false'}
            onClick={() => onSwitch(tab.id)}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: '6px',
              padding: '0 10px',
              cursor: 'pointer',
              borderRight: '1px solid var(--border)',
              // 활성 탭: accent 하단 강조 + 밝은 배경. 비활성: muted.
              background: isActive ? 'var(--bg)' : 'transparent',
              color: isActive ? 'var(--text)' : 'var(--text-muted)',
              borderBottom: isActive ? '2px solid var(--accent)' : '2px solid transparent',
              whiteSpace: 'nowrap',
            }}
          >
            <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', maxWidth: '160px' }}>
              {tab.name}
            </span>
            <button
              type="button"
              aria-label={`탭 닫기: ${tab.name}`}
              data-testid="tab-close"
              // ★탭 닫기★: 부모 onClick(전환)로 버블 금지 → stopPropagation. close_tab command 경로.
              onClick={e => {
                e.stopPropagation()
                onClose(tab.id)
              }}
              style={{
                background: 'transparent',
                border: 'none',
                color: 'var(--text-muted)',
                cursor: 'pointer',
                padding: '0 2px',
                fontSize: '12px',
                lineHeight: 1,
              }}
            >
              ×
            </button>
          </div>
        )
      })}
      <button
        type="button"
        aria-label="새 탭"
        data-testid="tab-add"
        onClick={onCreate}
        style={{
          background: 'transparent',
          border: 'none',
          color: 'var(--text-muted)',
          cursor: 'pointer',
          padding: '0 10px',
          fontSize: '14px',
        }}
      >
        +
      </button>
    </div>
  )
}
