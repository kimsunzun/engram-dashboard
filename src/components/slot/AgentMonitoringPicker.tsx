// ADR-0067: 에이전트 모니터링 검색 팝업(command-palette식) — 우클릭한 slot 에 실행중 에이전트를 배정한다.
//
// ★역할★: slot 우클릭 "에이전트 모니터링"(slot.assignRunningAgent command)이 monitoringPickerStore.open
//   (viewId, slotId)로 타깃을 실어 이 팝업을 연다. 검색창 + 실행중 에이전트 필터 목록을 그리고, 고르면
//   그 slot 에 assign 한다. 팝업 자체는 배치 상태를 갖지 않는다 — on-select 가 viewStore.assignAgent 로
//   흘려 §5 단일 제어 표면(assign_agent)을 지난다(별도 배치 상태 금지, ADR-0067 불변식).
//
// ★배치 타깃 = 우클릭한 slot(명시)★: 타깃 좌표는 store 에 담긴 (viewId, slotId) — 포커스에 의존하지 않는다
//   (focus-steal 원천 차단). 우클릭/팝업 열기는 focused_slot_id 를 건드리지 않는다(ADR-0067 포커스 불변식).
//
// ★낙관 갱신 X(ADR-0035)★: assignAgent 는 invoke(assign_agent) 만 부르고 화면은 layout:updated emit 으로만
//   반영된다 — 팝업은 assign 을 흘린 뒤 즉시 닫고, 실제 slot 콘텐츠 교체는 백엔드 권위 emit 이 그린다.
//
// ★창별 단일 마운트★: WindowLayout 이 창마다 하나 마운트하고, target 이 null 이면 아무것도 렌더하지 않는다
//   (닫힘). SlotContextMenu 의 fixed 오버레이 패턴과 동형 — 변수-only 스타일(테마 준수, 색 리터럴 0).
//
// ★스타일 = 변수-only★: 색·폰트는 전부 CSS 변수(PresetPalette/AgentList 와 동일 규율 — e-ink 대비).

import { useEffect, useRef, useState } from 'react'

import { ScrollArea } from '../ui/scroll-area'
import { useAgentStore } from '../../store/agentStore'
import { useMonitoringPickerStore } from '../../store/monitoringPickerStore'
import { useViewStore } from '../../store/viewStore'
import { filterMonitoringCandidates } from './monitoringPickerFilter'

