# 세션 핸드오프 / continue 재개 설계 — 리서치 보고서

**상태:** 완료 (medium, cross-family Claude+Codex)
**날짜:** 2026-06-27
**방법:** Claude(Sonnet) 3갈래 BLIND 팬아웃 + Codex BLIND 독립 교차 → opus 교차 대조
**확신도 범례:** 확실 / 가능성 높음 / 불확실

---

## 질문

멀티 세션 병행(여러 패널이 **같은 repo·같은 cwd**에서 각자 핸드오프 생성·소비) 환경에서:
1. 재개 대상 핸드오프를 **매번 묻지 않고** 정확히 자동 선택하는 법
2. 핸드오프 작성 위치·파일명·내용·라이프사이클
3. 쓰기·읽기 경로 단일 출처(SSOT)

---

## 핵심 수렴 결론 (Claude ↔ Codex 일치 — 확실)

**같은 cwd·repo의 병행 세션은 cwd·mtime으로 자동 구분 불가.** 조사한 모든 툴(Claude Code `--continue`, Codex CLI `--last`, Aider, OpenHands `--last`)이 "최신(mtime) 1개"를 잡아 **같은 cwd 병행 시 엉뚱한 세션을 집는다.** 이건 특정 툴 결함이 아니라 **공통 한계**다 — 추가 판별자(이름/ID) 없이는 논리적으로 불가능.

**해법 = 세션/패널 이름을 stable key로.** 두 family가 독립적으로 같은 결론:
- **Zellij**(최강 예시, 확실): 세션 이름 = 캐시 디렉토리명 = 1:1 식별. `attach <name>`이 안 묻고 복원. cwd 무관.
- GNU screen·abduco: 이름 소켓으로 명시 특정, 모호하면 자동선택 거부(확실).
- Codex 독립 추천(가능성 높음): **panel-name keyed 핸드오프 파일**(`.claude/history/dashboard-main.md`) + 선택적 `index.json`{panel, sessionId, updatedAt, status}. 쓰기·읽기 둘 다 **공유 resolver**로 같은 경로 도출.
- Claude Code의 공식 멀티세션 해법도 **git worktree(cwd 분리)** 또는 **명시 이름/uuid** — cwd만으론 안 됨을 공식 인정(확실).

→ **함의:** 파일을 패널 이름으로 키잉하면 `/continue`가 그 한 개만 읽으면 끝. 목록·mtime·질문이 전부 불필요해진다 = "매번 묻기"가 뿌리째 사라짐.

---

## 핸드오프 내용 — 최소 충분 섹션 (확실, 다수 출처 수렴)

한 줄 상태 · 완료한 것 · **다음 첫 액션** · 블로커/미결 결정 · **검증 안 된 항목**(AI 특화 — 검증된 것으로 오인 방지) · 실패한 접근(do-not) · 참조 경로(코드·ADR). engram 현행 핸드오프는 이미 이 구조를 따름.

---

## 쟁점 (두 family 갈림 — 사용자 판정 필요)

### 쟁점 A — 정체성 출처: 새 세션이 "내가 dashboard2"임을 어떻게 아나
조사는 **이름 키잉이 옳다**까지만 확정. *이름이 어디서 오나*는 환경별 — Codex는 "panelNameSource = 프로젝트 바인딩 상수"로 위임. engram 선택지:
- 사용자가 `/continue <panel>` 직접 입력 (무인프라·확실, 매번 타이핑)
- 패널 시작 시 정체성을 env/마커에 박음 → 자동 (무타이핑, orchestra가 패널명 아니 가능성)

### 쟁점 B — 파일 모델 (Codex vs 현행 갈림)
- **패널당 활성 1파일**(`<panel>.md`, 덮어쓰기+이전본 archive) — Codex 추천. 직접 lookup, mtime 불필요, 모호성 0. 인라인 히스토리 손실(archive로 완화).
- **append-only 날짜+슬러그+패널태그**(현행 근접) — 패널 매칭 후 최신. 히스토리 보존, 대신 select 로직 필요.

---

## 출처
- Zellij Session Resurrection — https://zellij.dev/documentation/session-resurrection.html (확실)
- Claude Code CLI reference — https://code.claude.com/docs/en/cli-reference (확실)
- Codex CLI reference — https://developers.openai.com/codex/cli/reference/ (확실)
- OpenHands resume — https://docs.openhands.dev/openhands/usage/cli/resume (확실)
- tmux-resurrect/continuum — https://github.com/tmux-plugins/tmux-resurrect (확실)
- GNU screen man — https://man7.org/linux/man-pages/man1/screen.1.html (확실)
- 핸드오프 효율 근거 — https://arxiv.org/abs/2606.02875 (가능성 높음)

## 공백 / 미검증
- Cursor·Continue.dev의 공식 세션 재개 식별 메커니즘 미확인(둘 다 불확실).
- 멀티패널 동시 핸드오프 **네이밍 충돌 방지 표준**은 공개 사례 없음 — 패널-키잉이 사실상의 해법이나 표준 문서는 부재.
