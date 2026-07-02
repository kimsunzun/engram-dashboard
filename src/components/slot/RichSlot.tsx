// ★★★ M0 스파이크(임시) — ADR-0044 JSON 모드 배선 ★★★
//
// 랩(`src/lab/richslot/`)의 구조화 렌더 프로토타입(stream-json → 5레이아웃)을 실제 대시보드 캔버스
// 슬롯 안에 띄우는 최소 배선. ★fixture 로 구동★ — 살아있는 에이전트/데몬 없이, 캡처한 stream-json
// 샘플을 파싱해 그린다(TerminalSlot 이 실 PTY 를 그리는 것과 대비). M2 에서 StdioTransport 실스트림으로
// 교체될 자리라, 지금은 "이 렌더가 실제 슬롯 안에서 보이나"를 눈으로 확인하는 용도다.
//
// ★랩 코드는 복붙하지 않고 import★(ADR-0012 모듈 격리) — 파서·레이아웃·RenderSettingsProvider·CSS 를
// 그대로 재사용한다. 여기 로직은 "어떤 fixture 를 어떤 layout 으로 그릴지" 고르는 슬롯 내 툴바뿐.
//
// ★소환 방법★: 사람은 슬롯 empty 플레이스홀더의 "JSON 스파이크" 버튼, LLM/cdp 는 window.__richslot
// (eventBus.ts) 로 이 콘텐츠를 슬롯에 붙인다(§5 — 정식 제어 표면 전 임시 경로).

import { useState } from 'react'

import { parseStreamJson } from '../../lab/richslot/parse'
import { LAYOUTS, type LayoutKey } from '../../lab/richslot/layouts'
import {
  RenderSettingsProvider,
  type CodeRender,
  type DiffRender,
} from '../../lab/richslot/renderSettings'
import showcaseFixture from '../../lab/richslot/fixtures/showcase.jsonl?raw'
import textFixture from '../../lab/richslot/fixtures/text.jsonl?raw'
import toolFixture from '../../lab/richslot/fixtures/tool.jsonl?raw'
import codeFixture from '../../lab/richslot/fixtures/code.jsonl?raw'
import reviewFixture from '../../lab/richslot/fixtures/review.jsonl?raw'

// 실측 stream-json 캡처(랩과 동일 Vite raw import). showcase = kitchen-sink(모든 블록 종류 1개씩).
const FIXTURES: Record<string, string> = {
  showcase: showcaseFixture,
  text: textFixture,
  tool: toolFixture,
  code: codeFixture,
  review: reviewFixture,
}

const TOOLBAR: React.CSSProperties = {
  flex: '0 0 auto',
  display: 'flex',
  gap: 10,
  alignItems: 'center',
  flexWrap: 'wrap',
  padding: '4px 8px',
  background: '#111',
  color: '#ccc',
  fontFamily: 'system-ui, sans-serif',
  fontSize: 12,
  borderBottom: '1px solid #2a2a2a',
}
const SELECT: React.CSSProperties = { fontFamily: 'inherit', fontSize: 12 }
const DIM: React.CSSProperties = { color: '#888' }

export default function RichSlot() {
  const [fixture, setFixture] = useState('showcase')
  const [layout, setLayout] = useState<LayoutKey>('chat') // 기본 = 대화형(가독 결과)
  // 코드/diff 렌더 — 기본은 가벼운 자체 렌더. 'monaco' 로 켜야 무거운 Monaco 청크가 lazy 로드된다.
  const [codeRender, setCodeRender] = useState<CodeRender>('plain')
  const [diffRender, setDiffRender] = useState<DiffRender>('inline')

  const LayoutComp = LAYOUTS[layout].Comp
  const messages = parseStreamJson(FIXTURES[fixture]) // 통짜 파싱(라이브 아님) — layout 변경 시 재파싱

  return (
    <div
      style={{
        width: '100%',
        height: '100%',
        display: 'flex',
        flexDirection: 'column',
        boxSizing: 'border-box',
        background: 'var(--lay-bg)', // chat 레이아웃은 배경이 없어 랩 다크 톤(#0a0a0a)을 슬롯이 깐다
      }}
      data-rich-spike="1" // cdp eval 에서 RichSlot 마운트 여부 확인용
    >
      <div style={TOOLBAR}>
        <span style={{ color: '#6aa0ff', fontWeight: 700 }}>JSON 스파이크</span>
        <span style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
          <span style={DIM}>fixture:</span>
          <select value={fixture} onChange={e => setFixture(e.target.value)} style={SELECT}>
            {Object.keys(FIXTURES).map(k => (
              <option key={k} value={k}>
                {k}
              </option>
            ))}
          </select>
        </span>
        <span style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
          <span style={DIM}>layout:</span>
          <select
            value={layout}
            onChange={e => setLayout(e.target.value as LayoutKey)}
            style={SELECT}
          >
            {(Object.keys(LAYOUTS) as LayoutKey[]).map(k => (
              <option key={k} value={k}>
                {LAYOUTS[k].label}
              </option>
            ))}
          </select>
        </span>
        <span style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
          <span style={DIM}>code:</span>
          <select
            value={codeRender}
            onChange={e => setCodeRender(e.target.value as CodeRender)}
            style={SELECT}
          >
            <option value="plain">plain</option>
            <option value="monaco">monaco</option>
          </select>
        </span>
        <span style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
          <span style={DIM}>diff:</span>
          <select
            value={diffRender}
            onChange={e => setDiffRender(e.target.value as DiffRender)}
            style={SELECT}
          >
            <option value="inline">inline</option>
            <option value="monaco">monaco</option>
          </select>
        </span>
      </div>

      {/* 선택한 layout 이 선택한 fixture 를 렌더(스크롤). 레이아웃 컴포넌트가 자체 overflow-y 를 가짐. */}
      <div style={{ flex: 1, minHeight: 0, overflowY: 'auto' }}>
        <RenderSettingsProvider value={{ codeRender, diffRender }}>
          <LayoutComp messages={messages} />
        </RenderSettingsProvider>
      </div>
    </div>
  )
}
