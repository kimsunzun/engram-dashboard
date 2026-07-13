# ADR-0079: JSON(RichSlot) 모드 resume 시 대화 스크롤백 복원 — 데몬이 Claude `.jsonl`을 읽어 OutputCore 버퍼에 seed(단일 소스 · pump 전)

- 상태: 확정 (2026-07-13, 근거: /research OSS 서베이 + Codex 적대 리뷰 + 라이브 CLI 실측)
- 관련: ADR-0044(json 모드 배선·stream-json 디코더) · ADR-0008(세션 복원 통제-sid) · ADR-0007(epoch 재구독) · ADR-0046(view-scoped replay·gen fence) · ADR-0049(thinking) · ADR-0052(user 에코 uuid dedup) · CLAUDE.md §5(LLM-우선 제어)·§1(OutputSink) · `backend/claude.rs`(디코더·build_spec) · `agent/output_core.rs`(Ring/replay) · `agent/manager.rs`(spawn_session) · step-log S(복원 UX)

## 맥락
JSON(RichSlot) 모드에서 세션을 resume하면 채팅창이 빈 화면(새 턴만)으로 뜬다. 원인 2층(라이브 실측 2026-07-13, claude 2.1.170):

1. **claude가 과거를 재방출하지 않음.** `claude --resume <sid>`(stream-json headless)은 모델 컨텍스트만 복원하고 과거 턴을 stdout으로 다시 내보내지 않는다. 과거 history를 stdout으로 재방출하는 CLI flag도 **존재하지 않는다**(`--replay-user-messages`는 새 stdin 입력만 에코, `--resume`/`--continue`/`--fork-session`/`--from-pr`은 컨텍스트 주입뿐). 즉 우리가 flag를 빠뜨린 게 아니다.
2. **replay 버퍼는 프로세스 단위.** `OutputCore`의 Ring(replay 버퍼)은 현재 프로세스가 emit한 것만 담고 재spawn/재시작 시 리셋된다 → resume 프로세스는 빈 버퍼로 시작.

터미널(xterm) 모드는 TUI가 과거 대화를 PTY로 repaint해 xterm에 뜨지만, JSON 모드는 repaint가 없어 이 비대칭이 발생한다. resume = 프로세스를 껐다 켜는 것이므로 매 resume(데몬 재시작·재활성화 포함)마다 과거를 다시 로드해야 한다.

## 결정
**데몬이 resume 스폰 시 Claude `.jsonl` transcript를 읽어 `OutputEvent`로 변환해 `OutputCore` replay 버퍼에 seed한다 — pump(라이브 출력) 시작 전에.** 이후 라이브 이벤트는 같은 버퍼에 append된다. 버퍼가 단일 소스가 되어 기존 replay 경로(subscribe→replay→live)로 프론트에 전달되고, RichSlot은 라이브와 동일하게 렌더한다.

- **단일 소스 = `OutputCore` 버퍼.** `.jsonl` 파일 자체가 아니라 버퍼가 소스. 과거는 seed로만, 라이브는 pump로만 채워지고 둘은 겹치지 않는다(resume가 재방출 안 하므로).
- **재사용.** 과거 턴 매핑은 기존 stream-json 디코더의 블록 매핑(assistant text→`TextDelta`, tool_use→`ToolCall`, user/tool_result→`Structured{kind:"user"}`, thinking→`Structured{kind:"thinking"}`)을 그대로 쓴다. 신규 `OutputEvent` variant 없음 — 프론트 accumulator·RichSlot이 이미 이 이벤트들을 렌더하므로 프론트 변경 최소.
- **claude 전용 지식 격리(ADR-0004).** `.jsonl` 경로(cwd→프로젝트 슬러그)·라인 타입 필터·파싱은 `backend/claude.rs`에만.
- **resume = 전량 재로드.** 버퍼는 재구성 가능한 캐시, 디스크의 `.jsonl`이 durable 진실. 데몬 재시작·재활성화 등 모든 resume에서 재-seed.
- **범위·기본값.** JSON(RichSlot) 모드만(터미널 미변경). sub-agent(sidechain) 턴·summary 라인 스킵(원본 턴만). 대용량은 파일 끝에서 Ring 상한(2MB/4096)만큼 tail 파싱 + 상한 초과 시 오래된 것부터 truncate 수용(기존 `Truncated` 신고 재사용). 과거/현재 divider 없음(연속 스트림). lazy(위로 스크롤 시 older 로드)는 후속 과제.

