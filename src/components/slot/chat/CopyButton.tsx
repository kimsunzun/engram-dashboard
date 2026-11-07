// Minimize dependencies: only a plain <button> + lucide icons, with no ui/button·cva·radix (our own implementation).
import { Check, Copy } from 'lucide-react'
import { useCallback, useState } from 'react'

import { t } from '../../../i18n'

const COPIED_MS = 1500

interface CopyButtonProps {
  /** Lazy computation — a code block's textContent is known only after render. */
  getText: () => string | null | undefined
  label?: string
}

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
