//! AgentList — 실행중 에이전트 ∪ 예약(Reserved) 프로필을 그리는 FLAT 목록(ADR-0062 상태 글리프 / ADR-0018 머지).
//!
//! ★AgentTree(react-arborist) 대체★: MVP 는 계층이 없어 트리 대신 평면 목록이다. 머지 로직(running ∪
//! reserved)은 순수 함수 mergeTreeNodes 를 그대로 재사용하고(트리 렌더링만 버린다), 각 행 = [glyph][name].
//!
//! ★상태 = 색이 아니라 모양(글리프)★(ADR-0062): e-ink(흑백)에서도 상태가 구분되도록 색 리터럴 대신
//! statusGlyph 로 모양을 고른다. 5-glyph 어휘 중 백엔드가 실제 구분 가능한 3개(●/◻/✗)만 점등, ○(유휴)·
//! ◐(입력대기)는 어휘로만 정의(◐ 는 백엔드 신호 없어 절대 점등 안 함).
//!
//! ★표시명 = cwd basename(프론트 파생)★: 이름을 저장하지 않고 cwd 의 마지막 세그먼트를 쓴다(공용 basename
//! 유틸 — PresetPalette 표시명과 단일 출처). cwd 미노출(요구: name 만, cwd 표시 안 함).
//!
//! ★두 우클릭 메뉴(§5 단일 제어 표면)★: 빈 영역 = "에이전트 생성"(프리셋 픽커) · 행 = 열기/종료/이름변경/
//! 재시작. 레이아웃·에이전트 조작은 전부 agentClient / viewStore(백엔드 권위 invoke→emit) 로만 흐른다 —
//! raw invoke/ptyApi 직접 호출 없음(ADR-0011). 이름변경·재시작은 대응 백엔드 command 가 없어 "준비 중"
//! 비활성 항목(날조 금지 — 존재하는 command 만 배선).

import { useEffect, useRef, useState } from 'react'

import { agentClient } from '../../api/clientFactory'
import { useAgentStore } from '../../store/agentStore'
import { currentViewId, selectView, useViewStore } from '../../store/viewStore'
import { refreshProfiles } from '../../store/eventBus'
import { basename } from '../../util/basename'
import { run } from '../../commands/registry'
import { mergeTreeNodes, type AgentTreeNode } from './mergeTreeNodes'

/**
 * 상태 → 글리프(모양) 매핑 — PURE(외부 의존 0, ADR-0062). 색이 아닌 모양이 상태를 담아 e-ink 에서도
 * 구분된다. 5-glyph 어휘를 전부 정의하되 현 백엔드가 구분 가능한 3개만 실제 점등한다.
 *
 * 매핑(ADR-0062):
 *   - Running               → ● (작업중)
 *   - Exiting/Exited/Killed  → ◻ (멈춤 — Exiting 은 terminal 직전 전이)
 *   - Failed                → ✗ (에러)
 *   - Reserved(프론트 합성)   → ○ (유휴/미spawn 깡통)
 *   - 그 외(미지 status)      → ○ (안전 degrade — 빈 칸 방지)
 *
 * ★◐(입력대기)는 어휘로만 존재 — 절대 점등하지 않는다★: 백엔드가 "입력 대기" 신호를 내지 않으므로
 *   이 함수는 ◐ 를 반환하는 분기가 없다(ADR-0062 — 미점등은 결함이 아니라 의도). 백엔드가 신호를 낼 때
 *   이 함수에 분기를 추가하는 것이 정규 경로.
 */
export function statusGlyph(status: string): string {
  switch (status) {
    case 'Running':
      return '●' // 작업중
    case 'Exiting':
    case 'Exited':
    case 'Killed':
      return '◻' // 멈춤(종료/전이)
    case 'Failed':
      return '✗' // 에러
    case 'Reserved':
      return '○' // 유휴(미spawn 깡통)
    default:
      return '○' // 미지 status → 유휴로 degrade(빈 글리프 방지)
  }
}

/** 배경(빈 영역) 우클릭 메뉴 좌표 + 픽커 열림 상태. */
type BgMenu = { x: number; y: number }
/** 행 우클릭 메뉴 — 가상화 없는 평면 목록이지만 AgentTree 와 동형으로 primitive snapshot 만 든다. */
type RowMenu = {
  x: number
  y: number
  agentId: string
  kind: 'running' | 'reserved'
}

