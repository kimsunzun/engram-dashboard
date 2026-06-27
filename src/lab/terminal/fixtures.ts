// 터미널 모드 mock ANSI fixture — 컬러 + box-drawing(80칼럼 초과) 으로 좁은 폭 깨짐 재현.
//
// 실측 claude welcome 캡처가 이상적이나(interactive PTY 라 캡처 까다로움), 깨짐 재현에는
// 통제된 합성 샘플이 더 낫다 — 박스 폭을 의도적으로 넓게(>80) 잡아 좁은 슬롯에서 줄바꿈/
// 박스 깨짐을 확실히 유발한다. 실측 welcome 은 메인 앱 실측 단계에서 별도 확보.

const ESC = '\x1b'
const C = {
  reset: `${ESC}[0m`,
  cyan: `${ESC}[1;36m`,
  yellow: `${ESC}[33m`,
  green: `${ESC}[32m`,
  magenta: `${ESC}[35m`,
  dim: `${ESC}[2m`,
}

// 박스 내부 폭 86칸(>80) — 좁은 슬롯에서 우측 테두리가 다음 줄로 밀려 깨진다.
const W = 86
const bar = '─'.repeat(W)
const pad = (s: string, visibleLen: number) => s + ' '.repeat(Math.max(0, W - visibleLen))

export const ansiWelcomeSample =
  `${C.cyan}╭${bar}╮${C.reset}\r\n` +
  `${C.cyan}│${C.reset}${pad(`  ${C.yellow}Claude Code${C.reset}  ${C.dim}v2.1 — RichSlot Lab terminal fixture${C.reset}`, 2 + 11 + 2 + 36)}${C.cyan}│${C.reset}\r\n` +
  `${C.cyan}│${C.reset}${pad('', 0)}${C.cyan}│${C.reset}\r\n` +
  `${C.cyan}│${C.reset}${pad(`  ${C.green}✓${C.reset} 컬러 렌더 확인용 라인 (green check)`, 2 + 1 + 30)}${C.cyan}│${C.reset}\r\n` +
  `${C.cyan}│${C.reset}${pad(`  ${C.magenta}●${C.reset} box-drawing 폭 ${W}칸 — 좁은 슬롯에서 깨짐 재현`, 2 + 1 + 36)}${C.cyan}│${C.reset}\r\n` +
  `${C.cyan}╰${bar}╯${C.reset}\r\n` +
  `\r\n` +
  `${C.dim}$ ${C.reset}echo "리사이즈 시 fit→onResize 전파 + reflow 확인"\r\n` +
  `리사이즈 시 fit→onResize 전파 + reflow 확인\r\n`
