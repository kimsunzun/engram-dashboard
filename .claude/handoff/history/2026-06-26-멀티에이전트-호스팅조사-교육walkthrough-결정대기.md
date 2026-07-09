# 핸드오프 — 멀티에이전트 호스팅·오케스트레이션 리서치 + 교육 walkthrough

> **만든 이유:** 메인 컨텍스트 57%(535k msg) 참 → 신선 세션으로 이어가기.
> **성격:** 리서치 + 사용자 교육 세션. **코드 변경 0, 새 ADR 0(결정 보류).**
> **이어가기:** 아래 "다음 스텝"부터. 풀 디테일은 산출물 doc에 있음 — 여기선 *포인터 + 대화에서만 나온 것*만.

---

## 1. 한 줄 상태
"engram 다중 에이전트 시 메모리 부담(process-per-agent)" 동기로 **호스팅·오케스트레이션 선택지 조사 완료(PRD/컨설 단계) → 사용자 결정 보류.** 이후 결과를 **교육식 walkthrough**로 함께 정독 중(A·B·C 다 돔, 오케스트라 5축만 미반영).

## 2. 산출물 (★ 먼저 읽을 것)
- **`docs/research/multi-agent-hosting-orchestration-research-2026-06-22.md`** — 풀 리서치. Stage 1(원자료 A/B/C1/C2/C3 + 호스팅 매트릭스)·1.5(전제 정정)·2(융합 선택지 0/1/1b/2/3)·3(2인 리뷰) 전부 들어있음. 출처·확신도 표기됨.
- 같은 줄기: `docs/research/control-surface-and-fleet.md`(이전 fleet 조사), `tracking.md` T-9(claude 풀링).

## 3. 핵심 결론 (데이터 — gist만, 상세는 doc)
- **process-per-agent는 CLI substrate 한정 강제.** 이미 뜬 CLI 프로세스 둘은 사후 병합 불가(OS 사실). 메모리 이득은 **API backend in-process(async task/tokio)** 에서만.
- **용어 정정(혼선 컸음):** "session"이 둘을 뭉갬 → **Agent Teams = 다중세션·다중프로세스**(engram 현 모델과 동류, 절감 대상 아님) / **Subagents = 단일세션**.
- **호스팅(어디서 도나) ⊥ 오케스트라(어떻게 엮나)** — 직교 축. 메모리는 호스팅 축.
- LLM은 프로세스 안에 없음(원격) → 에이전트 = "원격 모델 호출 루프". 로컬 비용은 대화상태+루프뿐. 무거운 건 CLI 껍데기(Node+번들)지 LLM 아님.

## 4. 교육 walkthrough 진척 (대화로 정독한 것 — doc엔 일부만)
- **A (Claude 1st-party):** spawn()=OS 프로세스 생성함수 / 손자(서브에이전트)는 claude 안에 갇혀 engram이 핸들·출력 못 잡음(engram=관객) / 메모리=고정 baseline vs 가변(TUI·LSP·MCP·CLAUDE.md) / `claude --help` 전체 args 정독(쓸만: `--effort`·`--bare`·`-p`+stream-json·`-w/--worktree`·`--max-budget-usd`·`--permission-mode`) / PTY 1:1(멀티 불가) / 헤드리스·프로그래매틱 제어 / 헤드리스↔TUI = spawn 선택(라이브 토글 X, kill+`--resume`로 전환) / tmux는 opt-in(`--worktree`·Teams), Windows엔 거의 무관 — engram이 곧 멀티플렉서.
- **B (서드파티 매니저):** claude-squad·Crystal(→Nimbalyst)·Conductor·vibe-kanban·ccmanager = engram 직접 사촌. 전부 process-per-agent + worktree-per-agent. 차용: Crystal SQLite 영속 / ccmanager 4상태머신+훅 / worktree 자동.
- **C1 (named tools):** Gastown(계층 supervisor: Mayor·Polecat·**Witness**·Deacon + **Seance** 세션복구) / multiagentcoordinator(불확실, 디스패처 패턴) / claude-flow→ruflo(Queen-스웜, 과함). 사용자 판단: "C1 크게 배울 점 없다."
- **C2 (프레임워크):** durable execution(Temporal/Restate replay, LangGraph thread_id 체크포인트)가 금광이나 — 사용자 판단: **"resume은 먼 얘기"** → 후순위. A2A Agent Card(capability 광고)만 참고.
- **C3 (Rust supervision):** OTP(let-it-crash·restart-intensity {MaxR,MaxT}·전략 3종), Ractor보다 plain-tokio+개념 이식 권장. **단 engram은 ADR-0019로 런타임 자동재시작 폐기 → 도입=재오픈 결정.**

## 5. ★ 열린 결정 (사용자 미정 — 다음에 받을 것)
1. **메모리 접근:** 옵션 0(가드만: maxParallelAgents·lazy spawn·`--bare`) / 1(API in-process) / 1b(engram 자체 out-of-proc API worker pool) 중.
2. **팀 제어(옵션 2, cwd 오케스트라 + lifecycle, §5)** 지금 vs 나중.
3. **supervision 자동재시작** 도입 = **ADR-0019 번복 ADR** 필요.

## 6. 문서 org 상태 (사용자 결정 반영)
- 리서치 doc은 `docs/research/`에 **단독 거주.** tracking.md엔 **안 넣음**(사용자 2026-06-24 결정: "액션 보류 아니라 참조 자료, 주제 나오면 거기 보고 판단").
- ⚠️ **엄밀히는 orphan**(tracking/step-log/앵커 포인터 없음) — 다음 세션은 `docs/research/` 직접 보거나 이 핸드오프로 찾아야 함.

## 7. 다음 스텝
- **바로 이어갈 것:** "오케스트라 관점 5축 정리"(토폴로지·에이전트 수명·두뇌(제어주체)·컨텍스트 공유·supervision) — 대화에서 도출했으나 **doc 미편입**. 다음 세션에 doc 별도 섹션으로 박기(research doc §7 TODO에도 적힘).
- 그 후: 사용자가 §5 열린 결정 중 하나 고르면 → 해당 옵션 TRD/ADR로.

## 8. 잔여 UNCERTAIN + 전략 리스크 (해석)
- **측정 필요:** codex_api 실제 메모리(추정 ~MB/agent, 미측정) · CLI Task subprocess 여부 · claude `agents`(background)·`--remote-control` 정체 · "Agent View" 정체.
- **전략 리스크(내 해석):** ① **Anthropic 자가 흡수** — claude가 `agents`·Teams·`ultrareview`·`--remote-control`을 1st-party로 넣는 중 = engram·사촌 commoditize 위험(최대 변수, 스파이크 가치). ② engram 차별점 = §5 LLM-우선 제어 + 백엔드 추상화인데 **대부분 미실현.** ③ Windows-first = AI 얼리어답터 공개 커뮤니티(Mac/Linux)엔 불리, 사내·엔터프라이즈·게임개발엔 적합.

## 9. 운영 메모
- 조사 방식: sonnet Explore 5 병렬 + Codex(저단가, web-light 추론·리뷰) — Opus 세션한도 보호. 이 분산이 quota에 효과적이었음. 재개 시 동일 패턴 권장.
- deep-research는 안 씀(메모리 폭주·재개 불가). 서브에이전트 격리 + 파일 체크포인트로 대체.