export default function AgentList() {
  const rowMenuRef = useRef<HTMLDivElement>(null)
  const bgMenuRef = useRef<HTMLDivElement>(null)
  const [rowMenu, setRowMenu] = useState<RowMenu | null>(null)
  const [bgMenu, setBgMenu] = useState<BgMenu | null>(null)
  // 배경 메뉴 → "에이전트 생성" 선택 시 뜨는 프리셋 픽커(경로 직접 입력 포함) 열림 좌표.
  const [picker, setPicker] = useState<{ x: number; y: number } | null>(null)
  const [pickerDraft, setPickerDraft] = useState('')
  // 픽커 자체의 spawn 실패 메시지(에이전트 id 없이 뜨는 생성 흐름 — errorById 와 분리). 다음 시도 때 지운다.
  const [pickerError, setPickerError] = useState<string | null>(null)
  const pickerRef = useRef<HTMLDivElement>(null)
  // ★ref = 권위적 double-fire 가드, state(busyIds) = 시각(disabled/opacity)★ (PresetPalette 패턴 동형):
  //   useState 가드만으로는 re-render commit 전 두 번째 호출이 stale closure 로 busyIds 를 아직 비어있게 읽어
  //   둘 다 통과하는 창이 있다(빠른 더블클릭). ref 는 동기 mutable 이라 같은 tick 두 번째 호출도 즉시 차단한다.
  //   그래서 busyRef 가 실제 중복 발화를 막고, busyIds(state)는 순수 시각 표시(disabled/opacity)로만 병행한다.
  const busyRef = useRef<Set<string>>(new Set())
  const [busyIds, setBusyIds] = useState<Set<string>>(new Set())
  // 액션 실패 메시지 — 토스트/StatusBar 가 없어 행 옆 인라인 표시(AgentTree MAJOR-3 패턴).
  const [errorById, setErrorById] = useState<Record<string, string>>({})

  const agents = useAgentStore(s => s.agents)
  const profiles = useAgentStore(s => s.profiles)
  const presets = useAgentStore(s => s.presets)
  const selectedAgentId = useAgentStore(s => s.selectedAgentId)
  const setSelectedAgent = useAgentStore(s => s.setSelectedAgent)

  // 예약(프로필) ∪ 실행중(agents) 머지 — 순수 함수 재사용(트리 렌더링만 버림).
  const rows: AgentTreeNode[] = mergeTreeNodes(profiles, agents)

  // 메뉴/픽커 바깥 클릭으로 닫기(각각 자기 ref 밖 mousedown 이면 닫는다). 항목 클릭의 mousedown 이 먼저
  //   메뉴를 닫아 onClick 이 무산되는 것을 막기 위해 자기 컨테이너 내부 클릭은 예외(SlotContextMenu 가드 동형).
  useEffect(() => {
    if (!rowMenu && !bgMenu && !picker) return
    const h = (e: MouseEvent) => {
      const t = e.target as Node
      if (rowMenu && rowMenuRef.current && !rowMenuRef.current.contains(t)) setRowMenu(null)
      if (bgMenu && bgMenuRef.current && !bgMenuRef.current.contains(t)) setBgMenu(null)
      if (picker && pickerRef.current && !pickerRef.current.contains(t)) setPicker(null)
    }
    document.addEventListener('mousedown', h)
    return () => document.removeEventListener('mousedown', h)
  }, [rowMenu, bgMenu, picker])

  // Escape 로 열린 행/배경 메뉴 닫기(픽커 input 은 자체 onKeyDown 으로 이미 Escape 처리 — 여기선 메뉴 전용).
  //   메뉴가 열려 있을 때만 리스너를 달고 닫힘/언마운트 시 해제한다(리스너 누수 방지).
  useEffect(() => {
    if (!rowMenu && !bgMenu) return
    const h = (e: KeyboardEvent) => {
      if (e.key !== 'Escape') return
      setRowMenu(null)
      setBgMenu(null)
    }
    document.addEventListener('keydown', h)
    return () => document.removeEventListener('keydown', h)
  }, [rowMenu, bgMenu])

  // in-flight 시작 — busyRef(동기 권위 가드)로 같은 tick 재호출 즉시 차단, busyIds(state)는 시각용 병행.
  const beginInFlight = (id: string): boolean => {
    if (busyRef.current.has(id)) return false // 동기 권위 가드(re-render 전 두 번째 호출 차단)
    busyRef.current.add(id) // async 진입 전 동기 lock
    setBusyIds(prev => new Set(prev).add(id)) // 시각(disabled/opacity)용 — 가드 아님
    return true
  }
  const endInFlight = (id: string) => {
    busyRef.current.delete(id) // 성공·실패 무관 lock 해제(에러가 UI 영구 잠금 방지)
    setBusyIds(prev => {
      const next = new Set(prev)
      next.delete(id)
      return next
    })
  }
  const setError = (id: string, msg: string) => setErrorById(prev => ({ ...prev, [id]: msg }))
  const clearError = (id: string) =>
    setErrorById(prev => {
      if (!(id in prev)) return prev
      const next = { ...prev }
      delete next[id]
      return next
    })

  // 예약 노드 활성화(spawnProfile) — AgentTree 와 동일 restore UX(리그레션 금지). 성공 시 프로필 refetch.
  const activateReserved = (agentId: string) => {
    if (!beginInFlight(agentId)) return
    clearError(agentId)
    agentClient
      .spawnProfile(agentId, false)
      .then(() => refreshProfiles())
      .catch(e => {
        console.error('[spawnProfile]', e)
        setError(agentId, `활성화 실패: ${String(e)}`)
      })
      .finally(() => endInFlight(agentId))
  }

  // "열기" = 이 에이전트를 main 활성 뷰의 포커스 슬롯에 배정(기존 assign 경로 재사용 — AgentTree 와 동일).
  //   agent-tree 창(config)엔 자기 슬롯 캔버스가 없어 currentViewId() 가 main 폴백을 준다(AgentTree 주석 참조).
  const openInFocusedSlot = (agentId: string) => {
    if (!beginInFlight(agentId)) return // 동기 가드 — 연타로 중복 assign 방지
    const vs = useViewStore.getState()
    const viewId = currentViewId()
    const slotId = selectView(vs, viewId)?.focusedSlotId
    if (!viewId || !slotId) {
      setError(agentId, '열기 실패: 활성 뷰/포커스 슬롯 없음')
      endInFlight(agentId) // 조기 반환에도 lock 해제(영구 잠금 방지)
      return
    }
    clearError(agentId)
    void vs
      .assignAgent(viewId, slotId, agentId)
      .catch(e => {
        console.error('[assignAgent]', e)
        setError(agentId, `열기 실패: ${String(e)}`)
      })
      .finally(() => endInFlight(agentId))
  }

  // 종료(kill) — 동기 가드로 연타 중복 kill 방지. 성공·실패 무관 lock 해제.
  const killAgentGuarded = (agentId: string) => {
    if (!beginInFlight(agentId)) return
    clearError(agentId)
    agentClient
      .killAgent(agentId)
      .catch(e => {
        console.error('[kill]', e)
        setError(agentId, `종료 실패: ${String(e)}`)
      })
      .finally(() => endInFlight(agentId))
  }

  // 예약 취소(삭제) — 예약 프로필을 데몬에서 제거(agentClient.deleteProfile). 다른 UI 로는 stale 예약 프로필을
  //   지울 수 없어 이 항목이 유일 경로(AgentTree reserved-row 의 "예약 취소" 리그레션 복원). 동기 가드 + 성공 시 refetch.
  const cancelReserved = (agentId: string) => {
    if (!beginInFlight(agentId)) return
    clearError(agentId)
    agentClient
      .deleteProfile(agentId)
      .then(() => refreshProfiles())
      .catch(e => {
        console.error('[deleteProfile]', e)
        setError(agentId, `예약 취소 실패: ${String(e)}`)
      })
      .finally(() => endInFlight(agentId))
  }

  // 에이전트 생성 — 프리셋(id→cwd 해소) 또는 경로 직접. agent.spawn command 로 라우팅(단일 제어 표면, §5).
  //   ★실패 시 픽커를 닫지 않고 인라인 에러 표시(AgentTree MAJOR-3)★ + ★run 의 동기 throw 도 잡는다★:
  //   run 은 미지 preset·빈 cwd·부모 검증 실패 시 동기 throw 할 수 있어 Promise.resolve(...).catch 로는
  //   못 잡는다. async try/catch 로 동기 throw 와 async reject 를 모두 잡고, 성공했을 때만 픽커를 닫는다.
  const spawnFromPreset = async (presetId: string) => {
    setPickerError(null) // 다음 시도 시 이전 에러 지움
    try {
      await run('agent.spawn', { preset: presetId })
      setPicker(null) // 성공 시에만 닫음
    } catch (e) {
      console.error('[agent.spawn preset]', e)
      setPickerError(`생성 실패: ${String(e)}`) // 실패 시 픽커 유지 + 인라인 표시
    }
  }
  const spawnFromPath = async () => {
    const cwd = pickerDraft.trim()
    if (!cwd) return
    setPickerError(null)
    try {
      await run('agent.spawn', { cwd })
      setPickerDraft('') // 성공 시에만 입력 비움
      setPicker(null)
    } catch (e) {
      console.error('[agent.spawn cwd]', e)
      setPickerError(`생성 실패: ${String(e)}`)
    }
  }

  // 행 우클릭 메뉴 항목 — kind 로 분기. reserved 는 활성화/예약취소, running 은 열기/종료/이름변경/재시작.
  //   이름변경·재시작은 백엔드 command 부재 → disabled "준비 중"(날조 금지, ADR-0011).
  //   disabled 시각 판정은 busyIds(state)로 — 실제 중복 발화 차단은 각 핸들러의 busyRef 동기 가드가 담당.
  const rowMenuItems: Array<{ label: string; disabled: boolean; action: () => void }> = !rowMenu
    ? []
    : rowMenu.kind === 'reserved'
      ? [
          {
            label: '활성화(spawn)',
            disabled: busyIds.has(rowMenu.agentId),
            action: () => activateReserved(rowMenu.agentId),
          },
          // ★예약 취소(삭제) — AgentTree reserved-row 리그레션 복원★: 이 항목 외엔 stale 예약 프로필을 지울
          //   UI 가 없다(deleteProfile 유일 경로). 동기 가드는 cancelReserved 내부 busyRef.
          {
            label: '예약 취소',
            disabled: busyIds.has(rowMenu.agentId),
            action: () => cancelReserved(rowMenu.agentId),
          },
        ]
      : [
          { label: '열기', disabled: busyIds.has(rowMenu.agentId), action: () => openInFocusedSlot(rowMenu.agentId) },
          {
            label: '종료',
            disabled: busyIds.has(rowMenu.agentId),
            action: () => killAgentGuarded(rowMenu.agentId),
          },
          // ★준비 중(백엔드 command 없음)★: 이름은 cwd basename 으로 파생돼 저장 이름 자체가 없다(rename
          //   대상 부재). 재시작 전용 command 도 protocolClient 에 없다(kill→re-spawn 조합뿐). 날조 금지 —
          //   실제 command 가 생기면 배선한다(ADR-0011).
          { label: '이름변경 (준비 중)', disabled: true, action: () => {} },
          { label: '재시작 (준비 중)', disabled: true, action: () => {} },
        ]

  return (
    <div
      data-agent-list="1"
      style={{ flex: 1, overflow: 'auto', minHeight: 0, height: '100%', background: 'var(--bg-secondary)' }}
      // 빈 영역 우클릭 → 배경 메뉴(에이전트 생성). 행에서의 우클릭은 행 핸들러가 stopPropagation 으로 가로챈다.
      //   상호배타(FIX#4): 메뉴를 열 때 픽커·다른 메뉴를 닫아 {행메뉴, 배경메뉴, 픽커} 중 하나만 열리게 한다.
      onContextMenu={e => {
        e.preventDefault()
        setRowMenu(null)
        setPicker(null)
        setBgMenu({ x: e.clientX, y: e.clientY })
      }}
    >
      {/* 슬롯 콘텐츠 라벨(사용자 요청) — 이 슬롯 = 에이전트 트리. 공용 슬롯 헤더가 아니라 PresetPalette·
          AgentList 이 2개 variant 컴포넌트에만 각자 넣는다. root 가 스크롤 컨테이너라 sticky 로 상단 고정. 변수-only. */}
      <div
        data-slot-label="agent-list"
        style={{
          position: 'sticky',
          top: 0,
          zIndex: 1,
          padding: '6px 8px',
          borderBottom: '1px solid var(--border)',
          background: 'var(--bg-secondary)',
          color: 'var(--text-muted)',
          fontFamily: 'var(--font-ui)',
          fontSize: '11px',
          fontWeight: 600,
          letterSpacing: '0.03em',
        }}
      >
        에이전트 트리
      </div>
      {rows.length === 0 ? (
        <div
          style={{
            height: '100%',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            color: 'var(--text-muted)',
            fontFamily: 'var(--font-ui)',
            fontSize: '12px',
          }}
        >
          에이전트 없음 — 우클릭으로 생성
        </div>
      ) : (
        rows.map(node => {
          const isReserved = node.kind === 'reserved'
          const isBusy = busyIds.has(node.id)
          const err = errorById[node.id]
          return (
            <div
              key={node.id}
              data-agent-row={node.id}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: '6px',
                padding: '4px 8px',
                cursor: isBusy ? 'wait' : 'pointer',
                background:
                  selectedAgentId === node.id
                    ? 'color-mix(in srgb, var(--accent) 15%, transparent)'
                    : 'transparent',
                fontFamily: 'var(--font-ui)',
                fontSize: '12px',
                // 예약(깡통)은 흐리게 + 이탤릭 — 미spawn 시각 구분(색 리터럴 없이 muted 변수·기울임).
                color: isReserved ? 'var(--text-muted)' : 'var(--text)',
                fontStyle: isReserved ? 'italic' : 'normal',
                opacity: isBusy ? 0.6 : 1,
                userSelect: 'none',
              }}
              onClick={() => setSelectedAgent(node.id)}
              // 더블클릭: 예약 행 → 활성화(spawn). 실행중 행 → no-op(AgentTree 동작 유지).
              onDoubleClick={() => {
                if (node.kind === 'reserved') activateReserved(node.id)
              }}
              title={err ?? (isReserved ? '더블클릭으로 활성화(spawn)' : node.cwd)}
              onContextMenu={e => {
                e.preventDefault()
                e.stopPropagation() // 배경 메뉴가 뜨지 않게 — 행 메뉴가 이긴다.
                setBgMenu(null)
                setPicker(null) // 상호배타(FIX#4): 행 메뉴 열 때 픽커 닫음
                setSelectedAgent(node.id)
                setRowMenu({ x: e.clientX, y: e.clientY, agentId: node.id, kind: node.kind })
              }}
            >
              {/* 상태 = 글리프 모양(색 아님, ADR-0062). muted 변수로만 렌더 — 모양이 상태를 담는다. */}
              <span data-agent-glyph="1" style={{ fontSize: '11px', color: 'var(--text-muted)', flexShrink: 0 }}>
                {statusGlyph(node.status)}
              </span>
              {/* 표시명 = cwd basename(프론트 파생 — 이름 미저장). cwd 는 노출 안 함(title 로만). */}
              <span
                data-agent-name="1"
                style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}
              >
                {basename(node.cwd)}
              </span>
              {err && (
                <span style={{ marginLeft: 'auto', color: 'var(--text-muted)', fontSize: '10px', flexShrink: 0 }}>
                  실패
                </span>
              )}
            </div>
          )
        })
      )}

      {/* ── 행 우클릭 메뉴 ─────────────────────────────────────────────── */}
      {rowMenu && (
        <div ref={rowMenuRef} style={MENU_STYLE(rowMenu.x, rowMenu.y)}>
          {rowMenuItems.map(item => (
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

      {/* ── 배경(빈 영역) 우클릭 메뉴 → 에이전트 생성 ───────────────────── */}
      {bgMenu && (
        <div ref={bgMenuRef} style={MENU_STYLE(bgMenu.x, bgMenu.y)}>
          <div
            data-agent-create="1"
            style={MENU_ITEM_STYLE(false)}
            onMouseEnter={e => (e.currentTarget.style.background = 'color-mix(in srgb, var(--accent) 20%, transparent)')}
            onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
            onClick={e => {
              e.stopPropagation()
              const at = { x: bgMenu.x, y: bgMenu.y }
              setRowMenu(null) // 상호배타(FIX#4): 픽커 열 때 메뉴 전부 닫음
              setBgMenu(null)
              setPickerDraft('')
              setPickerError(null) // 새 생성 흐름 — 이전 에러 지움
              setPicker(at) // 픽커를 같은 좌표에 연다.
            }}
          >
            에이전트 생성
          </div>
        </div>
      )}

      {/* ── 프리셋 픽커(에이전트 생성) ─────────────────────────────────── */}
      {picker && (
        <div
          ref={pickerRef}
          data-agent-picker="1"
          style={{
            ...MENU_STYLE(picker.x, picker.y),
            minWidth: '200px',
            maxHeight: '280px',
            overflow: 'auto',
            display: 'flex',
            flexDirection: 'column',
          }}
        >
          {/* 경로 직접 입력 행 — Enter 로 agent.spawn({cwd}). */}
          <div style={{ padding: '6px 8px', borderBottom: '1px solid var(--border)', display: 'flex', gap: '4px' }}>
            <input
              autoFocus
              data-agent-picker-input="1"
              value={pickerDraft}
              onChange={e => setPickerDraft(e.target.value)}
              onKeyDown={e => {
                if (e.key === 'Enter') void spawnFromPath()
                if (e.key === 'Escape') setPicker(null)
              }}
              placeholder="새 경로 직접"
              style={{
                flex: 1,
                minWidth: 0,
                padding: '3px 6px',
                background: 'var(--bg)',
                color: 'var(--text)',
                border: '1px solid var(--border)',
                borderRadius: '3px',
                fontFamily: 'var(--font-ui)',
                fontSize: '11px',
                outline: 'none',
              }}
            />
            <button
              data-agent-picker-go="1"
              onClick={() => void spawnFromPath()}
              style={{
                flexShrink: 0,
                padding: '3px 8px',
                background: 'var(--surface-elevated)',
                color: 'var(--text)',
                border: '1px solid var(--border)',
                borderRadius: '3px',
                cursor: 'pointer',
                fontFamily: 'var(--font-ui)',
                fontSize: '11px',
              }}
            >
              생성
            </button>
          </div>
          {/* 등록된 프리셋 목록 — 표시명 = cwd basename. 선택 시 agent.spawn({preset:id}). */}
          {presets.length === 0 ? (
            <div style={{ padding: '8px', color: 'var(--text-muted)', fontFamily: 'var(--font-ui)', fontSize: '11px' }}>
              프리셋 없음 — 위에 경로 입력
            </div>
          ) : (
            presets.map(preset => (
              <div
                key={preset.id}
                data-agent-picker-preset={preset.id}
                style={MENU_ITEM_STYLE(false)}
                title={preset.cwd}
                onMouseEnter={e => (e.currentTarget.style.background = 'color-mix(in srgb, var(--accent) 20%, transparent)')}
                onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
                onClick={e => {
                  e.stopPropagation()
                  void spawnFromPreset(preset.id)
                }}
              >
                {basename(preset.cwd)}
              </div>
            ))
          )}
          {/* ★생성 실패 인라인 표시(FIX#2)★: spawn 실패 시 픽커를 닫지 않고 여기 메시지를 띄운다. 색 리터럴
              없이 var(--text)(가독) + border-top(--border)로만 — danger 토큰이 없어 강조는 텍스트/보더로 대체. */}
          {pickerError && (
            <div
              data-agent-picker-error="1"
              style={{
                padding: '6px 8px',
                borderTop: '1px solid var(--border)',
                color: 'var(--text)',
                fontFamily: 'var(--font-ui)',
                fontSize: '11px',
                whiteSpace: 'normal',
                wordBreak: 'break-word',
              }}
            >
              {pickerError}
            </div>
          )}
        </div>
      )}
    </div>
  )
}

// 메뉴/픽커 공통 스타일(SlotContextMenu·AgentTree 인라인 메뉴와 동형 — 변수-only).
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