## 거부한 대안
- **프론트가 `.jsonl`을 직접 읽어 렌더** — §5 위반(프론트가 백엔드 두뇌가 못 쥐는 능력 획득 + 순수 I/O 원칙 파기)이고 라이브/복원 렌더 경로가 이원화된다. 데몬이 읽어 전달하면 LLM도 같은 경로로 제어 가능.
- **별도 History 채널로 seed-후-append(프론트가 과거+라이브 두 소스 stitch)** — 레퍼런스 실측상 이 방식(Claudia)은 경계 턴 double-render race 버그를 가진다. 소스를 버퍼 하나로 합치면(Crystal 방식) 그 race가 구조적으로 사라져 dedup 코드가 불필요하다.
- **자체 durable 이벤트 저널을 지금 신설(B2)** — claude 전용인 현 단계엔 과잉이다(스키마 정규화·WAL·crash consistency 비용). `.jsonl`이 이미 디스크 durable 소스라 불필요. codex/gemini 범용화 시 backend별 mapper와 함께 재고(principle 0: 고비용·불확실은 그때).
- **`.jsonl`을 `tail -f`로 라이브 소스까지 대체** — `.jsonl`은 완결 메시지 단위(스트리밍 델타 없음)라 라이브 토큰 스트리밍(타이핑)이 사라지고, 평상시 hot-path가 claude 파일 포맷에 결합된다(backend-agnostic 파괴). `.jsonl`은 과거 seed 소스로만, 라이브는 기존 stream-json 경로 유지.
- **resume에 history 재방출을 켜는 flag 기대** — 그런 CLI flag가 없음(실측 확인).

## 근거
- **/research OSS 서베이(설계-결정 모드):** Claude Code CLI 래퍼 부류(Claudia/opcode=우리 스택, claude-code-webui, Crystal)의 표준 = resume 시 `.jsonl` 직접 읽어 재구성. 서버측 스토어를 가진 유일 동종(Crystal)은 **단일 소스**(스토어에 라이브 기록 + 재조회)로 경계 중복을 구조적으로 제거. seed-후-append(Claudia)는 그 지점에 race 버그(소스 직접 확인).
- **Codex 적대 리뷰:** explicit history frames·event-sourcing·§5 제어 표면·seq 정체성을 지적 → 단일 버퍼 + (필요시) 마커로 흡수. "복원을 라이브로 위장 말라"는 원칙은 seed가 과거 turn을 그대로 재현(라이브 델타 아님)하는 것으로 충족.
- **라이브 CLI 실측(2026-07-13):** `claude --resume`(stream-json)은 과거 턴을 stdout으로 재방출하지 않으며 재방출 flag 부재. `--replay-user-messages`(우리가 이미 넘김)는 새 입력 에코 전용.

## 영향 / 불변식
- **seed-before-pump(load-bearing):** resume 스폰 시 `.jsonl` seed가 `session.start_pump()`(manager.rs) **전에** 완료돼야 한다. pump를 먼저 켜면 빠른 첫 라이브 이벤트가 seed보다 앞서 도착해 순서가 깨진다. 이 순서가 정합성의 못.
- **단일 소스:** 과거=seed·라이브=pump로만 채우고 겹치지 않는다(resume 미재방출 전제). 재시작 시 버퍼가 비면 `.jsonl`을 다시 seed(캐시 재구성).
- **claude 지식 격리(ADR-0004):** `.jsonl` 파싱·슬러그 인코딩·라인 타입 지식은 `backend/claude.rs`만. transport·core는 모른다.
- **프론트 = 순수 I/O(§5):** 프론트는 파일을 읽지 않는다. 데몬이 읽어 기존 이벤트 경로로 전달한다.
- **테스트:** `.jsonl` 파서는 외부 의존 없는 순수 함수 → 실 픽스처(`backend/fixtures/`)로 단위 테스트(ADR-0012 seam 격리).
