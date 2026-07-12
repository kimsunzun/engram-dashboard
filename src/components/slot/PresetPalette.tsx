//! PresetPalette — 프리셋(자주 쓰는 cwd 북마크) 슬롯 콘텐츠(ADR-0060 variant / ADR-0061 저장·리치화).
//!
//! ★역할★: 데몬 소유 프리셋 목록(agentStore.presets)을 그리고 행별 우클릭 메뉴(이름변경·삭제)로 CRUD 한다.
//! CRUD 는 agentClient(단일 제어 표면, ADR-0011) 프리셋 메서드만 부르고, 화면 반영은 낙관 갱신 없이
//! PresetListUpdated broadcast → store 미러 교체로만 이뤄진다(멀티창 동기화 불변식, ADR-0061).
//!
//! ★pane 메뉴 없음(ADR-0064)★: 옛 pane 우클릭 "추가" 메뉴 + stopPropagation 은 제거됐다 — 추가는 이제
//! 통합 슬롯 메뉴의 preset.add command(폴더 다이얼로그 → createPreset, presetCommands.ts)로 기여된다.
//! pane 우클릭은 상위 ViewLayoutRenderer 의 통합 SlotContextMenu 로 버블한다(공통 슬롯 ops 도 함께 노출).
//! ★행(ROW) 우클릭 메뉴만 소유★: 행 우클릭은 행 핸들러가 stopPropagation 으로 가로채 item-targeted 메뉴
//! (이름변경·삭제)를 띄운다(AgentList 행 메뉴와 동형 — MENU_STYLE/outside-click/target-gone 클론).
//!
//! ★표시명 = name override ?? cwd basename(ADR-0061 리치화)★: 프리셋은 {id,cwd,name?}를 저장한다. name
//! override 가 있으면 그대로, 없으면 cwd 의 마지막 경로 세그먼트를 파생한다(공용 basename 유틸 — AgentList
//! 와 단일 출처). 이름변경 = RenamePreset command(백엔드 persist → broadcast, 낙관 갱신 X).
//!
//! ★스타일 = 변수-only(테마 준수)★: 색·폰트는 전부 CSS 변수 참조 — 하드코딩 색 리터럴 0(e-ink 대비).

import { useEffect, useRef, useState } from 'react'

import { ScrollArea } from '../ui/scroll-area'
import { agentClient } from '../../api/clientFactory'
import { useAgentStore } from '../../store/agentStore'
import { refreshPresets } from '../../store/eventBus'
import { basename } from '../../util/basename'
import type { Preset } from '../../api/types'
import { t } from '../../i18n'

/**
 * 프리셋 표시명 = name override ?? cwd basename(ADR-0061 리치화). name override 가 있으면 그대로 쓰고,
 * 없으면(null) 공용 `basename` 유틸로 cwd 마지막 세그먼트를 파생한다(AgentList 행 표시명과 단일 출처
 * 공유 — 복제 시 win/posix·root 엣지가 갈린다). basename 파생 규칙은 위임한다.
 */
export function presetDisplayName(preset: Pick<Preset, 'cwd' | 'name'>): string {
  return preset.name ?? basename(preset.cwd)
}

/** 행 우클릭 메뉴 — primitive snapshot(좌표 + 대상 preset id). AgentList RowMenu 와 동형. */
type RowMenu = { x: number; y: number; presetId: string }

