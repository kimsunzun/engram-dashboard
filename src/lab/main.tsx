// 출력 실험실(lab) 진입점 — RichSlot 렌더 실험.
// JSON 으로 표현 가능한 모든 포맷(텍스트·마크다운·코드·diff·thinking·각 도구·에러)을 한 개씩 담은
// 쇼케이스 샘플을 한 화면에 길게 깔고(스크롤), 모드(layout/preset/code/diff)를 바꾸면 통째로 다시 그려진다.
// Tauri/데몬 없이 순수 브라우저.
//
// 툴바는 탭으로 카테고리 분리(랩이 커질 대비): 입력(샘플) · 표현(preset/layout/code/diff).
// 새 실험 영역은 TABS 에 한 줄 + 아래 컨텐츠 블록만 추가하면 된다.

import { createRoot } from 'react-dom/client'
import { useState, useRef, useEffect } from 'react'
import { parseStreamJson } from './richslot/fixtureParse'
import { LAYOUTS, type LayoutKey } from './richslot/layouts'
import { PRESETS, applyPreset } from './richslot/presets'
import {
  RenderSettingsProvider,
  type CodeRender,
  type DiffRender,
} from './richslot/renderSettings'
import showcaseFixture from './richslot/fixtures/showcase.jsonl?raw'
import textFixture from './richslot/fixtures/text.jsonl?raw'
import toolFixture from './richslot/fixtures/tool.jsonl?raw'
import partialFixture from './richslot/fixtures/partial.jsonl?raw'
import codeFixture from './richslot/fixtures/code.jsonl?raw'
import reviewFixture from './richslot/fixtures/review.jsonl?raw'

// 샘플 = 실제 claude 세션을 stream-json 으로 캡처/합성한 입력. 살아있는 에이전트 없이 렌더 실험용.
// showcase = JSON 표현 가능한 포맷을 한 개씩 모은 kitchen-sink(모드 비교의 기준 샘플).
const SAMPLES: Record<string, string> = {
  showcase: showcaseFixture, // 모든 포맷 한 개씩(thinking·md·code·diff·도구·에러)
  text: textFixture, // 텍스트 응답만
  tool: toolFixture, // Read 하나
  partial: partialFixture, // --include-partial-messages 델타
  code: codeFixture, // 코드 수정 + git diff
  review: reviewFixture, // 긴 마크다운 코드리뷰
}

const SEP: React.CSSProperties = { width: 1, alignSelf: 'stretch', background: '#333', margin: '0 4px' }
const SELECT: React.CSSProperties = { fontFamily: 'inherit', fontSize: 13 }
const SUBLABEL: React.CSSProperties = { color: '#888' }

// 탭 카테고리 — 새 실험 영역이 생기면 여기 한 줄 + 아래 컨텐츠 블록만 추가(랩 확장 쉽게).
const TABS = [
  { key: 'input', label: '입력' },
  { key: 'present', label: '표현' },
] as const
type TabKey = (typeof TABS)[number]['key']

