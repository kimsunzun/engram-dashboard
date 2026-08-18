// ClaudeMascot — JSON 모드 빈 상태에 그리는 픽셀 격자 마스코트(ADR-0145).
//
// ★조정하는 자리는 아래 상수 셋뿐★ — 형태(MASCOT_ROWS) · 색 · 셀 크기. 사용자가 화면을 보고 형태를
//   고치는 자리라 렌더 코드와 분리해 둔다.
//
// 이미지 자산을 쓰지 않는 이유 = 배율에 따라 흐려지고, 문자 블록으로 찍지 않는 이유 = 모노스페이스 셀이
// 정사각형이 아니라 캐릭터가 세로로 늘어난다(ADR-0145 거부한 대안).

/**
 * 마스코트 도트 배치. `█` = 채운 칸, 그 밖의 문자(공백) = 빈 칸.
 * 줄 길이가 서로 달라도 가장 긴 줄에 맞춰 오른쪽을 비우므로, 형태 수정은 이 배열만 고치면 된다.
 *
 * 원본(클로드코드 마스코트 "Clawd") 대조점 — 고칠 때 이 넷을 깨지 말 것:
 * 몸통 좌우 중간 높이에 각각 튀어나온 돌기(한쪽만 두면 팔 한 짝이 빠진 모양이 된다) ·
 * 눈 = 세로로 긴 얇은 막대 둘(굵히면 다른 스티커 변형이 된다) ·
 * 다리 = 짧은 블록 넷(두 쌍, 쌍 사이가 넓게 빈다. 눈은 각 쌍의 가운데 위에 온다) ·
 * 몸통 가로:세로 ≈ 1.5.
 */
const MASCOT_ROWS = [
  '   ████████████████████   ',
  '   ████████████████████   ',
  '   ████████████████████   ',
  '   ████ ██████████ ████   ',
  '   ████ ██████████ ████   ',
  '   ████ ██████████ ████   ',
  '   ████ ██████████ ████   ',
  '██████████████████████████',
  '██████████████████████████',
  '██████████████████████████',
  '   ████████████████████   ',
  '   ████████████████████   ',
  '   ████████████████████   ',
  '     ██ ██      ██ ██     ',
  '     ██ ██      ██ ██     ',
  '     ██ ██      ██ ██     ',
  '     ██ ██      ██ ██     ',
]

/** 원본 몸통 색(테라코타빛 살구) 근사 — 테마 강조색을 따라가지 않는다(ADR-0145 사용자 결정). */
const MASCOT_COLOR = '#c97b5c'

/** 셀 한 변(px). 가로·세로 같은 값 = 1:1 정사각 셀. */
const MASCOT_CELL_PX = 6

const FILLED = '█'

const MASCOT_COLS = Math.max(...MASCOT_ROWS.map(row => row.length))

/**
 * 가로로 이어진 칸을 rect 하나로 병합한 목록. 셀마다 노드를 찍으면 격자 칸 수만큼 DOM 이 불어나고,
 * 빈 상태를 띄운 슬롯 수만큼 곱해진다. 데이터가 상수라 모듈 로드 시 한 번만 계산한다.
 */
const MASCOT_RUNS: { x: number; y: number; w: number }[] = MASCOT_ROWS.flatMap((row, y) => {
  const runs: { x: number; y: number; w: number }[] = []
  let x = 0
  while (x < MASCOT_COLS) {
    if (row[x] !== FILLED) {
      x += 1
      continue
    }
    let w = 1
    while (x + w < MASCOT_COLS && row[x + w] === FILLED) w += 1
    runs.push({ x, y, w })
    x += w
  }
  return runs
})

export function ClaudeMascot() {
  const rows = MASCOT_ROWS.length
  return (
    <svg
      data-rich-mascot="1" // 실측 스크립트가 이 속성으로 찾는다.
      aria-hidden
      width={MASCOT_COLS * MASCOT_CELL_PX}
      height={rows * MASCOT_CELL_PX}
      // viewBox = 격자 칸 단위 → 표시 크기는 위 셀 상수만으로 정해지고 도형은 그대로 확대·축소된다.
      viewBox={`0 0 ${MASCOT_COLS} ${rows}`}
      // 칸 경계가 정수 좌표라 안티에일리어싱 이음새(반투명 실선)를 없앤다.
      shapeRendering="crispEdges"
    >
      {MASCOT_RUNS.map(run => (
        <rect
          key={`${run.y}:${run.x}`}
          x={run.x}
          y={run.y}
          width={run.w}
          height={1}
          fill={MASCOT_COLOR}
        />
      ))}
    </svg>
  )
}
