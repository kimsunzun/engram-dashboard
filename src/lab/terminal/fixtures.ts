// Terminal mode mock ANSI fixture — reproduces breakage at narrow widths with color + box-drawing (over 80 columns).
//
// A measured claude welcome capture would be ideal (but an interactive PTY is tricky to capture), but for reproducing breakage a
// controlled synthetic sample is better — the box width is deliberately set wide (>80) so a narrow slot reliably causes line wrapping /
// box breakage. The measured welcome is obtained separately in the main app measurement phase.

const ESC = '\x1b'
const C = {
  reset: `${ESC}[0m`,
  cyan: `${ESC}[1;36m`,
  yellow: `${ESC}[33m`,
  green: `${ESC}[32m`,
  magenta: `${ESC}[35m`,
  dim: `${ESC}[2m`,
}

// It must be >80 so that in a narrow slot the right border is pushed to the next line and breaks.
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
