// LoadingPanel — a shared panel that draws "still arriving" as a single icon (ADR-0226).
//
// ★There is no visible text by default★: the name is carried only off-screen (aria-label = common.loading). Places that
//   need visible text pass `label` — the two are separate, so passing `label` leaves the aria name as is.
// ★Position · size · background are decided by the caller through `className`★ — this panel just puts the icon in the middle of its own area.

import type { ReactNode } from 'react'
import { LoaderCircle } from 'lucide-react'

import { cn } from '@/lib/utils'
import { t } from '../../i18n'
import './loading-panel.css' // Where the rotation keyframes are defined (always spins, even under reduced-motion · e-ink).

interface LoadingPanelProps {
  label?: ReactNode
  className?: string
}

// Do not stand up an empty text field for values React draws nothing for (null · undefined · boolean · '').
function hasLabel(label: ReactNode): boolean {
  return label != null && typeof label !== 'boolean' && label !== ''
}

export function LoadingPanel({ label, className }: LoadingPanelProps) {
  return (
    <div
      role="status"
      aria-busy="true"
      aria-label={t('common.loading')}
      data-loading-panel="1"
      className={cn('flex flex-col items-center justify-center gap-2 text-muted', className)}
    >
      <LoaderCircle aria-hidden className="engram-loading-spinner size-8" />
      {hasLabel(label) && <div className="text-[13px]">{label}</div>}
    </div>
  )
}