export default function AgentMonitoringPicker() {
  const target = useMonitoringPickerStore(s => s.target)
  const close = useMonitoringPickerStore(s => s.close)
  const agents = useAgentStore(s => s.agents)

  const [query, setQuery] = useState('')
  // 키보드 선택 하이라이트(Enter 로 배정) — 목록 인덱스. 검색어 변경 시 0 으로 리셋.
  const [activeIndex, setActiveIndex] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)

  const candidates = filterMonitoringCandidates(agents, query)

  // 마운트 직후 검색창 자동 포커스. key={openId} 로 open() 마다 fresh remount 되므로(ADR-0067 — monitoringPickerStore.openId)
  // query/activeIndex 리셋은 useState 초기값이 담당한다 — 여기선 포커스만.
  useEffect(() => {
    if (!target) return
    // 마운트 직후 포커스(팝업이 방금 열림). requestAnimationFrame 없이도 ref 는 이 시점 확정.
    inputRef.current?.focus()
  }, [target])

  // 검색어가 바뀌면 하이라이트를 첫 항목으로(범위 밖 인덱스가 남지 않게).
  useEffect(() => {
    setActiveIndex(0)
  }, [query])

  // Esc 로 닫기 — 열려 있을 때만 리스너 부착(누수 방지). 입력창 keydown 과 별개(백드롭 밖 전역 Esc 도 잡음).
  useEffect(() => {
    if (!target) return
    const h = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close()
    }
    document.addEventListener('keydown', h)
    return () => document.removeEventListener('keydown', h)
  }, [target, close])

  // 닫힘 상태 = 아무것도 렌더하지 않음(SlotContextMenu 의 조건부 마운트와 동형).
  if (!target) return null

  // ADR-0067: on-select 배치 = viewStore.assignAgent(우클릭한 slot 좌표) → invoke(assign_agent) → emit.
  //   §5 단일 제어 표면을 지난다(별도 배치 상태 없음). 낙관 갱신 X — 화면은 emit 으로만. 실패는 콘솔 경고
  //   (토스트 없음 — AgentList openInFocusedSlot 과 동형 처리). 성공·실패 무관 팝업은 닫는다.
  const selectAgent = (agentId: string) => {
    void useViewStore
      .getState()
      .assignAgent(target.viewId, target.slotId, agentId)
      .catch(e => console.error('[AgentMonitoringPicker] assignAgent 실패:', e))
    close()
  }

  // 목록 키보드 내비 — ↑/↓ 이동, Enter 배정. 입력창에 포커스가 있어도 동작하도록 input onKeyDown 에 건다.
  const onInputKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault()
      setActiveIndex(i => Math.min(i + 1, Math.max(0, candidates.length - 1)))
    } else if (e.key === 'ArrowUp') {
      e.preventDefault()
      setActiveIndex(i => Math.max(i - 1, 0))
    } else if (e.key === 'Enter') {
      e.preventDefault()
      const chosen = candidates[activeIndex]
      if (chosen) selectAgent(chosen.id)
    }
    // Escape 는 위 전역 리스너가 처리.
  }

  return (
    // 백드롭 — 클릭 시 닫기(팝업 본체 클릭은 stopPropagation 으로 통과). fixed 전체 덮기 + 반투명(변수 없이
    //   rgba 는 오버레이 관용 — 색조가 아니라 명도 딤이라 테마 무관). zIndex 는 SlotContextMenu(1000/1001) 위.
    <div
      data-monitoring-picker-backdrop="1"
      onClick={close}
      style={{
        position: 'fixed',
        inset: 0,
        background: 'rgba(0,0,0,0.4)',
        zIndex: 2000,
        display: 'flex',
        alignItems: 'flex-start',
        justifyContent: 'center',
        paddingTop: '12vh',
      }}
    >
      <div
        data-monitoring-picker="1"
        onClick={e => e.stopPropagation()}
        style={{
          width: 'min(480px, 90vw)',
          maxHeight: '60vh',
          display: 'flex',
          flexDirection: 'column',
          background: 'var(--bg-secondary)',
          color: 'var(--text)',
          border: '1px solid var(--border)',
          borderRadius: '6px',
          boxShadow: '0 8px 32px rgba(0,0,0,0.4)',
          fontFamily: 'var(--font-ui)',
          fontSize: '13px',
          overflow: 'hidden',
        }}
      >
        {/* 라벨 — 이 팝업의 목적(우클릭한 slot 에 실행중 에이전트 배정). 변수-only. */}
        <div
          style={{
            padding: '6px 10px',
            borderBottom: '1px solid var(--border)',
            color: 'var(--text-muted)',
            fontSize: '11px',
            fontWeight: 600,
            letterSpacing: '0.03em',
            flexShrink: 0,
          }}
        >
          에이전트 모니터링 — 이 슬롯에 실행중 에이전트 배정
        </div>

        {/* 검색창 — 자동 포커스(위 effect). 표시명·cwd 부분일치. */}
        <input
          ref={inputRef}
          data-monitoring-picker-search="1"
          value={query}
          onChange={e => setQuery(e.target.value)}
          onKeyDown={onInputKeyDown}
          placeholder="에이전트 검색 (이름·경로)"
          style={{
            margin: '8px',
            padding: '6px 8px',
            background: 'var(--bg)',
            color: 'var(--text)',
            border: '1px solid var(--border)',
            borderRadius: '4px',
            fontFamily: 'var(--font-ui)',
            fontSize: '13px',
            outline: 'none',
            flexShrink: 0,
          }}
        />

        {/* 후보 목록 — 실행중 필터 + 검색어 좁힘. 빈 상태 메시지 분기. 공용 ScrollArea seam(ADR-0053)으로
            스크롤(raw overflow:auto → 오버레이 스크롤바). 평면 목록(가상화 없음)이라 Viewport 로 감싸도 무해. */}
        <ScrollArea style={{ flex: 1, minHeight: 0 }}>
          {candidates.length === 0 ? (
            <div
              data-monitoring-picker-empty="1"
              style={{ padding: '12px', color: 'var(--text-muted)', fontSize: '12px' }}
            >
              {agents.some(a => a.status.type === 'Running')
                ? '검색 결과 없음'
                : '실행중 에이전트 없음 — 트리에서 에이전트를 생성/활성화하세요.'}
            </div>
          ) : (
            candidates.map((c, i) => (
              <div
                key={c.id}
                data-monitoring-candidate={c.id}
                onClick={() => selectAgent(c.id)}
                onMouseEnter={() => setActiveIndex(i)}
                title={c.cwd}
                style={{
                  padding: '6px 10px',
                  cursor: 'pointer',
                  borderBottom: '1px solid var(--border)',
                  // 하이라이트 = 키보드 activeIndex(마우스 hover 도 activeIndex 로 동기화). 색 리터럴 없이 accent mix.
                  background:
                    i === activeIndex ? 'color-mix(in srgb, var(--accent) 20%, transparent)' : 'transparent',
                }}
              >
                <div
                  style={{ color: 'var(--text)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}
                >
                  {c.name}
                </div>
                <div
                  style={{
                    color: 'var(--text-muted)',
                    fontSize: '11px',
                    whiteSpace: 'nowrap',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                  }}
                >
                  {c.cwd}
                </div>
              </div>
            ))
          )}
        </ScrollArea>
      </div>
    </div>
  )
}