function Lab() {
  const [sample, setSample] = useState('showcase') // 대화형류 layout 일 때 그릴 데이터
  const [layout, setLayout] = useState<LayoutKey>('catalog') // 스타일 카탈로그 기본 — 각 스타일 견본을 라벨로 구분
  const [preset, setPreset] = useState('color-chat')
  // 코드/diff 렌더 — 기본 가벼운 자체 렌더. 'monaco' 로 켜면 VS Code 스타일(무거우니 opt-in).
  const [codeRender, setCodeRender] = useState<CodeRender>('plain')
  const [diffRender, setDiffRender] = useState<DiffRender>('inline')
  const [tab, setTab] = useState<TabKey>('present') // 툴바 활성 카테고리

  // applyPreset 가 data-theme·CSS var 를 박을 대상(레이아웃 컨테이너). 안의 layout 이 상속.
  const viewRef = useRef<HTMLDivElement>(null)

  // preset 선택 → 뷰에 색/폰트/모드 적용 + 반환된 layout key 로 활성 layout 동기화.
  // 사람 클릭과 LLM(window.__lab.applyPreset) 이 같은 경로(§5).
  function selectPreset(name: string) {
    if (!viewRef.current) return
    const lk = applyPreset(viewRef.current, name)
    if (lk) {
      setPreset(name)
      setLayout(lk)
    }
  }

  useEffect(() => {
    if (viewRef.current) applyPreset(viewRef.current, preset)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // ★§5 LLM-제어 표면★: command/CDP 가 표현/샘플을 바꾸는 임시 핸들(정식 control surface 전).
  useEffect(() => {
    ;(window as unknown as { __lab: unknown }).__lab = {
      applyPreset: (name: string) => selectPreset(name),
      setLayout: (v: LayoutKey) => LAYOUTS[v] && setLayout(v),
      setCodeRender: (v: CodeRender) => setCodeRender(v),
      setDiffRender: (v: DiffRender) => setDiffRender(v),
      setSample: (v: string) => SAMPLES[v] && setSample(v),
      PRESETS,
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // 탭 버튼 — 활성 탭 강조.
  const tabBtn = (active: boolean): React.CSSProperties => ({
    fontWeight: active ? 700 : 400,
    color: active ? '#fff' : '#9aa',
    background: active ? '#2a3a5a' : 'transparent',
    border: `1px solid ${active ? '#4a6a9a' : '#333'}`,
    borderRadius: 4,
    padding: '2px 12px',
    cursor: 'pointer',
  })

  const LayoutComp = LAYOUTS[layout].Comp
  const messages = parseStreamJson(SAMPLES[sample])

  return (
    <div style={{ height: '100vh', display: 'flex', flexDirection: 'column' }}>
      <div
        style={{
          padding: 8,
          background: '#111',
          color: '#ccc',
          display: 'flex',
          gap: 12,
          alignItems: 'center',
          flexWrap: 'wrap',
          fontFamily: 'system-ui, sans-serif',
          fontSize: 13,
          flex: '0 0 auto',
        }}
      >
        {/* 탭 바 — 카테고리 분리(활성 탭 컨트롤만 노출). 랩 확장 시 TABS 에 추가. */}
        <span style={{ display: 'flex', gap: 4 }}>
          {TABS.map((t) => (
            <button key={t.key} onClick={() => setTab(t.key)} style={tabBtn(tab === t.key)}>
              {t.label}
            </button>
          ))}
        </span>
        <span style={SEP} />

        {/* ── 입력(Source): 무엇을 그리나 ── */}
        {tab === 'input' &&
          (layout === 'catalog' ? (
            // 카탈로그는 고정 견본이라 샘플 무관 — 대화형류 layout 에서만 샘플이 의미 있음.
            <span style={SUBLABEL}>스타일 카탈로그는 고정 견본을 보여줍니다 (샘플 무관 · layout 을 대화형 등으로 바꾸면 샘플 적용).</span>
          ) : (
            <span style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
              <span style={SUBLABEL}>샘플:</span>
              <select value={sample} onChange={(e) => setSample(e.target.value)} style={SELECT}>
                {Object.keys(SAMPLES).map((k) => (
                  <option key={k} value={k}>
                    {k}
                  </option>
                ))}
              </select>
            </span>
          ))}

        {/* ── 표현(Presentation): 어떻게 그리나 (모드 — 바꾸면 같은 샘플이 통째로 다시 그려짐) ── */}
        {tab === 'present' && (
          <>
            <span style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
              <span style={SUBLABEL}>preset:</span>
              <select value={preset} onChange={(e) => selectPreset(e.target.value)} style={SELECT}>
                {Object.keys(PRESETS).map((k) => (
                  <option key={k} value={k}>
                    {k}
                  </option>
                ))}
              </select>
            </span>
            <span style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
              <span style={SUBLABEL}>layout:</span>
              <select
                value={layout}
                onChange={(e) => setLayout(e.target.value as LayoutKey)}
                style={SELECT}
              >
                {(Object.keys(LAYOUTS) as LayoutKey[]).map((k) => (
                  <option key={k} value={k}>
                    {LAYOUTS[k].label}
                  </option>
                ))}
              </select>
            </span>
            <span style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
              <span style={SUBLABEL}>code:</span>
              <select
                value={codeRender}
                onChange={(e) => setCodeRender(e.target.value as CodeRender)}
                style={SELECT}
              >
                <option value="plain">plain</option>
                <option value="monaco">monaco</option>
              </select>
            </span>
            <span style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
              <span style={SUBLABEL}>diff:</span>
              <select
                value={diffRender}
                onChange={(e) => setDiffRender(e.target.value as DiffRender)}
                style={SELECT}
              >
                <option value="inline">inline</option>
                <option value="monaco">monaco</option>
              </select>
            </span>
          </>
        )}
      </div>

      {/* 단일 뷰 — 선택한 layout 이 선택한 샘플을 렌더(스크롤). 모드 변경 시 이 안이 통째로 다시 그려짐. */}
      <div ref={viewRef} style={{ flex: 1, overflowY: 'auto' }}>
        <RenderSettingsProvider value={{ codeRender, diffRender }}>
          <LayoutComp messages={messages} />
        </RenderSettingsProvider>
      </div>
    </div>
  )
}

createRoot(document.getElementById('root')!).render(<Lab />)