export default function PresetPalette() {
  const presets = useAgentStore(s => s.presets)
  const rowMenuRef = useRef<HTMLDivElement>(null)
  const [rowMenu, setRowMenu] = useState<RowMenu | null>(null)

  // in-flight 가드(로컬 컴포넌트 상태 only — store 낙관 갱신 금지, ADR-0061). Ack 전 중복 제출이
  //   duplicate DeletePreset 을 쏘는 것을 막는다. 성공·실패 무관 완료 시 해제.
  //
  // ★ref = 권위적 double-fire 가드, state = 시각(disabled/opacity)★: useState 가드는 re-render commit
  //   전 두 번째 호출이 stale closure 로 아직 false 를 읽어 둘 다 통과하는 창이 있다. ref 는 동기 mutable 이라
  //   같은 tick 내 두 번째 호출도 즉시 true 를 읽는다 — ref 가 실제 중복 발화 차단을 담당하고 state 는 시각용.
  const deletingRef = useRef<Set<string>>(new Set())
  const [deleting, setDeleting] = useState<ReadonlySet<string>>(() => new Set())
  // rename in-flight 가드(delete 가드와 동형 — RenamePreset 중복 제출 차단).
  const renamingRef = useRef<Set<string>>(new Set())
  const [renaming, setRenaming] = useState<ReadonlySet<string>>(() => new Set())

  // ★인라인 편집 로컬 상태(프론트 전용 — 백엔드 권위 이름과 별개의 임시 draft, TabBar 패턴)★:
  //   editingId=편집 중 preset id(없으면 null), draft=입력 중 문자열. 확정(Enter/blur) 시에만 renamePreset.
  const [editingId, setEditingId] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  // ★안정 ref★: 편집 진입(editingId 변화) 시점에만 정확히 1회 select() — 인라인 콜백 ref 로 select 하면
  //   매 렌더 재부착돼 타이핑이 깨진다(TabBar FIX 1 동형).
  const inputRef = useRef<HTMLInputElement>(null)
  useEffect(() => {
    if (editingId !== null) inputRef.current?.select()
  }, [editingId])

  // 행 메뉴 바깥 클릭으로 닫기(자기 ref 밖 mousedown 이면 닫는다 — AgentList 동형).
  useEffect(() => {
    if (!rowMenu) return
    const h = (e: MouseEvent) => {
      const target = e.target as Node
      if (rowMenuRef.current && !rowMenuRef.current.contains(target)) setRowMenu(null)
    }
    document.addEventListener('mousedown', h)
    return () => document.removeEventListener('mousedown', h)
  }, [rowMenu])

  // Escape 로 열린 행 메뉴 닫기(열려 있을 때만 리스너 — 누수 방지).
  useEffect(() => {
    if (!rowMenu) return
    const h = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setRowMenu(null)
    }
    document.addEventListener('keydown', h)
    return () => document.removeEventListener('keydown', h)
  }, [rowMenu])

  // ★타깃 사라지면 메뉴 닫기★: rowMenu 는 대상 행보다 오래 산다 — 목록이 바뀌어(삭제 등) 대상 preset id 가
  //   presets 에서 빠져도 rowMenu 는 남는다. 대상이 목록에서 사라지면 즉시 null 로 리셋(AgentList 동형).
  useEffect(() => {
    if (rowMenu && !presets.some(p => p.id === rowMenu.presetId)) setRowMenu(null)
  }, [rowMenu, presets])

  // 삭제: id 별 in-flight 추적 → 같은 프리셋의 삭제는 resolve 전까지 1회만 발화(더블클릭 가드).
  //   deletingRef(동기)가 권위 가드 — 다른 id 는 여전히 동시 삭제, 같은 id 는 1회만.
  const removePreset = (id: string): void => {
    if (deletingRef.current.has(id)) return // 동기 권위 가드(같은 tick 두 번째 호출 즉시 차단)
    deletingRef.current.add(id) // async 진입 전 동기 lock
    setDeleting(prev => {
      const next = new Set(prev)
      next.add(id)
      return next
    })
    agentClient
      .deletePreset(id)
      // ★성공 시 refreshPresets() 안전망(AgentList delete/rename 과 대칭)★: 목록 반영은 원칙적으로
      //   PresetListUpdated broadcast 로 오나, 그걸 놓쳤을 때(재연결 창·이벤트 유실) 이 창만 stale 로 남는다.
      //   권위 목록을 다시 끌어와 전체 교체(broadcast 정상 도달 시 같은 목록 재적용 — 무해·멱등).
      .then(() => refreshPresets())
      .catch(e => console.error('[PresetPalette] deletePreset 실패:', e))
      .finally(() => {
        deletingRef.current.delete(id) // 성공·실패 무관 lock 해제(에러가 UI 영구 잠금 방지)
        setDeleting(prev => {
          const next = new Set(prev)
          next.delete(id)
          return next
        })
      })
  }

  // 편집 진입: 현재 표시명을 draft 로 시드(우클릭 "이름 변경").
  const beginEdit = (preset: Preset) => {
    setEditingId(preset.id)
    setDraft(presetDisplayName(preset))
  }
  const cancelEdit = () => setEditingId(null)
  // 확정: trim 후 비었거나 현재 표시명과 같으면 no-op(revert), 아니면 renamePreset. 어느 경우든 편집 종료.
  // ★멱등★: editingId 가 이 preset 이 아니면 즉시 return — Enter 가 setEditingId(null) 로 input 을 언마운트하면
  //   브라우저가 blur 를 쏴 onBlur→commitEdit 이 한 번 더 돈다. Enter 후 editingId 는 이미 null 이라 blur 의
  //   commitEdit 은 no-op → renamePreset 이중 호출을 막는다(TabBar 멱등 동형).
  const commitEdit = (preset: Preset) => {
    if (editingId !== preset.id) return
    const trimmed = draft.trim()
    setEditingId(null)
    // 미변경(현재 표시명과 동일)·빈 문자열이면 발화 안 함 — 백엔드에 불필요한 RenamePreset 을 안 보낸다.
    if (trimmed.length === 0 || trimmed === presetDisplayName(preset)) return
    if (renamingRef.current.has(preset.id)) return // 동기 권위 가드(rename 중복 제출 차단)
    renamingRef.current.add(preset.id)
    setRenaming(prev => {
      const next = new Set(prev)
      next.add(preset.id)
      return next
    })
    agentClient
      .renamePreset(preset.id, trimmed)
      // ★성공 시 refreshPresets() 안전망(deletePreset 과 대칭)★: 표시명 반영은 PresetListUpdated broadcast
      //   로 오나, 놓쳤을 때 이 창만 stale 로 남는다 — 권위 목록 재적용으로 대칭·멱등 보장(위 delete 주석 참조).
      .then(() => refreshPresets())
      .catch(e => console.error('[PresetPalette] renamePreset 실패:', e))
      .finally(() => {
        renamingRef.current.delete(preset.id)
        setRenaming(prev => {
          const next = new Set(prev)
          next.delete(preset.id)
          return next
        })
      })
  }

  return (
    <div
      data-preset-palette="1"
      style={{
        width: '100%',
        height: '100%',
        boxSizing: 'border-box',
        display: 'flex',
        flexDirection: 'column',
        background: 'var(--bg)',
        color: 'var(--text)',
        fontFamily: 'var(--font-ui)',
        fontSize: '13px',
        overflow: 'hidden',
      }}
    >
      {/* 슬롯 콘텐츠 라벨(사용자 요청) — 이 슬롯 = 프리셋 팔레트임을 표시. 공용 슬롯 헤더가 아니라
          PresetPalette·AgentList 이 2개 variant 컴포넌트에만 각자 넣는다(터미널 등 다른 슬롯 무영향). 변수-only. */}
      <div
        data-slot-label="preset"
        style={{
          padding: '6px 8px',
          borderBottom: '1px solid var(--border)',
          color: 'var(--text-muted)',
          fontFamily: 'var(--font-ui)',
          fontSize: '11px',
          fontWeight: 600,
          letterSpacing: '0.03em',
          flexShrink: 0,
        }}
      >
        {t('preset.label')}
      </div>

      {/* 프리셋 목록 — 각 행: 표시명(name override ?? basename) + 전체 cwd(muted). 행 우클릭 = 메뉴(이름변경·삭제).
          공용 ScrollArea seam(ADR-0053)으로 스크롤. 평면 목록(가상화 없음)이라 Viewport 로 감싸도 무해. */}
      <ScrollArea style={{ flex: 1, minHeight: 0 }}>
        {presets.length === 0 ? (
          <div style={{ padding: '12px', color: 'var(--text-muted)', fontSize: '12px' }}>
            {t('preset.empty')}
          </div>
        ) : (
          presets.map(preset => {
            const isBusy = deleting.has(preset.id) || renaming.has(preset.id)
            const isEditing = editingId === preset.id
            return (
            <div
              key={preset.id}
              data-preset-id={preset.id}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: '8px',
                padding: '6px 8px',
                borderBottom: '1px solid var(--border)',
                cursor: isBusy ? 'wait' : 'default',
                opacity: isBusy ? 0.6 : 1, // in-flight 시각 표시(색 리터럴 없이 opacity 만)
                userSelect: 'none',
              }}
              // ★행 우클릭 = 행 메뉴(이름변경·삭제)★: 상위 통합 슬롯 메뉴로 새지 않게 stopPropagation(ADR-0064).
              onContextMenu={e => {
                e.preventDefault()
                e.stopPropagation()
                setRowMenu({ x: e.clientX, y: e.clientY, presetId: preset.id })
              }}
            >
              <div style={{ flex: 1, minWidth: 0 }}>
                {/* 표시명 = name override ?? cwd basename(ADR-0061 리치화). 편집 중이면 인라인 input. */}
                {isEditing ? (
                  <input
                    data-preset-rename-input={preset.id}
                    value={draft}
                    autoFocus
                    ref={inputRef}
                    onChange={e => setDraft(e.target.value)}
                    onKeyDown={e => {
                      // ★버블 차단★: 편집 중 키가 전역 키바인딩으로 새면 안 된다(TabBar 동형).
                      e.stopPropagation()
                      if (e.key === 'Enter') commitEdit(preset)
                      else if (e.key === 'Escape') cancelEdit() // 취소(revert — renamePreset 안 부름).
                    }}
                    onBlur={() => commitEdit(preset)}
                    onClick={e => e.stopPropagation()}
                    style={{
                      // 내용 폭에 맞춤(field-sizing:content) — TabBar rename input 동형. minWidth/maxWidth 로 상·하한.
                      fieldSizing: 'content',
                      minWidth: '3ch',
                      maxWidth: '180px',
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
                  <div
                    data-preset-name="1"
                    style={{ color: 'var(--text)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}
                  >
                    {presetDisplayName(preset)}
                  </div>
                )}
                <div
                  title={preset.cwd}
                  style={{
                    color: 'var(--text-muted)',
                    fontSize: '11px',
                    whiteSpace: 'nowrap',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                  }}
                >
                  {preset.cwd}
                </div>
              </div>
            </div>
            )
          })
        )}

        {/* ── 행 우클릭 메뉴(이름변경 · 삭제) ─────────────────────────────────── */}
        {rowMenu && (
          <div ref={rowMenuRef} style={MENU_STYLE(rowMenu.x, rowMenu.y)}>
            {rowMenuItems(rowMenu, presets, { beginEdit, removePreset, deleting, renaming }).map(item => (
              <div
                key={item.label}
                style={MENU_ITEM_STYLE(item.disabled)}
                onMouseEnter={e => {
                  if (!item.disabled) e.currentTarget.style.background = 'color-mix(in srgb, var(--accent) 20%, transparent)'
                }}
                onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
                onClick={e => {
                  e.stopPropagation()
                  if (!item.disabled) {
                    item.action()
                    setRowMenu(null)
                  }
                }}
              >
                {item.label}
              </div>
            ))}
          </div>
        )}
      </ScrollArea>
    </div>
  )
}

/** 행 메뉴 항목 산출 — 대상 preset 을 찾아 이름변경/삭제 액션을 만든다. in-flight 면 disabled(시각). */
function rowMenuItems(
  rowMenu: RowMenu,
  presets: Preset[],
  ctx: {
    beginEdit: (p: Preset) => void
    removePreset: (id: string) => void
    deleting: ReadonlySet<string>
    renaming: ReadonlySet<string>
  },
): Array<{ label: string; disabled: boolean; action: () => void }> {
  const preset = presets.find(p => p.id === rowMenu.presetId)
  if (!preset) return []
  const busy = ctx.deleting.has(preset.id) || ctx.renaming.has(preset.id)
  return [
    { label: t('preset.rename'), disabled: busy, action: () => ctx.beginEdit(preset) },
    // 메뉴 라벨은 짧은 '삭제'(preset.deleteBtn) — command 제목 '프리셋 삭제'(preset.delete)와 구분.
    { label: t('preset.deleteBtn'), disabled: busy, action: () => ctx.removePreset(preset.id) },
  ]
}

// 메뉴 공통 스타일(AgentList 인라인 메뉴와 동형 — 변수-only).
function MENU_STYLE(x: number, y: number): React.CSSProperties {
  return {
    position: 'fixed',
    top: y,
    left: x,
    background: 'var(--bg-secondary)',
    border: '1px solid var(--border)',
    borderRadius: '4px',
    zIndex: 1000,
    minWidth: '150px',
    boxShadow: '0 2px 8px rgba(0,0,0,0.3)',
    fontFamily: 'var(--font-ui)',
    fontSize: '12px',
  }
}
function MENU_ITEM_STYLE(disabled: boolean): React.CSSProperties {
  return {
    padding: '6px 12px',
    cursor: disabled ? 'default' : 'pointer',
    color: disabled ? 'var(--text-muted)' : 'var(--text)',
    opacity: disabled ? 0.5 : 1,
  }
}
