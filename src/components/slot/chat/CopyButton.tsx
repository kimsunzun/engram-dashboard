// 코드블록 복사 버튼 — Markdown 의 <pre> 렌더가 hover-reveal 로 얹는다.
//   의존 최소화: ui/button·cva·radix 없이 순수 <button> + lucide 아이콘만 쓴다(우리 자체 구현).
//   복사 후 1.5초간 Check 아이콘으로 바꿔 시각 피드백을 준다.
import { Check, Copy } from 'lucide-react'
import { useCallback, useState } from 'react'

import { t } from '../../../i18n'

const COPIED_MS = 1500

interface CopyButtonProps {
  /** 복사할 텍스트를 지연 계산(코드블록 textContent 는 렌더 후에야 안다). */
  getText: () => string | null | undefined
  label?: string
}

/** hover 시 나타나는 우상단 복사 버튼. group-hover 로 부모(.group)에서 노출을 제어한다. */
export function CopyButton({ getText, label = t('common.copy') }: CopyButtonProps) {
  const [copied, setCopied] = useState(false)

  const onClick = useCallback(() => {
    const text = getText()
    if (!text) return
    navigator.clipboard
      .writeText(text)
      .then(() => {
        setCopied(true)
        setTimeout(() => setCopied(false), COPIED_MS)
      })
      .catch((err) => console.warn('[chat/CopyButton] copy failed', err))
  }, [getText])

  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={copied ? t('common.copied') : label}
      className="absolute top-1.5 right-1.5 opacity-0 group-hover:opacity-100 transition-opacity rounded p-1 text-muted hover:text-foreground hover:bg-surface"
    >
      {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
    </button>
  )
}
