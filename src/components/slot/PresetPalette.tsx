//! PresetPalette — 프리셋(자주 쓰는 cwd 북마크) 슬롯 콘텐츠(ADR-0060 variant / ADR-0061 저장).
//!
//! ★역할★: 데몬 소유 프리셋 목록(agentStore.presets)을 그리고, 우클릭 "추가"(네이티브 폴더 다이얼로그)·
//! 행별 삭제로 프리셋을 CRUD 한다. CRUD 는 agentClient(단일 제어 표면, ADR-0011) 프리셋 메서드만 부르고,
//! 화면 반영은 낙관 갱신 없이 PresetListUpdated broadcast → store 미러 교체로만 이뤄진다(멀티창 동기화
//! 불변식, ADR-0061).
//!
//! ★경로 추가 = 네이티브 폴더 픽커(탑바 텍스트 입력 대체)★: 팔레트 pane 우클릭 → "추가" → OS 폴더 선택
//! 다이얼로그(@tauri-apps/plugin-dialog open({directory:true})). 고른 디렉토리(non-null)를 createPreset 에
//! 넘긴다. 취소(null) 면 no-op. 다이얼로그 open 은 네이티브 창(webview 밖)이라 cdp 로 검증 불가 —
//! 픽커→createPreset 배선만 단위테스트로 단언한다.
//!
//! ★표시명 = cwd basename(프론트 파생, ADR-0061)★: 프리셋은 {id,cwd}만 저장하고 이름을 저장하지 않는다.
//! 그래서 행 라벨은 여기서 cwd 의 마지막 경로 세그먼트로 파생한다(win/posix 구분자 모두 처리).
//!
//! ★스타일 = 변수-only(테마 준수)★: 색·폰트는 전부 CSS 변수(var(--bg)/--text/--border/--accent/…) 참조 —
//! 하드코딩 색 리터럴 0. e-ink 테마(고대비 흑백)에서도 깨지지 않게 한다.
//!
//! ★우클릭 메뉴(AgentList 동형)★: pane 우클릭 → fixed-position 메뉴(현재 항목 "추가" 1개 — 더 늘리는 건
//! 후속 사용자 스펙). onContextMenu 는 preventDefault+stopPropagation 으로 상위 SlotContextMenu(제네릭
//! 슬롯 메뉴)가 팔레트 pane 위에서 같이 열리지 않게 막는다. 바깥 클릭/Escape 로 닫고 리스너 누수 없음.

import { useEffect, useRef, useState } from 'react'

import { open } from '@tauri-apps/plugin-dialog'

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

/** pane 우클릭 메뉴 좌표(AgentList 의 BgMenu 동형 — primitive snapshot 만). */
type PaneMenu = { x: number; y: number }

