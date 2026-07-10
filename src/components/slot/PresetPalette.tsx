//! PresetPalette — 프리셋(자주 쓰는 cwd 북마크) 슬롯 콘텐츠(ADR-0060 variant / ADR-0061 저장).
//!
//! ★역할★: 데몬 소유 프리셋 목록(agentStore.presets)을 그리고 행별 삭제로 CRUD 한다. CRUD 는
//! agentClient(단일 제어 표면, ADR-0011) 프리셋 메서드만 부르고, 화면 반영은 낙관 갱신 없이
//! PresetListUpdated broadcast → store 미러 교체로만 이뤄진다(멀티창 동기화 불변식, ADR-0061).
//!
//! ★pane 메뉴 없음(ADR-0064)★: 옛 pane 우클릭 "추가" 메뉴 + stopPropagation 은 제거됐다 — 추가는 이제
//! 통합 슬롯 메뉴의 preset.add command(폴더 다이얼로그 → createPreset, presetCommands.ts)로 기여된다.
//! pane 우클릭은 상위 ViewLayoutRenderer 의 통합 SlotContextMenu 로 버블한다(공통 슬롯 ops 도 함께 노출 —
//! 옛 구조에선 프리셋 슬롯이 닫기·분할조차 못 하던 버그를 해소). 이 컴포넌트는 목록/삭제만 소유한다.
//!
//! ★표시명 = cwd basename(프론트 파생, ADR-0061)★: 프리셋은 {id,cwd}만 저장하고 이름을 저장하지 않는다.
//! 그래서 행 라벨은 여기서 cwd 의 마지막 경로 세그먼트로 파생한다(공용 basename 유틸 — AgentList 와 단일 출처).
//!
//! ★스타일 = 변수-only(테마 준수)★: 색·폰트는 전부 CSS 변수 참조 — 하드코딩 색 리터럴 0(e-ink 대비).

import { useRef, useState } from 'react'

import { agentClient } from '../../api/clientFactory'
import { useAgentStore } from '../../store/agentStore'
import { basename } from '../../util/basename'

/**
 * 프리셋 표시명 = cwd basename(프론트 파생, ADR-0061 — 이름 미저장). 실제 파생 규칙은 공용 `basename`
 * 유틸에 있다(AgentList 행 표시명과 단일 출처 공유 — 복제 시 win/posix·root 엣지가 갈린다). 여기선
 * 도메인 의미(프리셋 표시명)를 이름으로 노출만 하고 규칙은 위임한다.
 */
export function presetDisplayName(cwd: string): string {
  return basename(cwd)
}

export default function PresetPalette() {
  const presets = useAgentStore(s => s.presets)
  // in-flight 가드(로컬 컴포넌트 상태 only — store 낙관 갱신 금지, ADR-0061). Ack 전 중복 제출이
  //   duplicate DeletePreset 을 쏘는 것을 막는다. 성공·실패 무관 완료 시 해제.
  //
  // ★ref = 권위적 double-fire 가드, state = 시각(disabled/opacity)★: useState 가드는 re-render commit
  //   전 두 번째 호출이 stale closure 로 아직 false 를 읽어 둘 다 통과하는 창이 있다. ref 는 동기 mutable 이라
  //   같은 tick 내 두 번째 호출도 즉시 true 를 읽는다 — ref 가 실제 중복 발화 차단을 담당하고 state 는 시각용.
  const deletingRef = useRef<Set<string>>(new Set())
  const [deleting, setDeleting] = useState<ReadonlySet<string>>(() => new Set())

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
        프리셋
      </div>

      {/* 프리셋 목록 — 각 행: cwd basename(표시명) + 전체 cwd(muted) + 삭제. */}
      <div style={{ flex: 1, overflow: 'auto' }}>
        {presets.length === 0 ? (
          <div style={{ padding: '12px', color: 'var(--text-muted)', fontSize: '12px' }}>
            프리셋 없음 — 우클릭 "추가"로 폴더를 선택하세요.
          </div>
        ) : (
          presets.map(preset => {
            const isDeleting = deleting.has(preset.id)
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
              }}
            >
              <div style={{ flex: 1, minWidth: 0 }}>
                {/* 표시명 = cwd basename(프론트 파생 — 이름 미저장, ADR-0061). */}
                <div
                  data-preset-name="1"
                  style={{ color: 'var(--text)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}
                >
                  {presetDisplayName(preset.cwd)}
                </div>
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
              <button
                data-preset-delete={preset.id}
                aria-label="프리셋 삭제"
                onClick={() => removePreset(preset.id)}
                disabled={isDeleting}
                style={{
                  flexShrink: 0,
                  padding: '2px 8px',
                  background: 'transparent',
                  color: 'var(--text-muted)',
                  border: '1px solid var(--border)',
                  borderRadius: '3px',
                  cursor: isDeleting ? 'default' : 'pointer',
                  fontFamily: 'var(--font-ui)',
                  fontSize: '11px',
                  opacity: isDeleting ? 0.6 : 1, // in-flight 시각 표시(색 리터럴 없이 opacity 만)
                }}
              >
                삭제
              </button>
            </div>
            )
          })
        )}
      </div>
    </div>
  )
}
