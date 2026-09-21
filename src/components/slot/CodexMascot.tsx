// CodexMascot — JSON 모드 빈 상태에 그리는 codex 쪽 표식. ClaudeMascot 의 형제이며 렌더 기법(도트 격자 →
//   가로 run 병합 → viewBox 가 칸 단위)을 그대로 따른다.
//
// ★ClaudeMascot 을 고쳐 쓰지 않고 파일을 나눈 이유★: 그쪽은 호출처가 하나뿐이라 한 줄만 손대도 claude
//   패널이 함께 바뀐다. 두 표식은 서로 다른 제품을 가리키므로 형태·색을 각자 소유한다.
//
// ★벤더 로고를 베끼지 않는다★: 상표를 그대로 찍는 대신 프롬프트 기호(`>_`)를 쓴다. 선택 기준은 ADR-0062 의
//   「모양이 1차 신호」 — Clawd 의 덩어리 실루엣과 겹치지 않아 e-ink(색 무력화)에서도 갈린다.
//
// ★격자 크기는 ClaudeMascot 과 같은 26×17 이다★ — 빈 상태 묶음이 백엔드에 따라 위아래로 움직이지 않게
//   자리(footprint)를 맞춘다. 형태만 다르고 칸 크기·정렬 규칙은 형제와 동일하다.

/**
 * 표식 도트 배치. `█` = 채운 칸, 그 밖의 문자(공백) = 빈 칸.
 *
 * 형태 = 프롬프트 `>_`. 고칠 때 이 둘을 깨지 말 것:
 * 꺾쇠는 위아래가 대칭이고 꼭짓점이 오른쪽 하나(둘이 되면 화살표가 아니라 마름모로 읽힌다) ·
 * 밑줄은 꺾쇠 아래꼬리와 같은 줄에서 시작해 오른쪽으로 뻗는다(위로 올리면 등호처럼 보인다).
 */
const MASCOT_ROWS = [
  '   ███                    ',
  '    ███                   ',
  '     ███                  ',
  '      ███                 ',
  '       ███                ',
  '        ███               ',
  '         ███              ',
  '          ███             ',
  '           ███            ',
  '          ███             ',
  '         ███              ',
  '        ███               ',
  '       ███                ',
  '      ███                 ',
  '     ███       ██████████ ',
  '    ███        ██████████ ',
  '   ███         ██████████ ',
]

/**
 * codex 표식 색(청록) — 테마 강조색을 따라가지 않는다(형제 ClaudeMascot 과 같은 규율).
 * ★`richBranding.css` 의 `--rich-brand` 와 같은 값이다★ — 표식과 패널 색조가 한 색에서 나온다.
 */
const MASCOT_COLOR = '#10a37f'

/** 셀 한 변(px). 형제와 같은 값 = 같은 표시 크기. */
const MASCOT_CELL_PX = 6

const FILLED = '█'

const MASCOT_COLS = Math.max(...MASCOT_ROWS.map(row => row.length))

/** 가로로 이어진 칸을 rect 하나로 병합한 목록(형제와 같은 이유 — 셀마다 노드를 찍으면 DOM 이 불어난다). */
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

export function CodexMascot() {
  const rows = MASCOT_ROWS.length
  return (
    <svg
      data-rich-mascot="1" // 형제와 같은 표면 — 「빈 상태 표식이 떴나」는 백엔드와 무관하게 이 속성으로 본다.
      data-rich-brand="codex" // 어느 백엔드인가(안정 토큰 — richBranding.ts 가 정의처).
      aria-hidden
      width={MASCOT_COLS * MASCOT_CELL_PX}
      height={rows * MASCOT_CELL_PX}
      viewBox={`0 0 ${MASCOT_COLS} ${rows}`}
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
