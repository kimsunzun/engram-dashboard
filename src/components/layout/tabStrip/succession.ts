// TRD S21-slot-docking §3-3 · §4-2: 셸 `layout::manager::succession` 의 거울 — 탭 닫기 · 놓기에서 셸이 고르는 다음 활성과,
//   끄는 동안 출처 창이 보이는 남은 탭이 같은 탭이어야 놓은 뒤 화면이 바뀌지 않는다. 규칙을 고치면 셸 판과 함께 고친다
//   (같은 표 시험이 양쪽에 있다 — 셸 `succession_gives_the_same_tab_as_closing`).

/** 탭 목록 `tabs` 에서 `removed` 가 빠질 때 활성을 이을 탭 — 오른쪽 이웃 우선, 없으면 왼쪽. `removed` 가 목록에 없거나 혼자면 `null`. */
export function succession(tabs: readonly string[], removed: string): string | null {
  const pos = tabs.indexOf(removed)
  if (pos < 0) return null
  return tabs[pos + 1] ?? tabs[pos - 1] ?? null
}
