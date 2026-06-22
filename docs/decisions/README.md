# 설계 결정 기록 (ADR)

이 폴더는 **"왜 이렇게 정했나"를 시점 무관하게 박제**한다. `docs/process/`(언제 무엇을 했나, 시간순 흐름)와 역할이 다르다 — 여기는 영구 못(декision record).

## 왜 ADR인가

LLM 세션은 바뀌면 결정 맥락을 잊고 같은 대안을 다시 꺼낸다. ADR은 **결정 + 거부한 대안 + 이유**를 적어 재론(re-litigation)을 막는다. 특히 "거부한 대안과 그 이유"가 핵심이다 — 그게 없으면 클로드가 같은 "개선 제안"을 반복한다.

## 규칙 (CLAUDE.md에서 강제)

1. **작업 전** 관련 ADR을 먼저 읽는다.
2. **설계 결정을 내리면** 새 ADR을 추가한다(다음 번호).
3. **기존 결정을 바꾸려면** 해당 ADR을 `폐기(Superseded by ADR-NNNN)`로 표시하고 새 번호로 기록한다 — ADR은 덮어쓰지 않고 누적한다(이력 보존).

## 상태 범례

- **확정(Accepted)** — 현재 유효, 따른다.
- **제안(Proposed)** — 논의 중, 아직 강제 아님.
- **폐기(Superseded)** — 다른 ADR로 대체됨. 본문은 이력으로 남긴다.
- **거부(Rejected)** — 검토했으나 채택 안 함.

## 템플릿

```markdown
# ADR-NNNN: <한 줄 제목>

- 상태: 확정 (YYYY-MM-DD, 근거: spike/commit)
- 관련: CLAUDE.md §X · <파일:라인> · step-log SN

## 맥락
무슨 문제를 풀어야 했나.

## 결정
무엇으로 정했나.

## 거부한 대안
- 대안 A — 왜 버렸나.
- 대안 B — 왜 버렸나.

## 근거
실측·리뷰 등 결정의 뒷받침.

## 영향 / 불변식
이 결정이 묶는 코드·게이트. 어기면 무엇이 깨지나.
```

## 인덱스

