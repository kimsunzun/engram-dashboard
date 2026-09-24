// LoadingPanel — "아직 오는 중" 을 아이콘 하나로 그리는 공용 패널(ADR-0226).
//
// ★보이는 텍스트는 기본으로 없다★: 이름은 화면 밖(aria-label = common.loading)으로만 싣는다. 보이는 문구가
//   필요한 자리는 `label` 로 넘긴다 — 둘은 별개라 `label` 을 넘겨도 aria 이름은 그대로다.
// ★자리·크기·배경은 부르는 쪽이 `className` 으로 정한다★ — 이 패널은 자기 영역 가운데에 아이콘을 놓을 뿐이다.

import type { ReactNode } from 'react'
import { LoaderCircle } from 'lucide-react'

import { cn } from '@/lib/utils'
import { t } from '../../i18n'
import './loading-panel.css' // 회전 keyframes 의 정의처(reduced-motion·e-ink 에서도 늘 돈다).

interface LoadingPanelProps {
  label?: ReactNode
  className?: string
}

// React 가 아무것도 안 그리는 값(null·undefined·boolean·'')에 빈 텍스트 칸을 세우지 않는다.
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
