//! PresetPalette — 프리셋(자주 쓰는 cwd 북마크) 슬롯 콘텐츠(ADR-0060 variant / ADR-0061 저장).
//!
//! ★역할★: 데몬 소유 프리셋 목록(agentStore.presets)을 그리고, add-path 입력·행별 삭제로 프리셋을
//! CRUD 한다. CRUD 는 agentClient(단일 제어 표면, ADR-0011) 프리셋 메서드만 부르고, 화면 반영은 낙관
//! 갱신 없이 PresetListUpdated broadcast → store 미러 교체로만 이뤄진다(멀티창 동기화 불변식, ADR-0061).
//!
//! ★표시명 = cwd basename(프론트 파생, ADR-0061)★: 프리셋은 {id,cwd}만 저장하고 이름을 저장하지 않는다.
//! 그래서 행 라벨은 여기서 cwd 의 마지막 경로 세그먼트로 파생한다(win/posix 구분자 모두 처리).
//!
//! ★스타일 = 변수-only(테마 준수)★: 색·폰트는 전부 CSS 변수(var(--bg)/--text/--border/--accent/…) 참조 —
//! 하드코딩 색 리터럴 0. e-ink 테마(고대비 흑백)에서도 깨지지 않게 한다.
//!
//! ★범위(Slice B)★: 프리셋 소비 + 팔레트 UI 만. 프리셋에서 스폰(클릭→에이전트 생성)·우클릭 메뉴는
//! Slice C 다 — 여기선 목록/추가/삭제만.

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
  const [draft, setDraft] = useState('')
  // in-flight 가드(로컬 컴포넌트 상태 only — store 낙관 갱신 금지, ADR-0061). Ack 전 Enter/버튼 중복
  //   제출이 duplicate createPreset/DeletePreset 을 쏘는 것을 막는다. 성공·실패 무관 완료 시 해제.
  //
  // ★ref = 권위적 double-fire 가드, state = 시각(disabled/opacity)★: useState 가드는 re-render commit
  //   전 두 번째 호출이 stale closure 로 `creating` 을 아직 false 로 읽어 둘 다 통과하는 창이 있다
  //   (render 타이밍 의존). ref 는 동기 mutable 이라 같은 tick 내 두 번째 호출도 즉시 true 를 읽는다 —
  //   그래서 ref 가 실제 중복 발화 차단을 담당하고, state 는 순수 시각 표시용으로만 병행한다.
  const creatingRef = useRef(false)
  const deletingRef = useRef<Set<string>>(new Set())
  const [creating, setCreating] = useState(false)
  const [deleting, setDeleting] = useState<ReadonlySet<string>>(() => new Set())

  // 추가: agentClient.createPreset(cwd) → Ack. 생성분은 뒤이은 PresetListUpdated broadcast 로 store 에
  //   들어온다(낙관 갱신 안 함, ADR-0061). 빈 입력은 무시. 성공 시 입력창 비운다.
  //   in-flight 중이면 중복 제출 무시(Enter 두 번·Enter+버튼).
  const addPreset = (): void => {
    if (creatingRef.current) return // 동기 권위 가드(같은 tick 두 번째 호출 즉시 차단)
    const cwd = draft.trim()
    if (!cwd) return
    creatingRef.current = true // async 진입 전 동기 lock
    setCreating(true) // 시각(disabled/opacity)용 — 가드 아님
    agentClient
      .createPreset(cwd)
      .then(() => setDraft('')) // 성공 시에만 입력창 비움(실패 시 재시도 위해 유지)
      .catch(e => console.error('[PresetPalette] createPreset 실패:', e))
      .finally(() => {
        creatingRef.current = false // 성공·실패 무관 lock 해제(에러가 UI 영구 잠금 방지)
        setCreating(false)
      })
  }

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
      {/* add-path 입력 행 — Enter 또는 추가 버튼으로 createPreset(cwd). */}
      <div
        style={{
          display: 'flex',
          gap: '6px',
          padding: '8px',
          borderBottom: '1px solid var(--border)',
        }}
      >
        <input
          data-preset-input="1"
          value={draft}
          disabled={creating}
          onChange={e => setDraft(e.target.value)}
          onKeyDown={e => {
            if (e.key === 'Enter') addPreset()
          }}
          placeholder="작업 디렉토리 경로"
          style={{
            flex: 1,
            minWidth: 0,
            padding: '4px 8px',
            background: 'var(--bg-secondary)',
            color: 'var(--text)',
            border: '1px solid var(--border)',
            borderRadius: '3px',
            fontFamily: 'var(--font-ui)',
            fontSize: '12px',
            outline: 'none',
            opacity: creating ? 0.6 : 1, // in-flight 시각 표시(색 리터럴 없이 opacity 만)
          }}
        />
        <button
          data-preset-add="1"
          onClick={addPreset}
          disabled={creating}
          style={{
            padding: '4px 10px',
            background: 'var(--surface-elevated)',
            color: 'var(--text)',
            border: '1px solid var(--border)',
            borderRadius: '3px',
            cursor: creating ? 'default' : 'pointer',
            fontFamily: 'var(--font-ui)',
            fontSize: '12px',
            opacity: creating ? 0.6 : 1,
          }}
        >
          추가
        </button>
      </div>

      {/* 프리셋 목록 — 각 행: cwd basename(표시명) + 전체 cwd(muted) + 삭제. */}
      <div style={{ flex: 1, overflow: 'auto' }}>
        {presets.length === 0 ? (
          <div style={{ padding: '12px', color: 'var(--text-muted)', fontSize: '12px' }}>
            프리셋 없음 — 위에서 경로를 추가하세요.
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