| # | 제목 | 상태 |
|---|---|---|
| [0001](0001-kill-2동사.md) | kill = 2동사 (shutdown + join_pump) | 확정 |
| [0002](0002-output-event-seam.md) | 출력 seam = OutputEvent (터미널 가정 금지) | 확정 |
| [0003](0003-output-status-sink-격리.md) | OutputSink/StatusSink — 코어 Tauri 격리 | 확정 |
| [0004](0004-agent-transport-backend-격리.md) | AgentTransport seam + backend 지식 격리 | 확정 |
| [0005](0005-finalize-1회.md) | finalize 정확히 1회 (pump 단독) | 확정 |
| [0006](0006-락-순서.md) | 락 순서 규율 (sessions → 내부) | 확정 |
| [0007](0007-epoch-재구독.md) | epoch 맵교체 재구독 | 확정 |
| [0008](0008-세션복원-sid-통제.md) | 세션 복원 — 우리가 sid 통제, 추적 파일 best-effort | 확정 |
| [0009](0009-tauri-2x-핀.md) | tauri 최신 2.x 핀 (Channel 무손실 실측) | 확정 |
| [0010](0010-cargo-workspace-분리.md) | Cargo workspace 3-crate 분리 | 확정 |
| [0011](0011-agentclient-제어표면.md) | agentClient 제어 표면 facade (데몬 대비) | 확정 |
| [0012](0012-테스트-격리-하네스-tdd.md) | 테스트 전략 — 모듈 격리 하네스 + TDD | 확정 |
| [0013](0013-데몬-참조-3대장.md) | 데몬 참조 3대장 — tmux / Zellij / Mosh | 확정 |
| [0014](0014-오케스트레이션-참조-후보.md) | 오케스트레이션 참조 후보 (설계 시 고려) | 제안 |
| [0015](0015-데몬-수명-콘솔-뷰어.md) | 데몬 수명 = persist-until-kill, 콘솔 = detachable 뷰어 | 확정 |
| [0016](0016-에이전트-수명-모델.md) | 에이전트 수명 모델 — sid 인스턴스, 저장=살림·삭제=끔, 단순 가드 | 확정 (restart=Always 런타임 해석은 0019가 일부 폐기) |
| [0017](0017-세션-슬롯-구조-죽음정의.md) | 세션/슬롯 구조 — 슬롯=한 모드의 한 세션(끝나면 슬롯도 끝), 터미널 비저장, 죽음=Run 종료+이유 | 확정 |
| [0018](0018-깡통-예약-에이전트-프론트-머지.md) | 깡통(예약) 에이전트 — Reserved=프론트 합성, 백엔드 무변경 | 확정 |
| [0019](0019-세션-종료-분류-프로필-disposition.md) | 세션 종료 분류 — disposition(유저kill·정상=삭제 / 크래시=예약 / 셧다운=유지), 런타임 자동재시작 폐기 | 확정 |
| [0020](0020-클라이언트-경로-통합-단일-프로토콜.md) | 클라이언트/백엔드 경로 통합 — 단일 프로토콜 + transport-중립 dispatch core(embedded/daemon carrier만 교체) | 확정 |
| [0021](0021-데몬-수명-on-demand-무재시작.md) | 데몬 수명 — on-demand spawn + 자동재시작 없음(tmux/wezterm 모델), ensure(명시)/reconnect(attach-only) 분리 | 확정 |
| [0022](0022-통합-command-registry-palette-키바인딩.md) | 통합 command registry — palette+키바인딩+LLM+메뉴/트레이 단일 출처(VS Code 모델, 추가 여파 0 지향) | 제안 |
| [0023](0023-트레이-프로세스-토폴로지.md) | 트레이/프로세스 토폴로지 — 순수-Rust tray-host + detached 데몬 + UI(X=hide), 3프로세스 | 폐기 (Superseded by ADR-0026) |
| [0024](0024-데몬-소유-생사-종료-데이터위치.md) | 데몬 소유·생사·종료·데이터 위치 — self-owned detached + WS/lockfile liveness + 재입양 + `.engram-data/` | 확정 (C3은 0025가 폐기 · 데이터위치/공유는 0027이 폐기) |
| [0025](0025-UI-부팅-데몬-ensure-유지.md) | UI 부팅 1회 데몬 ensure 유지 — ADR-0024 C3("UI ensure 금지") 폐기 | 확정 |
| [0026](0026-트레이-앱-통합-2프로세스.md) | 트레이/프로세스 토폴로지 재결정 — 트레이를 Tauri 앱에 통합(2프로세스), 데몬 별도 (ADR-0023 폐기) | 확정 |
| [0027](0027-모드별-인스턴스-스코프-데이터위치.md) | 모드별 인스턴스 스코프 + 데이터 위치 — embedded=폴더별/폴더-로컬, daemon=전역/유저-global | 폐기 (Superseded by ADR-0029) |
| [0028](0028-백엔드-이벤트버스-소유-단일push채널.md) | 백엔드가 이벤트버스 소유 — 단일 push 채널(백엔드→트레이/WebView/LLM), 상태는 항상 아래로 (ADR-0003 일반화) | 확정 |
| [0029](0029-embedded-제거-daemon-only-통일.md) | embedded(싱글) 모드 제거 — daemon-only 통일, 모드 축→데몬 위치(로컬/원격) 흡수 (ADR-0027 폐기, 0020/0026 일부 정리) | 확정 |
| [0030](0030-capability-합성-transport-backend.md) | capability 산출 = transport(물리) ⊕ backend(프로그램) 합성 — 타입으로 소유권 강제, shell resume=false 정확화 (ADR-0002 구체화) | 확정 |
| [0031](0031-검수체계-opus-codex-2자-적대리뷰.md) | 검수 체계 = opus + Codex 2자 적대 리뷰(단계별 특화 Advocate/Adversary) — 웹 consult 폐기, 불일치→사용자, effort 메인 xhigh | 확정 |
