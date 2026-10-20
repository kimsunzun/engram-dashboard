# 가장자리에 붙은 펼침 패널 — 어느 쪽으로 펼치나 · 창 끝이면 (관행 요약)

- **상태:** light · 결정 대기(프리셋 PRD 이전 구상 — 편집 칸에서 줄을 누르면 상세 패널이 라이브러리 슬롯 가장자리에 붙어 옆 슬롯 위로 겹쳐 나온다. 슬롯이 창 끝이면?)
- **날짜:** 2026-10-15
- **방법:** research light. 수집자 1(검색 4~6회 · 상당 부분 기억 · 얇은 원문) · 메인 grounding(JetBrains 원문 — 앞 조사 `side-detail-panel-push-vs-overlay-survey-2026-10-15.md` 에서 확인) · 적대 리뷰 없음.

## 결론

1. **가장자리(독)에 붙은 펼침 패널은 「붙은 가장자리의 반대쪽 = 내용 쪽」으로 펼친다. 방향은 독이 어느 가장자리냐로 정해지고, 그때그때 뒤집지 않는다.** Visual Studio 자동 숨김 도구 창(붙은 가장자리의 탭에서 미끄러져 나옴 — 원문) · JetBrains Dock Unpinned(자기 막대 쪽에서 나옴 — 원문 · 덮는지는 미확인) · 포토샵 아이콘 패널(오른쪽 독이면 왼쪽으로 — 기억). **가능성 높음**에 못 미치는 **불확실**(단일 규칙을 적어 둔 출처는 없다).
2. **「선호 방향 → 자리 없으면 반대쪽 → 창 안으로 끼워 넣기」 뒤집기는 기준점에 붙는 팝업(플라이아웃 · 팝오버)의 규칙이다 — 독 패널의 규칙이 아니다.** WinUI `FlyoutPlacementMode.Auto` 는 선호 자리가 모자라면 다른 자리를 자동으로 고른다(원문) · NSPopover 의 반대쪽 → 수직쪽 순서는 **불확실**(Apple 원문 미확인).
3. **창 끝에서 「붙박이 · 칸 안으로 되돌리기」로 물러나는 피어는 못 찾았다.** **공백.**

## Engram 에 옮기면 (후보 — 결정 아님)

- 우리 상세 패널은 슬롯 가장자리에 붙어 **바깥(옆 슬롯 위)** 으로 펼치는 꼴이라 독 패널과 방향이 반대다(옆 슬롯을 덮음 = JetBrains Undock 류).
- 슬롯이 창 끝이면 독 패널 규칙(붙은 쪽의 반대 = 안쪽)대로 **자기 슬롯 안쪽으로 덮어 펼치는** 것이 관행과 어긋나지 않는다 — 단 관행 근거는 얇다.

## grounding

| 클레임 | 출처 | 판정 |
|---|---|---|
| WinUI Auto — 선호 자리 모자라면 다른 자리 자동 | https://learn.microsoft.com/en-us/UWP/api/windows.ui.xaml.controls.primitives.flyoutplacementmode?view=winrt-22621 (수집자) | 미확인(메인 대조 안 함) |
| VS 자동 숨김 — 붙은 가장자리 탭에서 다시 나옴 | https://learn.microsoft.com/en-us/previous-versions/z4y0hsax(v=vs.110) (수집자) | 미확인 |
| JetBrains Dock Unpinned · Undock 문구 | https://www.jetbrains.com/help/idea/viewing-modes.html (앞 조사에서 메인 확인) | 지지 |
| 포토샵 아이콘 패널 방향 · NSPopover 순서 | 기억 · 서드파티 | 불확실 |

## 공백

- 못 본 곳: Blender N/T 패널 · Figma · 언리얼 콘텐츠 서랍.
