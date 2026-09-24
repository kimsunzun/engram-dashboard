// SlotErrorBoundary — 슬롯 콘텐츠 하나의 렌더 오류를 그 슬롯 자리에 가둔다.
//
// 경계가 없으면 React 19 는 렌더 중 throw 한 번에 루트를 통째로 내려 창 전체가 빈 화면이 된다. 이 경계는
//   슬롯 래퍼 *안쪽*에 놓여 우클릭 메뉴·포커스는 살아 있다 — 사용자가 그 메뉴로 슬롯을 비우거나 닫아 빠져나온다.

import { Component, type ErrorInfo, type ReactNode } from 'react'

import { t } from '../../i18n'

interface SlotErrorBoundaryProps {
  slotId: string
  /**
   * 바뀌면 대체 표시를 걷고 자식을 다시 그린다(슬롯 정체·콘텐츠·렌더 모드가 바뀐 경우). ★key 로 대신하지
   * 않는다★ — key 면 에이전트 재배정마다 슬롯 컴포넌트가 재마운트되는데, 오늘은 같은 인스턴스가 새 agentId
   * prop 으로 재구독한다. 오류가 없을 때의 동작을 바꾸지 않으려고 값 비교로 푼다.
   */
  resetKey: string
  children: ReactNode
}

interface SlotErrorBoundaryState {
  failed: boolean
  resetKey: string
}

export class SlotErrorBoundary extends Component<SlotErrorBoundaryProps, SlotErrorBoundaryState> {
  state: SlotErrorBoundaryState = { failed: false, resetKey: this.props.resetKey }

  static getDerivedStateFromProps(
    props: SlotErrorBoundaryProps,
    state: SlotErrorBoundaryState,
  ): Partial<SlotErrorBoundaryState> | null {
    return props.resetKey !== state.resetKey ? { failed: false, resetKey: props.resetKey } : null
  }

  static getDerivedStateFromError(): Partial<SlotErrorBoundaryState> {
    return { failed: true }
  }

  componentDidCatch(error: unknown, info: ErrorInfo): void {
    console.error(`[SlotErrorBoundary] slot ${this.props.slotId} render failed`, error, info.componentStack)
  }

  render(): ReactNode {
    if (!this.state.failed) return this.props.children
    return (
      <div
        // 관측 표면(cdp·테스트)이 깨진 슬롯을 DOM 으로 셀 수 있게.
        data-slot-error="1"
        role="alert"
        className="flex h-full w-full items-center justify-center p-2 text-center text-xs text-muted"
        style={{ fontFamily: 'var(--font-ui)' }}
      >
        {t('slot.renderFailed')}
      </div>
    )
  }
}
