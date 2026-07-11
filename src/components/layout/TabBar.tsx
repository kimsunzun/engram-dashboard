// TabBar — 한 창(label)의 탭 목록 + 활성 탭 표시 + [+] 새 탭 + 탭별 닫기 + 이름 인라인 편집(ADR-0057, §7-2).
//
// ★§5 손발/두뇌 분리★: 사람 클릭(탭 전환/추가/닫기/이름변경)은 viewStore 액션(switchTab/createTab/closeTab/
// renameTab)만 부른다 — LLM(window.__engramLayout·__engramCmd)이 같은 command 를 흔드는 것과 물리적으로
// 동일 표면. 실제 상태 변경은 백엔드 ViewManager(권위)가 하고 window:tabs-updated emit 으로 반영된다(낙관 갱신 X).
// 이름 편집도 확정 시 onRename(→renameTab) 만 부르고, 화면 이름은 emit 으로만 바뀐다(로컬 draft 는 편집 중 임시값).
//
// 스타일: shadcn 탭 풍(창 상단 가로 바). 순수 내부라 위치/스타일은 메인 재량 — CSS 변수 테마 토큰 사용.

import { useEffect, useRef, useState } from 'react'

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
  /**
   * 탭 이름 확정 콜백(더블클릭 → 인라인 편집 → Enter/blur 확정). 이름 정규화(trim·공백거부·미변경 스킵)는
   * TabBar 가 확정 직전에 처리하므로 여기엔 이미 trim 된 non-empty·변경된 이름만 온다. renameTab 과 동일 표면(§5).
   */
  onRename: (viewId: string, name: string) => void
}

export default function TabBar({
  label,
  tabs,
  active,
  onSwitch,
  onCreate,
  onClose,
  onRename,
}: TabBarProps) {
  // ★인라인 편집 로컬 상태(프론트 전용 — 백엔드 권위 이름과 별개의 임시 draft)★: editingId=편집 중인 탭 id
  //   (없으면 null), draft=입력 중 문자열. 확정(Enter/blur) 시에만 onRename 을 호출한다.
  const [editingId, setEditingId] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  // ★안정 ref★: input 에 매 렌더 새 콜백 ref 를 걸면 React 가 매번 재부착 → select() 가 매 키입력마다
  //   실행돼 방금 친 글자를 다시 전체선택 → 다음 글자가 통째로 덮어써진다("New"→"w"). 안정 ref + effect 로
  //   편집 진입(editingId 변화) 시점에만 정확히 1회 select() 하게 고정한다.
  const inputRef = useRef<HTMLInputElement>(null)
  useEffect(() => {
    if (editingId !== null) inputRef.current?.select()
  }, [editingId])

  // 편집 진입: 현재 이름을 draft 로 시드. (더블클릭 대상 탭.)
  const beginEdit = (tab: ViewMeta) => {
    setEditingId(tab.id)
    setDraft(tab.name)
  }
  const cancelEdit = () => setEditingId(null)
  // 확정: trim 후 비었거나 원래 이름과 같으면 no-op(revert), 아니면 onRename. 어느 경우든 편집 종료.
  // ★멱등★: editingId 가 이 탭이 아니면 즉시 return — Enter 가 setEditingId(null) 로 input 을 언마운트하면
  //   브라우저가 blur 를 쏴 onBlur→commitEdit 이 한 번 더 돈다. Enter 후 editingId 는 이미 null 이므로
  //   blur 의 commitEdit 은 여기서 no-op 이 돼 onRename 이중 호출(중복 rename_tab invoke)을 막는다.
  //   (두 이벤트가 서로 다른 렌더 사이클·다른 클로저라 editingId 최신값을 본다.)
  const commitEdit = (tab: ViewMeta) => {
    if (editingId !== tab.id) return
    const trimmed = draft.trim()
    if (trimmed.length > 0 && trimmed !== tab.name) onRename(tab.id, trimmed)
    setEditingId(null)
  }

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
            {editingId === tab.id ? (
              <input
                data-testid="tab-rename-input"
                data-view-id={tab.id}
                value={draft}
                autoFocus
                // 전체 선택(빠른 덮어쓰기 UX)은 편집 진입 시 1회만 — 위 useEffect([editingId]) 담당.
                // 여기 인라인 콜백 ref 로 select() 하면 매 렌더 재실행돼 타이핑이 깨진다(FIX 1).
                ref={inputRef}
                onChange={e => setDraft(e.target.value)}
                onKeyDown={e => {
                  // ★버블 차단★: 편집 중 키가 부모 탭(onClick=switch)·전역 키바인딩으로 새면 안 된다.
                  e.stopPropagation()
                  if (e.key === 'Enter') commitEdit(tab)
                  else if (e.key === 'Escape') cancelEdit() // 취소(revert — onRename 안 부름).
                }}
                // blur 확정(다른 곳 클릭). trim 비었거나 미변경이면 commitEdit 내부에서 revert.
                onBlur={() => commitEdit(tab)}
                // 입력 클릭/더블클릭이 부모 탭 onClick(switch)·onDoubleClick(편집 진입)으로 버블 금지.
                onClick={e => e.stopPropagation()}
                onDoubleClick={e => e.stopPropagation()}
                style={{
                  // ★내용 폭에 맞춤★: field-sizing:content 로 input 이 draft 텍스트 폭만큼만 차지한다
                  //   (기본 input 은 size 속성 폭 ~160px 고정이라 짧은 이름에도 넓게 벌어짐). 브라우저가
                  //   실제 글자 폭을 측정하므로 CJK·비례폰트도 정확. WebView2(Chromium 123+) 지원.
                  //   minWidth 로 빈/한글자 때 너무 좁아지는 것만 방어, maxWidth 는 상한.
                  fieldSizing: 'content',
                  minWidth: '3ch',
                  maxWidth: '160px',
                  font: 'inherit',
                  color: 'var(--text)',
                  background: 'var(--bg)',
                  border: '1px solid var(--accent)',
                  borderRadius: '2px',
                  padding: '0 2px',
                  outline: 'none',
                }}
              />
            ) : (
              <span
                data-testid="tab-name"
                // ★단일 클릭 = 탭 전환(부모 onClick 로 버블), 더블클릭 = 인라인 편집 진입(§5 사람 UI 경로).★
                //   더블클릭 제스처는 dblclick 전에 click 을 2번 쏜다 — 그 클릭들이 부모 onClick(switch)로
                //   새면 편집 진입 전에 탭이 엉뚱하게 전환된다. e.detail>=2(더블클릭을 완성하는 두 번째 클릭)만
                //   stopPropagation 으로 삼켜 재전환을 막고, 진짜 단일 클릭(e.detail===1)은 그대로 전환시킨다.
                onClick={e => {
                  if (e.detail >= 2) e.stopPropagation()
                }}
                onDoubleClick={e => {
                  e.stopPropagation()
                  beginEdit(tab)
                }}
                style={{ overflow: 'hidden', textOverflow: 'ellipsis', maxWidth: '160px' }}
              >
                {tab.name}
              </span>
            )}
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
