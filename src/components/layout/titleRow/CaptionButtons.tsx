// TRD S26-window-chrome §7-4: 창 단추 셋(최소화 · 최대화/복원 · 셋째). ★누르면 레지스트리 `window.*` 명령을 부른다 — Tauri 창
//   API 를 부르지 않는다★(AC10 — `src/__tests__/noWindowControl.test.ts`). 사람 누름과 LLM 이 같은 명령을 흔든다(CLAUDE.md
//   「LLM-우선 제어」). 셋째 = `window.close` — main 이면 셸이 모든 창을 숨기고 팝아웃이면 닫는다(TRD §5-4).

import type { CSSProperties, ReactNode } from 'react'
import { Copy, EyeOff, Minus, Square, X } from 'lucide-react'

import { fireAndForget } from '../../../commands/dispatch'
import { t } from '../../../i18n'
import { CAPTION_BUTTON_W } from './metrics'

interface CaptionButtonsProps {
  label: string
  /** main 창인가 — 셋째 단추가 「모든 창 숨기기」(눈 가림 · 옅은 맴돎)이고, 아니면 「창 닫기」(× · 빨간 맴돎)다. */
  main: boolean
  maximized: boolean
}

export default function CaptionButtons({ label, main, maximized }: CaptionButtonsProps) {
  const target = { window: label }
  return (
    <div data-testid="caption-buttons" style={{ display: 'flex', flex: '0 0 auto', alignSelf: 'stretch', borderBottom: '1px solid var(--tab-bar-edge)' }}>
      <CaptionButton testId="caption-minimize" name={t('window.captionMinimize')} onPress={() => fireAndForget('window.minimize', target)}>
        <Minus aria-hidden size={GLYPH} strokeWidth={GLYPH_STROKE} />
      </CaptionButton>
      {maximized ? (
        <CaptionButton testId="caption-maximize" name={t('window.captionRestore')} onPress={() => fireAndForget('window.unmaximize', target)}>
          <Copy aria-hidden size={GLYPH_BOX} strokeWidth={GLYPH_STROKE} />
        </CaptionButton>
      ) : (
        <CaptionButton testId="caption-maximize" name={t('window.captionMaximize')} onPress={() => fireAndForget('window.maximize', target)}>
          <Square aria-hidden size={GLYPH_BOX} strokeWidth={GLYPH_STROKE} />
        </CaptionButton>
      )}
      {main ? (
        <CaptionButton testId="caption-close" name={t('window.captionHideAll')} onPress={() => fireAndForget('window.close', target)}>
          <EyeOff aria-hidden size={GLYPH} strokeWidth={GLYPH_STROKE} />
        </CaptionButton>
      ) : (
        <CaptionButton testId="caption-close" name={t('window.captionClose')} danger onPress={() => fireAndForget('window.close', target)}>
          <X aria-hidden size={GLYPH} strokeWidth={GLYPH_STROKE} />
        </CaptionButton>
      )}
    </div>
  )
}

function CaptionButton({
  testId,
  name,
  danger = false,
  onPress,
  children,
}: {
  testId: string
  name: string
  danger?: boolean
  onPress: () => void
  children: ReactNode
}) {
  return (
    <button
      type="button"
      data-testid={testId}
      aria-label={name}
      title={name}
      onClick={onPress}
      className={danger ? DANGER_CLASS : BUTTON_CLASS}
      style={BUTTON_STYLE}
    >
      {children}
    </button>
  )
}

// 글리프 크기 = 스파이크(2026-10-24)의 값 — Windows 11 글리프와 견주지 않았다(미검 · S11).
const GLYPH = 14
const GLYPH_BOX = 12
const GLYPH_STROKE = 1.5

// ★클래스 이름은 통째 글자로 둔다★ — Tailwind 는 소스 글자를 훑어 클래스를 만들므로 조각을 이어 붙이면 생기지 않는다. 색 ·
//   맴돎 바탕을 인라인 스타일로 두지 말 것 — 인라인이 맴돎 클래스를 이겨 맴돎이 사라진다. 포커스 테두리는 안쪽에 그린다 —
//   오른쪽 끝 단추는 창 가장자리에 붙어 바깥 테두리가 잘린다.
const BUTTON_CLASS =
  'bg-transparent text-[color:var(--caption-fg)] hover:bg-[var(--caption-hover-bg)] focus-visible:outline focus-visible:outline-1 focus-visible:outline-accent focus-visible:-outline-offset-2'
const DANGER_CLASS =
  'bg-transparent text-[color:var(--caption-fg)] hover:bg-[var(--caption-close-hover-bg)] hover:text-[color:var(--caption-close-hover-fg)] focus-visible:outline focus-visible:outline-1 focus-visible:outline-accent focus-visible:-outline-offset-2'

// Windows 캡션 단추는 화살표 커서다.
const BUTTON_STYLE: CSSProperties = {
  width: CAPTION_BUTTON_W,
  padding: 0,
  border: 'none',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  cursor: 'default',
}
