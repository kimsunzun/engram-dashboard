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
| [0007](0007-epoch-재구독.md) | epoch 맵교체 재구독 | 확정 (부분 폐기 by ADR-0046: 프론트 epoch 권위 조항: SubscribeAck 단독 → src-tauri decide_epoch 1차 필터 + 필터된 frame/마커 epoch 채택 — [agentId, epoch] 재구독 원칙은 유지) |
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
| [0020](0020-클라이언트-경로-통합-단일-프로토콜.md) | 클라이언트/백엔드 경로 통합 — 단일 프로토콜 + transport-중립 dispatch core(embedded/daemon carrier만 교체) | 확정 (부분 폐기 by ADR-0037: 결정3: 프로토콜 의미론 위치 — JS ProtocolClient → Rust(DaemonClient/protocol_state) |
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
| [0032](0032-주석컨벤션-2계층-overview헤더.md) | 주석 컨벤션 = 2계층(인라인 좁히기 + load-bearing overview 헤더) + ADR 앵커 점진 확대 (캐논 docs/reference/commenting-conventions.md) | 확정 |
| [0033](0033-검증기록-스킬-인프라-2층-골격바인딩-adr-하이브리드.md) | 검증·기록 스킬 인프라 — 2층 골격+바인딩 + adr 하이브리드 | 확정 |
| [0034](0034-문서-아키텍처-개발-플로우-중심-frame-claudemd-라우터화.md) | 문서 아키텍처 — 개발 플로우 중심 frame + CLAUDE.md 라우터화 | 확정 |
| [0035](0035-레이아웃-권위-src-tauri-클라-데몬-ui-불가지론-에디터-모델.md) | 레이아웃 권위 = src-tauri 클라 (데몬 UI 불가지론, 에디터 모델) | 확정 (부분 폐기 by ADR-0057: ViewManager 내부 모델: 전역 active_view_id(main-전용) |
| [0036](0036-전송-중계-통일-src-tauri-단일-데몬-클라이언트-출력-라우터-창tauri-ipc.md) | 전송 중계 통일 — src-tauri 단일 데몬 클라이언트 + 출력 라우터 (창=Tauri IPC) | 확정 |
| [0037](0037-전송-의미론-위치-rust-단독-가드-프론트-protocolclient-박막화.md) | 전송 의미론 위치 — Rust 단독 가드, 프론트 ProtocolClient 박막화 | 확정 (부분 폐기 by ADR-0046: seq dedup/진도 거처 조항: Rust 단독 → 웹뷰 뷰 단위 lastDeliveredSeq — epoch 1차 필터는 Rust 존속) |
| [0038](0038-비자명-기술결함은-솔로-추측매직넘버-대신-oss-사례-조사-우선.md) | 비자명 기술결함은 솔로 추측·매직넘버 대신 OSS 사례 조사 우선 | 확정 |
| [0039](0039-프론트-이벤트-구독-teardown-subscribeviewevents-동기-disposeready-반환.md) | 프론트 이벤트 구독 teardown — subscribeViewEvents 동기 dispose+ready 반환 | 확정 |
| [0040](0040-출력-관리-단위-view-독립-중계-허브-공유-버퍼-per-view-인덱스.md) | 출력 관리 단위 = View 독립 (중계 허브 공유 버퍼 + per-view 인덱스) | 폐기 (Superseded by ADR-0046) |
| [0041](0041-데몬-출력-구독-소유-layout-델타-단독-프론트-직접-구독-차단.md) | 데몬 출력 구독 소유 = layout 델타 단독 (프론트 직접 구독 차단) | 확정 |
| [0042](0042-구독-델타-slot-단위-diff-agent-union-한계-보완.md) | 구독 델타 = slot 단위 diff (agent-union 한계 보완) | 확정 |
| [0043](0043-mount-replay-actor-경유-deliverable-게이트-배정등록-fresh-분기.md) | mount-replay = actor 경유 + deliverable 게이트 + 배정·등록 fresh 분기 | 확정 (부분 폐기 by ADR-0046: deliverable gate·미러 cursor 메커니즘 조항: 폐기 → 뷰 buffering phase + gen 펜스로 대체 — mount-replay 원칙 자체는 전량 재replay로 승계) |
| [0044](0044-json-모드-배선-stdiotransport-신설-바이트-통로-공용-지속-프로세스.md) | JSON 모드 배선 — StdioTransport 신설 + 바이트 통로 공용 + 지속 프로세스 | 확정 (부분 폐기 by ADR-0045: 통로 무정제·프론트 파싱 → 백엔드 서버 정제(타입 OutputEvent) |
| [0045](0045-출력-정제를-백엔드로-이동-타입-outputevent를-서버에서-파싱해-wire로-흘림.md) | 출력 정제를 백엔드로 이동 — 타입 OutputEvent를 서버에서 파싱해 wire로 흘림 | 확정 |
| [0046](0046-pc-미러-버퍼-제거-뷰-직결-replayview-direct-single-flight-gen-펜스.md) | PC 미러 버퍼 제거 — 뷰 직결 replay(view-direct) + single-flight gen 펜스 | 확정 |
| [0047](0047-프론트-스타일링-tailwind-css-v4-shadcnlucide-채택-순수-css-기조-전환.md) | 프론트 스타일링 = Tailwind CSS v4 + shadcn/lucide 채택 (순수 CSS 기조 전환) | 확정 (부분 폐기 by ADR-0048: 채팅 UI 렌더 방식: CC룩 네이티브 직접 구현·OSS 참조한정(코드 복붙 아님) |
| [0048](0048-채팅-렌더-cline-잎-컴포넌트-verbatim-포트-우리-dispatch-react-markdown-스택apache-20-귀속.md) | 채팅 렌더 = Cline 잎 컴포넌트 verbatim 포트 + 우리 dispatch (react-markdown 스택·Apache-2.0 귀속) | 폐기 (Superseded by ADR-0050) |
| [0049](0049-json-에이전트-thinking-기본-활성화-max-thinking-tokens-백엔드-주입.md) | JSON 에이전트 thinking 기본 활성화 — MAX_THINKING_TOKENS 백엔드 주입 | 확정 |
| [0050](0050-채팅-렌더-자체-구현-cline-포트-제거-claude-code-vscode-확장-시각-벤치마크.md) | 채팅 렌더 = 자체 구현 (Cline 포트 제거) + Claude Code VSCode 확장 시각 벤치마크 | 확정 |
| [0051](0051-채팅-렌더-스타일간격폰트을-llm-제어-프론트-control-surface로-노출-zustandcss변수localstorage-영속.md) | 채팅 렌더 스타일(간격·폰트)을 LLM 제어 프론트 control surface로 노출 — Zustand+CSS변수+localStorage 영속 | 확정 |
| [0052](0052-json-모드-유저-에코-중복-제거-uuidisreplay-기반-dedup-blunt-suppress-폐기.md) | json 모드 유저 에코 중복 제거 = uuid/isReplay 기반 dedup (blunt suppress 폐기) | 확정 |
| [0053](0053-채팅-슬롯-오버레이-스크롤바-radix-scrollarea-채택-네이티브-css전용-라이브러리자작-거부.md) | 채팅 슬롯 오버레이 스크롤바 = Radix ScrollArea 채택 (네이티브 CSS·전용 라이브러리·자작 거부) | 확정 |
| [0054](0054-런타임-webviewwindow는-config-창과-동일한-webview2-additionalbrowserargs를-써야-한다-환경-옵션-parity-불변식.md) | 런타임 WebviewWindow는 config 창과 동일한 WebView2 additionalBrowserArgs를 써야 한다 (환경 옵션 parity 불변식) | 확정 |
| [0055](0055-command-registry-구현-방향-프론트-레지스트리-handler-라우팅기존-invoke-재사용-골격-먼저점진-이관-adr-0022-구체화.md) | command registry 구현 방향 — 프론트 레지스트리 + handler 라우팅(기존 invoke 재사용), 골격 먼저·점진 이관 (ADR-0022 구체화) | 확정 |
| [0056](0056-탭-전환-렌더링-전략-keep-alivea-보이는-슬롯만-webgl-좌석-렌더모드domxterm-교체-레버.md) | 탭 전환 렌더링 전략 — keep-alive(A) + 보이는 슬롯만 WebGL 좌석, 렌더모드(dom/xterm) 교체 레버 | 확정 |
| [0057](0057-탭-소유-모델-창별-탭-유니크-소유-owner-index-하이브리드.md) | 탭 소유 모델 — 창별 탭 + 유니크 소유 (owner-index 하이브리드) | 확정 |
| [0058](0058-spawn-into-명시-backend-pre-spawn-fail-loud-데몬-wire-부재-조용한-셸-대체-금지.md) | spawn_into 명시 backend = pre-spawn fail-loud (데몬 wire 부재 — 조용한 셸 대체 금지) | 확정 |
| [0059](0059-spawn-into-slotnone-탭-첫-빈-슬롯-스캔-leftmost-root-only-거부-없으면-noemptyslot.md) | spawn_into slot=None = 탭 첫 빈 슬롯 스캔 (leftmost-root-only 거부 — 없으면 NoEmptySlot) | 확정 |
| [0060](0060-슬롯-콘텐츠-모델-타입드-유니온slotcontent-enum-view-type-레지스트리p2urip3-거부.md) | 슬롯 콘텐츠 모델 = 타입드 유니온(SlotContent enum) — view-type 레지스트리(P2)·URI(P3) 거부 | 확정 |
| [0061](0061-프리셋-영속-데몬-소유-presetsjson-프로필-패턴-미러.md) | 프리셋 영속 = 데몬 소유 (presets.json, 프로필 패턴 미러) | 확정 |
