// RootErrorBoundary — 슬롯 밖(탭 바·레이아웃 분기·분할 컴포넌트·페이지)의 렌더 오류가 창 전체를 지우지 않게 받는다.
//
// 경계가 없으면 React 19 는 렌더 중 throw 한 번에 루트를 통째로 내려 창이 재시작 전까지 빈 화면이 된다. 슬롯
//   안쪽 오류는 SlotErrorBoundary 가 먼저 가두므로 여기까지 오는 것은 그 밖의 오류뿐이다.
// ★대체 표시는 라우터·스토어·테마 스토어에 기대지 않는다★ — 그중 무엇이 터진 원인일지 모른다. 색은 main.tsx 가
//   첫 페인트 전에 :root 에 박는 CSS 변수만 쓴다.

import { Component, type ErrorInfo, type ReactNode } from 'react'

import { t } from '../../i18n'

interface RootErrorBoundaryProps {
  children: ReactNode
}

interface RootErrorBoundaryState {
  failed: boolean
}

export class RootErrorBoundary extends Component<RootErrorBoundaryProps, RootErrorBoundaryState> {
  state: RootErrorBoundaryState = { failed: false }

  static getDerivedStateFromError(): Partial<RootErrorBoundaryState> {
    return { failed: true }
  }

  componentDidCatch(error: unknown, info: ErrorInfo): void {
    console.error(
      `[RootErrorBoundary] render failed (route ${window.location.hash || '#/'})`,
      error,
      info.componentStack,
    )
  }

  private redraw = (): void => {
    this.setState({ failed: false })
  }

  private reload = (): void => {
    window.location.reload()
  }

  render(): ReactNode {
    if (!this.state.failed) return this.props.children
    return (
      <div
        data-root-error="1"
        role="alert"
        className="flex h-screen w-screen flex-col items-center justify-center gap-3 bg-background p-4 text-center text-sm text-foreground"
        style={{ fontFamily: 'var(--font-ui)' }}
      >
        <div>{t('window.renderFailed')}</div>
        <div className="flex gap-2">
          <button
            type="button"
            onClick={this.redraw}
            className="rounded border border-border bg-surface px-3 py-1 hover:bg-elevated"
          >
            {t('window.redraw')}
          </button>
          <button
            type="button"
            onClick={this.reload}
            className="rounded border border-border bg-surface px-3 py-1 hover:bg-elevated"
          >
            {t('window.reload')}
          </button>
        </div>
      </div>
    )
  }
}
