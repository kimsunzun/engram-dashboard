// 알림을 레이아웃 위에 덮는 층 — 레이아웃을 밀지 않는다. 고정된 배치에서 일하므로 알림이 뜰 때 그것이 밀리면 안 되고,
// TabBar 를 가려도 된다 — 알림은 ✕ 로 닫는다. 여럿이면 쌓고 넘치면 스크롤한다(사용자 결정 2026-10-06 · ADR-0276 ·
// 연결 띠 = ADR-0277).
// 이 층에 새 알림을 얹으려면 결정이 따로 필요하다 — ADR-0180 · ADR-0277.
// ★자리 기준은 호출부가 깐다★ — 이 층은 가장 가까운 positioned 조상의 맨 위에 붙는다. 호출부는 positioned 조상을
//   대고, 그 조상의 크기는 이 층이 바꾸지 않는다(층은 흐름 밖이다).

import type { ReactNode } from 'react'

import { ScrollArea } from '../ui/scroll-area'

/**
 * `children` = 쌓을 알림들, 위에서부터 그 차례로. 알림이 하나도 안 그려지면(전부 `null`) 층의 높이는 0 이다.
 *
 * ★알림 줄은 바탕이 불투명해야 한다★ — 층은 줄 사이 틈을 그리려고 자기 바탕을 깐다(아래 `notice-stack` 의 주석).
 * 투명한 줄은 그 바탕 위에 그려진다.
 *
 * ★알림의 글 칸은 아무 데서나 줄을 바꿔야 한다(`wrap-anywhere`)★ — Radix Viewport 는 children 을
 * `display: table; min-width: 100%` 로 감싼다(`@radix-ui/react-scroll-area` 1.2.11). 끊을 데 없는 긴 낱말(경로)이
 * 표를 창보다 넓히면 가로는 잘려 ✕ 가 화면 밖으로 밀린다(정적 판독 — GUI 로 재지 않았다).
 */
export default function NoticeOverlay({ children }: { children: ReactNode }) {
  return (
    // ★자리는 감싼 div 가 잡는다★ — Radix ScrollArea 의 Root 는 인라인 `position: relative` 를 박아 클래스로 못 덮는다.
    // ★층의 높이는 알림 줄만큼이다★ — 전체 높이의 투명 막을 깔면 줄 밖의 클릭까지 먹는다. 상한 = 창 높이의 1/3(세션
    //   판단 — 쌓여도 배치의 2/3 는 보인다). 그림자는 그려진 줄의 모양을 따르는 `drop-shadow` 라 줄이 없으면 안 그린다.
    // z 40 = 분할선 · 칸 막(z-20) 위 · 복원 모달(z-50 — main 만) · 메뉴(1000 이상) 아래.
    <div
      data-testid="notice-overlay"
      className="drop-shadow-md"
      style={{ position: 'absolute', top: 0, left: 0, right: 0, zIndex: 40 }}
    >
      <ScrollArea viewportClassName="max-h-[33vh]">
        {/* 줄 사이 1px 틈에 앱 바탕을 비춘다 — 다크에서 알림 바탕(`--surface-elevated`)과 테두리(`--border`)가 거의 같아
              겹친 줄의 경계가 안 보인다(세션 판단). */}
        <div data-testid="notice-stack" className="flex flex-col gap-px bg-background">
          {children}
        </div>
      </ScrollArea>
    </div>
  )
}