export default function PresetPalette() {
  const presets = useAgentStore(s => s.presets)
  const menuRef = useRef<HTMLDivElement>(null)
  const [menu, setMenu] = useState<PaneMenu | null>(null)
  // 픽커 열림·에러 표시(에이전트 id 없는 생성 흐름 — 인라인 힌트). 다음 시도 때 지운다.
  const [addError, setAddError] = useState<string | null>(null)
  // in-flight 가드(로컬 컴포넌트 상태 only — store 낙관 갱신 금지, ADR-0061). Ack 전 중복 제출이
  //   duplicate createPreset/DeletePreset 을 쏘는 것을 막는다. 성공·실패 무관 완료 시 해제.
  //
  // ★ref = 권위적 double-fire 가드, state = 시각(disabled/opacity)★: useState 가드는 re-render commit
  //   전 두 번째 호출이 stale closure 로 `creating` 을 아직 false 로 읽어 둘 다 통과하는 창이 있다
  //   (render 타이밍 의존). ref 는 동기 mutable 이라 같은 tick 내 두 번째 호출도 즉시 true 를 읽는다 —
  //   그래서 ref 가 실제 중복 발화 차단을 담당하고, state 는 순수 시각 표시용으로만 병행한다.
  const creatingRef = useRef(false)
  const deletingRef = useRef<Set<string>>(new Set())
  const [creating, setCreating] = useState(false)
  const [deleting, setDeleting] = useState<ReadonlySet<string>>(() => new Set())

  // 메뉴 바깥 클릭으로 닫기(자기 ref 밖 mousedown 이면 닫음). 항목 클릭의 mousedown 이 먼저 메뉴를 닫아
  //   onClick 이 무산되는 것을 막기 위해 자기 컨테이너 내부 클릭은 예외(AgentList/SlotContextMenu 가드 동형).
  //   메뉴가 열려 있을 때만 리스너를 달고 닫힘/언마운트 시 해제한다(리스너 누수 방지).
  useEffect(() => {
    if (!menu) return
    const h = (e: MouseEvent) => {
      const t = e.target as Node
      if (menuRef.current && !menuRef.current.contains(t)) setMenu(null)
    }
    document.addEventListener('mousedown', h)
    return () => document.removeEventListener('mousedown', h)
  }, [menu])

  // Escape 로 열린 메뉴 닫기 — 열려 있을 때만 리스너를 달고 닫힘/언마운트 시 해제(누수 방지).
  useEffect(() => {
    if (!menu) return
    const h = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setMenu(null)
    }
    document.addEventListener('keydown', h)
    return () => document.removeEventListener('keydown', h)
  }, [menu])

  // 추가: 네이티브 폴더 다이얼로그 → 고른 디렉토리(non-null)를 agentClient.createPreset(cwd) 로 넘긴다.
  //   생성분은 뒤이은 PresetListUpdated broadcast 로 store 에 들어온다(낙관 갱신 안 함, ADR-0061). 취소
  //   (null)면 no-op. in-flight 중이면 중복 발화 무시(다이얼로그 이중 오픈·연타 방지).
  //   ★open 은 async 라 creatingRef 를 async 진입 전 동기 set 해야 같은 tick 재호출을 즉시 막는다★.
  const addPreset = async (): Promise<void> => {
    if (creatingRef.current) return // 동기 권위 가드(같은 tick 두 번째 호출 즉시 차단)
    creatingRef.current = true // async(다이얼로그+createPreset) 진입 전 동기 lock
    setCreating(true) // 시각(disabled/opacity)용 — 가드 아님
    setAddError(null)
    try {
      // 네이티브 OS 폴더 선택 창(webview 밖). directory+multiple:false → 반환은 string | null.
      const picked = await open({ directory: true, multiple: false, title: '프리셋 경로 선택' })
      // 취소(null) 또는 (방어적) 배열이면 no-op. 정상은 단일 경로 문자열.
      const cwd = typeof picked === 'string' ? picked : null
      if (cwd) {
        await agentClient.createPreset(cwd)
      }
    } catch (e) {
      console.error('[PresetPalette] 폴더 선택/createPreset 실패:', e)
      setAddError(`추가 실패: ${String(e)}`) // 인라인 힌트(조용히 삼키지 않음)
    } finally {
      creatingRef.current = false // 성공·실패 무관 lock 해제(에러가 UI 영구 잠금 방지)
      setCreating(false)
    }
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
      // pane 우클릭 → 자기 메뉴. ★preventDefault+stopPropagation★: 상위 ViewLayoutRenderer 의 제네릭
      //   SlotContextMenu 가 팔레트 pane 위에서 같이 열리지 않게 이벤트를 여기서 멈춘다(AgentList 동형).
      onContextMenu={e => {
        e.preventDefault()
        e.stopPropagation()
        setMenu({ x: e.clientX, y: e.clientY })
      }}
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

      {/* 추가 실패 인라인 힌트 — 색 리터럴 없이 var(--text)(가독) + border-bottom(--border). danger 토큰
          부재라 강조는 텍스트/보더로 대체. 다음 "추가" 시도 시 지워진다. */}
      {addError && (
        <div
          data-preset-add-error="1"
          style={{
            padding: '6px 8px',
            borderBottom: '1px solid var(--border)',
            color: 'var(--text)',
            fontFamily: 'var(--font-ui)',
            fontSize: '11px',
            whiteSpace: 'normal',
            wordBreak: 'break-word',
            flexShrink: 0,
          }}
        >
          {addError}
        </div>
      )}

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

      {/* ── pane 우클릭 메뉴(현재 항목 "추가" 1개 — 후속 사용자 스펙으로 확장) ─────────────── */}
      {menu && (
        <div ref={menuRef} style={MENU_STYLE(menu.x, menu.y)}>
          <div
            data-preset-menu-add="1"
            style={MENU_ITEM_STYLE(creating)}
            onMouseEnter={e => {
              if (!creating) e.currentTarget.style.background = 'color-mix(in srgb, var(--accent) 20%, transparent)'
            }}
            onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
            onClick={e => {
              e.stopPropagation()
              if (creating) return // in-flight 중엔 메뉴는 열려 있으되 재발화 안 함(ref 가드가 최종 방어)
              setMenu(null)
              void addPreset() // 네이티브 폴더 다이얼로그 → createPreset
            }}
          >
            추가
          </div>
        </div>
      )}
    </div>
  )
}

// 메뉴 공통 스타일(SlotContextMenu·AgentList 인라인 메뉴와 동형 — 변수-only).
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
