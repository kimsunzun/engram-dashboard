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
| [0008](0008-세션복원-sid-통제.md) | 세션 복원 — 우리가 sid 통제, 추적 파일 best-effort | 확정 (부분 폐기 by ADR-0082: resume 조기종료 → fresh-fallback 조항 폐지: 실패는 Failed 로 직행, 자동 fresh 재spawn 없음) |
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
| [0019](0019-세션-종료-분류-프로필-disposition.md) | 세션 종료 분류 — disposition(유저kill·정상=삭제 / 크래시=예약 / 셧다운=유지), 런타임 자동재시작 폐기 | 확정 (부분 폐기 by ADR-0083: 유저 kill·정상 exit(code0) → 프로필 삭제 조항 폐지: 모든 종료는 프로필 시체 보존(KeepDisableAutoRestore), 자동 삭제 없음 — 삭제는 명시적 사용자 명령으로만 / ADR-0084: apply_disposition epoch-guard 추가 — stale reap 이 재활성화된 산 세션을 auto_restore=false 로 강등 못 하게(reaped msg.epoch 가 현재 세션 epoch 와 일치할 때만 적용). 재활성화(resume)는 맵 교체이므로 epoch bump(ADR-0007 재확인).) |
| [0020](0020-클라이언트-경로-통합-단일-프로토콜.md) | 클라이언트/백엔드 경로 통합 — 단일 프로토콜 + transport-중립 dispatch core(embedded/daemon carrier만 교체) | 확정 (부분 폐기 by ADR-0037: 결정3: 프로토콜 의미론 위치 — JS ProtocolClient → Rust(DaemonClient/protocol_state)) |
| [0021](0021-데몬-수명-on-demand-무재시작.md) | 데몬 수명 — on-demand spawn + 자동재시작 없음(tmux/wezterm 모델), ensure(명시)/reconnect(attach-only) 분리 | 확정 |
| [0022](0022-통합-command-registry-palette-키바인딩.md) | 통합 command registry — palette+키바인딩+LLM+메뉴/트레이 단일 출처(VS Code 모델, 추가 여파 0 지향) | 제안 |
| [0023](0023-트레이-프로세스-토폴로지.md) | 트레이/프로세스 토폴로지 — 순수-Rust tray-host + detached 데몬 + UI(X=hide), 3프로세스 | 폐기 (Superseded by ADR-0026) |
| [0024](0024-데몬-소유-생사-종료-데이터위치.md) | 데몬 소유·생사·종료·데이터 위치 — self-owned detached + WS/lockfile liveness + 재입양 + `.engram-data/` | 확정 (C3은 0025가 폐기 · 데이터위치/공유는 0027이 폐기) |
| [0025](0025-UI-부팅-데몬-ensure-유지.md) | UI 부팅 1회 데몬 ensure 유지 — ADR-0024 C3("UI ensure 금지") 폐기 | 확정 |
| [0026](0026-트레이-앱-통합-2프로세스.md) | 트레이/프로세스 토폴로지 재결정 — 트레이를 Tauri 앱에 통합(2프로세스), 데몬 별도 (ADR-0023 폐기) | 확정 |
| [0027](0027-모드별-인스턴스-스코프-데이터위치.md) | 모드별 인스턴스 스코프 + 데이터 위치 — embedded=폴더별/폴더-로컬, daemon=전역/유저-global | 폐기 (Superseded by ADR-0029) |
| [0028](0028-백엔드-이벤트버스-소유-단일push채널.md) | 백엔드가 이벤트버스 소유 — 단일 push 채널(백엔드→트레이/WebView/LLM), 상태는 항상 아래로 (ADR-0003 일반화) | 확정 |
| [0029](0029-embedded-제거-daemon-only-통일.md) | embedded(싱글) 모드 제거 — daemon-only 통일, 모드 축→데몬 위치(로컬/원격) 흡수 (ADR-0027 폐기, 0020/0026 일부 정리) | 확정 (부분 폐기 by ADR-0134: 단일 인스턴스 스코프와 릴리스 데이터 폴더 위치 대체) |
| [0030](0030-capability-합성-transport-backend.md) | capability 산출 = transport(물리) ⊕ backend(프로그램) 합성 — 타입으로 소유권 강제, shell resume=false 정확화 (ADR-0002 구체화) | 확정 |
| [0031](0031-검수체계-opus-codex-2자-적대리뷰.md) | 검수 체계 = opus + Codex 2자 적대 리뷰(단계별 특화 Advocate/Adversary) — 웹 consult 폐기, 불일치→사용자, effort 메인 xhigh | 확정 |
| [0032](0032-주석컨벤션-2계층-overview헤더.md) | 주석 컨벤션 = 2계층(인라인 좁히기 + load-bearing overview 헤더) + ADR 앵커 점진 확대 (캐논 docs/reference/commenting-conventions.md) | 확정 |
| [0033](0033-검증기록-스킬-인프라-2층-골격바인딩-adr-하이브리드.md) | 검증·기록 스킬 인프라 — 2층 골격+바인딩 + adr 하이브리드 | 확정 |
| [0034](0034-문서-아키텍처-개발-플로우-중심-frame-claudemd-라우터화.md) | 문서 아키텍처 — 개발 플로우 중심 frame + CLAUDE.md 라우터화 | 확정 |
| [0035](0035-레이아웃-권위-src-tauri-클라-데몬-ui-불가지론-에디터-모델.md) | 레이아웃 권위 = src-tauri 클라 (데몬 UI 불가지론, 에디터 모델) | 확정 (부분 폐기 by ADR-0057: ViewManager 내부 모델: 전역 active_view_id(main-전용)+window_bindings → 창별 active + view_owner/windows 탭 소유) |
| [0036](0036-전송-중계-통일-src-tauri-단일-데몬-클라이언트-출력-라우터-창tauri-ipc.md) | 전송 중계 통일 — src-tauri 단일 데몬 클라이언트 + 출력 라우터 (창=Tauri IPC) | 확정 |
| [0037](0037-전송-의미론-위치-rust-단독-가드-프론트-protocolclient-박막화.md) | 전송 의미론 위치 — Rust 단독 가드, 프론트 ProtocolClient 박막화 | 확정 (부분 폐기 by ADR-0046: seq dedup/진도 거처 조항: Rust 단독 → 웹뷰 뷰 단위 lastDeliveredSeq — epoch 1차 필터는 Rust 존속) |
| [0038](0038-비자명-기술결함은-솔로-추측매직넘버-대신-oss-사례-조사-우선.md) | 비자명 기술결함은 솔로 추측·매직넘버 대신 OSS 사례 조사 우선 | 확정 |
| [0039](0039-프론트-이벤트-구독-teardown-subscribeviewevents-동기-disposeready-반환.md) | 프론트 이벤트 구독 teardown — subscribeViewEvents 동기 dispose+ready 반환 | 확정 |
| [0040](0040-출력-관리-단위-view-독립-중계-허브-공유-버퍼-per-view-인덱스.md) | 출력 관리 단위 = View 독립 (중계 허브 공유 버퍼 + per-view 인덱스) | 폐기 (Superseded by ADR-0046) |
| [0041](0041-데몬-출력-구독-소유-layout-델타-단독-프론트-직접-구독-차단.md) | 데몬 출력 구독 소유 = layout 델타 단독 (프론트 직접 구독 차단) | 확정 |
| [0042](0042-구독-델타-slot-단위-diff-agent-union-한계-보완.md) | 구독 델타 = slot 단위 diff (agent-union 한계 보완) | 확정 |
| [0043](0043-mount-replay-actor-경유-deliverable-게이트-배정등록-fresh-분기.md) | mount-replay = actor 경유 + deliverable 게이트 + 배정·등록 fresh 분기 | 확정 (부분 폐기 by ADR-0046: deliverable gate·미러 cursor 메커니즘 조항: 폐기 → 뷰 buffering phase + gen 펜스로 대체 — mount-replay 원칙 자체는 전량 재replay로 승계) |
| [0044](0044-json-모드-배선-stdiotransport-신설-바이트-통로-공용-지속-프로세스.md) | JSON 모드 배선 — StdioTransport 신설 + 바이트 통로 공용 + 지속 프로세스 | 확정 (부분 폐기 by ADR-0045: 통로 무정제·프론트 파싱 → 백엔드 서버 정제(타입 OutputEvent)로 전환) |
| [0045](0045-출력-정제를-백엔드로-이동-타입-outputevent를-서버에서-파싱해-wire로-흘림.md) | 출력 정제를 백엔드로 이동 — 타입 OutputEvent를 서버에서 파싱해 wire로 흘림 | 확정 |
| [0046](0046-pc-미러-버퍼-제거-뷰-직결-replayview-direct-single-flight-gen-펜스.md) | PC 미러 버퍼 제거 — 뷰 직결 replay(view-direct) + single-flight gen 펜스 | 확정 |
| [0047](0047-프론트-스타일링-tailwind-css-v4-shadcnlucide-채택-순수-css-기조-전환.md) | 프론트 스타일링 = Tailwind CSS v4 + shadcn/lucide 채택 (순수 CSS 기조 전환) | 확정 (부분 폐기 by ADR-0048: 채팅 UI 렌더 방식: CC룩 네이티브 직접 구현·OSS 참조한정(코드 복붙 아님) → Cline 잎 컴포넌트 verbatim 코드 포트(Apache-2.0 귀속)) |
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
| [0062](0062-agentlist-mvp-상태-표현-5-glyph-어휘-현-백엔드-3-state-매핑.md) | AgentList MVP 상태 표현 = 5-glyph 어휘 / 현 백엔드 3-state 매핑 | 확정 |
| [0063](0063-슬롯-콘텐츠-배치-제어표면-set-slot-content-제네릭-command-부팅-기본-agentlistempty-분할-고정-사이드패널-제거.md) | 슬롯 콘텐츠 배치 제어표면 = set_slot_content 제네릭 command + 부팅 기본 = AgentList·Empty 분할 (고정 사이드패널 제거) | 확정 |
| [0064](0064-슬롯-컨텍스트-메뉴-단일-기여-api-공통-target별-콘텐츠-co-location-등록-일원화-command-단일소스.md) | 슬롯 컨텍스트 메뉴 = 단일 기여 API (공통 target=별 + 콘텐츠 co-location + 등록 일원화, command 단일소스) | 확정 (부분 폐기 by ADR-0065: descriptor 스키마 확장: hideOn 제외조건 + children 1단 서브메뉴 (when-DSL 연기를 hideOn으로 부분 실현)) |
| [0065](0065-슬롯-메뉴-descriptor-확장-hideon-제외조건-children-1단-서브메뉴-빈-슬롯-트림-콘텐츠-채움-접기.md) | 슬롯 메뉴 descriptor 확장 — hideOn 제외조건 + children 1단 서브메뉴 (빈-슬롯 트림 + 콘텐츠-채움 접기) | 확정 |
| [0066](0066-슬롯-포커스배치-제어-표면-click-to-focus-focus-then-place-slot-geometry-노출-5-llm-제어.md) | 슬롯 포커스·배치 제어 표면 — click-to-focus + focus-then-place + slot geometry 노출 (§5 LLM 제어) | 확정 (부분 폐기 by ADR-0067: 결정 2(focus-then-place 배치) + 결정 5(크로스-윈도우 place 타깃) → 우클릭 컨텍스트 메뉴 배치로 대체 / ADR-0068: 결정 3: LLM 공간 타깃 = geometry {x,y,w,h} 좌표 노출 우선 → 논리 도면 기반 방향/이웃/순서 핸들 우선으로 개정 (좌표계·실측 픽셀 보류)) |
| [0067](0067-슬롯-콘텐츠-배치-우클릭-컨텍스트-메뉴-2경로-검색-팝업-focus-then-place-대체.md) | 슬롯 콘텐츠 배치 = 우클릭 컨텍스트 메뉴 2경로 + 검색 팝업 (focus-then-place 대체) | 확정 |
| [0068](0068-llm-공간-타깃-논리-도면viewmanager-방향이웃순서-핸들-우선-geometry-좌표-노출-보류.md) | LLM 공간 타깃 = 논리 도면(ViewManager) 방향·이웃·순서 핸들 우선 (geometry 좌표 노출 보류) | 확정 |
| [0069](0069-ui-문자열-중앙화-i18n-ready-strings-모듈-완전-i18n-보류.md) | UI 문자열 중앙화 — i18n-ready strings 모듈 (완전 i18n 보류) | 확정 |
| [0070](0070-디스플레이-이름-백엔드-저장-override-presetname-agentprofiledisplay-name-serde-default무마이그레이션.md) | 디스플레이 이름 = 백엔드 저장 override (Preset.name / AgentProfile.display_name, serde default·무마이그레이션) | 확정 |
| [0071](0071-persistence-registry-mutate-락-규율-storesave를-map-락-보유-중-실행-동시-rename-stale-snapshot-race-fix.md) | persistence registry mutate 락 규율 — store.save를 map 락 보유 중 실행 (동시 rename stale-snapshot race fix) | 확정 |
| [0072](0072-에이전트-트리-계층-구조백엔드-parent-id-reparentprofile-react-arborist-부활-1단-중첩부모삭제루트승격상태-글리프.md) | 에이전트 트리 = 계층 구조(백엔드 parent_id + ReparentProfile) + react-arborist 부활 — 1단 중첩·부모삭제=루트승격·상태 글리프 | 확정 |
| [0073](0073-제어-슬롯트리팔레트-포커스-제외-click-to-focus를-콘텐츠-슬롯으로-한정.md) | 제어 슬롯(트리·팔레트) 포커스 제외 — click-to-focus를 콘텐츠 슬롯으로 한정 | 확정 |
| [0074](0074-json-stream-json-모드-resume-활성화-adr0044-후속-완료-통제-sid-adr0008-재사용.md) | json(stream-json) 모드 resume 활성화 — ADR-0044 후속 완료 (통제-sid/ADR-0008 재사용) | 확정 |
| [0075](0075-상태-글리프-색-허용-활성-녹색-테마-변수-adr0062-색-아님-개정-eink-별도-모드.md) | 상태 글리프 색 허용(활성=녹색, 테마 변수) — ADR-0062 "색 아님" 개정 (e-ink 별도 모드) | 확정 |
| [0076](0076-활성화기존-세션-resume-fresh는-새-sid-발급재사용-금지-adr-0008-정련.md) | 활성화=기존 세션 resume, Fresh는 새 sid 발급(재사용 금지) — ADR-0008 정련 | 확정 (부분 폐기 by ADR-0077: 수동 활성화(activate_profile)도 resume 조기종료 시 restore_one 과 동일한 fresh-fallback 을 공유한다 / ADR-0082: fallback_fresh 관련 불변식·"fresh-fallback 유효" 문구 폐지: 활성화=resume·Fresh=새 sid·sid 발급 단일점은 유효) |
| [0077](0077-수동-활성화도-resume-조기종료-시-fresh-fallback-공유-adr-0076-정련.md) | 수동 활성화도 resume 조기종료 시 fresh-fallback 공유 — ADR-0076 정련 | 폐기 (Superseded by ADR-0082) |
| [0078](0078-렌더-모드는-에이전트-생성-시-결정고정-per-activation-활성화-오버라이드-폐기.md) | 렌더 모드는 에이전트 생성 시 결정·고정 (per-activation 활성화 오버라이드 폐기) | 확정 |
| [0079](0079-jsonrichslot-모드-resume-시-대화-스크롤백-복원-데몬이-claude-jsonl-transcript를-읽어-history-프레임으로-전달.md) | JSON(RichSlot) 모드 resume 시 대화 스크롤백 복원 — 데몬이 Claude `.jsonl`을 읽어 OutputCore 버퍼에 seed(단일 소스 · pump 전) | 확정 |
| [0080](0080-llm-제어-표면-아키텍처-bashengram-ctl데몬-ws백엔드-직행-데몬-opaque-relay앱-viewmanagerui.md) | LLM 제어 표면 아키텍처 — Bash→engram-ctl→데몬 WS(백엔드 직행) + 데몬 opaque-relay→앱 ViewManager(UI) | 폐기 (Superseded by ADR-0085) |
| [0081](0081-llm-ui-제어-relay-앱데몬-명령-수신-ws-peer-opaque-relay-봉투-tauri-invoke-shim-적용사람-경로-재사용.md) | LLM UI 제어 relay: 앱=데몬 명령 수신 WS peer + opaque relay 봉투 + Tauri invoke-shim 적용(사람 경로 재사용) | 확정 |
| [0082](0082-활성화이어받기resume-전용-fresh-fallback-폐지-실패는-failed시체원인-로그-llm-에이전트가-분석에스컬레이션.md) | 활성화=이어받기(resume) 전용 — fresh-fallback 폐지, 실패는 Failed(시체)+원인 로그, LLM 에이전트가 분석·에스컬레이션 | 확정 |
| [0083](0083-종료-시-프로필-자동-삭제-폐지-유저-kill정상-exit-포함-모든-종료는-시체-보존-삭제는-명시적-사용자-명령으로만.md) | 종료 시 프로필 자동 삭제 폐지 — 유저 kill·정상 exit 포함 모든 종료는 시체 보존, 삭제는 명시적 사용자 명령으로만 | 확정 |
| [0084](0084-재활성화resume-epoch-bump-apply-disposition-epoch-guard-stale-reap-산-세션-강등프론트-재구독-누락-차단.md) | 재활성화(resume) epoch bump + apply_disposition epoch-guard — stale reap 산-세션 강등·프론트 재구독 누락 차단 | 확정 |
| [0085](0085-cli-백엔드-제어-채널-in-band-출력-마커m3-engram-ctl-폐기.md) | CLI 백엔드 제어 채널 = in-band 출력 마커(M3) — engram-ctl 폐기 | 폐기 (Superseded by ADR-0086) |
| [0086](0086-제어-채널-듀얼-typed-입구mcpcli-sqlite-메일박스-first-마커m3-폐기.md) | 제어 채널 = 듀얼 typed 입구(MCP+CLI) + SQLite 메일박스-first — 마커(M3) 폐기 | 확정 (부분 폐기 by ADR-0087: 스텝 사다리 ②③④ 분할 순서 → 2-min 최소 전송 일괄 선행 + SQLite 메일박스 보류(사용자 학습 후 재개)) |
| [0087](0087-send-message-시맨틱-최소-전송-선행발신자-생사관측만봉투-데몬-렌더이름-유일채팅방-분리.md) | send_message 시맨틱 — 최소 전송 선행·발신자 생사=관측만·봉투 데몬 렌더·이름 유일·채팅방 분리 | 확정 (부분 폐기 by ADR-0088: 사다리 순서(포맷 스파이크 → 배달 정확성 검증 선행으로 재편) + 봉투 판정 축(위조내성 → 이스케이프로 이관, 포맷은 가독성·준수 한정)) |
| [0088](0088-배달-정확성-검증-선행-계측-선행위조방어이스케이프-본체포맷가독성준수-한정-adr-0087-사다리-개정.md) | 배달 정확성 검증 선행 — 계측 선행·위조방어=이스케이프 본체·포맷=가독성/준수 한정 (ADR-0087 사다리 개정) | 확정 (부분 폐기 by ADR-0091: 사다리 순서 + 단계4 분리 (포화를 포맷-수용부 뒤로 재배치; 이스케이프 유보)) |
| [0089](0089-mid-flight-epoch-race-결정론-재현-test-harness-yield-seam-배달-관측-epoch-자족화-adr-0088-후속.md) | mid-flight epoch race 결정론 재현 — test-harness yield seam + 배달 관측 epoch 자족화 (ADR-0088 후속) | 확정 |
| [0090](0090-stage-2-컨텍스트-포화-실측-실행-설계-파일럿-선행전용-실험-binsonnet-핀안전-범위-해석.md) | Stage 2 컨텍스트 포화 실측 실행 설계 — 파일럿 선행·전용 실험 bin·sonnet 핀·안전 범위 해석 | 확정 |
| [0091](0091-stage-2포화를-포맷-수용부-뒤로-재배치-단계4-분리-adr-0088-사다리-재개정.md) | Stage 2(포화)를 포맷-수용부 뒤로 재배치 + 단계4 분리 — ADR-0088 사다리 재개정 | 확정 |
| [0092](0092-s17-수신-계약-프라이밍외부-md-seam스폰-시-시스템프롬프트-주입-11-선행다중수신-추상-happy-path-first-adr-00900091-원인-정정.md) | S17 수신 계약 — 프라이밍(외부 MD seam·스폰 시 시스템프롬프트 주입) + 1:1 선행(다중수신 추상) + happy-path-first — ADR-0090/0091 원인 정정 | 확정 |
| [0093](0093-s17-답장-왕복-실험-하네스-발신-안내-케이스-매트릭스-c0c3.md) | S17 답장 왕복 실험 하네스 — 발신-안내 케이스 매트릭스 (C0~C3) | 확정 |
| [0094](0094-s17-에이전트-간-발신-권한-런타임-최소권한-pre-authorization-grant-seam.md) | S17 에이전트 간 발신 권한 — 런타임 최소권한 pre-authorization (grant seam) | 확정 (부분 폐기 by ADR-0097: 발신만 pre-authorize·bypassPermissions 거부 → 스폰 기본을 auto mode(bypassPermissions)로 채택(2026-07-22 사용자 결정). grant seam은 미래 공용 제약 레이어용 정책 표면으로 유지 / ADR-0098: CLI 발신 grant 번역을 절대경로 Bash({exe} *)에서 bare-name Bash/PowerShell({exe}:*) + PATH 주입으로 정렬(claude 권한 매처 미매칭 0/38 해소·배포 이식성)) |
| [0095](0095-봉투-포맷-스위칭-구조-기본-colon대체-xml-bracket-기각.md) | 봉투 포맷 스위칭 구조 — 기본 colon·대체 xml (bracket 기각) | 확정 (부분 폐기 by ADR-0096: 봉투 포맷 스위치 저장 위치·노출 방식 (결정 5)) |
| [0096](0096-봉투-포맷-운영-스위치-데몬-전역-상태-invoke-커맨드-조종-표면-전용워커-mcp-미노출.md) | 봉투 포맷 운영 스위치 — 데몬 전역 상태 + invoke 커맨드 (조종 표면 전용·워커 MCP 미노출) | 확정 |
| [0097](0097-스폰-에이전트-기본-auto-modebypasspermissions-채택-헤드리스-워커-권한-현실화.md) | 스폰 에이전트 기본 auto mode(bypassPermissions) 채택 — 헤드리스 워커 권한 현실화 | 확정 |
| [0098](0098-cli-발신-grant를-bare-name-path-주입으로-정렬-절대경로-미매칭-해소.md) | CLI 발신 grant를 bare-name + PATH 주입으로 정렬 — 절대경로 미매칭 해소 | 확정 |
| [0099](0099-채널-선택-백엔드-capability-스위치-프라이밍-정적-2파일mcp-capableboth-teaching-비-mcpcli-only.md) | 채널 선택 = 백엔드 capability 스위치 + 프라이밍 정적 2파일(MCP-capable=both-teaching / 비-MCP=CLI-only) | 확정 (부분 폐기 by ADR-0126: engram-send 폴백 교육 폐지와 채널 정합 불변식 단방향화 / ADR-0128: 결정 2 engram-send 물리 주입) |
| [0100](0100-릴리즈-패키징-포터블-폴더-조립-스크립트-co-location-불변식.md) | 릴리즈 패키징 — 포터블 폴더 조립 스크립트 (co-location 불변식) | 확정 (부분 폐기 by ADR-0134: 런타임 데이터 위치를 실행 폴더 하위로 대체) |
| [0101](0101-에이전트-canonical-이름-표시-이름display-name-cwd-basename-라우팅표시발신자명-단일화-adr-0087-이름주소-step-1.md) | 에이전트 canonical 이름 = 표시 이름(display_name ?? cwd basename) — 라우팅·표시·발신자명 단일화 (ADR-0087 이름주소 step 1) | 확정 |
| [0102](0102-부팅-레이스-방지-managed-state는-build-전-등록-프론트-부팅-pull-재시도-main-창-무한-로딩-근절.md) | 부팅 레이스 방지 — managed state는 build 전 등록 + 프론트 부팅 pull 재시도 (main 창 무한 로딩 근절) | 확정 |
| [0103](0103-메시징-v1-xml-봉투회신계약그룹인메모리-메일박스mcp-주력-입구.md) | 메시징 v1 — XML 봉투·회신계약·그룹·인메모리 메일박스·MCP 주력 입구 | 확정 (부분 폐기 by ADR-0105: 파킹 TTL 1h → 24h — 인메모리 단계 한정 / ADR-0107: notice cap 예외 통로 및 파킹 단일 수신자 전용 조항 / ADR-0111: 결정 2 request 단일 한정, 결정 4 그룹 등록 명단과 skip, 결정 5 부재 파킹, 결정 6 group 툴, 방송 소급 금지 불변식) |
| [0104](0104-메시징-v1-보완-그룹-해석-seamwake-연기idle-게이트-일괄-주입.md) | 메시징 v1 보완 — 그룹 해석 seam·wake 연기·idle 게이트 일괄 주입 | 확정 (부분 폐기 by ADR-0112: 결정 1 런타임 등록 명단 소스, 결정 2 잠든 수신자 파킹) |
| [0105](0105-파킹-ttl-1h24h-상향-인메모리-단계-시계-기반-단일-규칙-유지.md) | 파킹 TTL 1h→24h 상향 — 인메모리 단계, 시계 기반 단일 규칙 유지 | 확정 |
| [0106](0106-스폰-툴-deny-레이어-내장-sendmessage-차단제어-채널-스폰-한정.md) | 스폰 툴 deny 레이어 — 내장 SendMessage 차단(제어 채널 스폰 한정) | 확정 |
| [0107](0107-메일박스-용량순서-모델-v2-유계-2레인압력-회수in-flight-회계-c4-게이트-round-69.md) | 메일박스 용량·순서 모델 v2 — 유계 2레인·압력 회수·in-flight 회계 (C4 게이트 round 6~9) | 확정 (부분 폐기 by ADR-0114: 결정 2 압력 회수와 결정 1 NOTICE_CAP 20 / ADR-0111: 결정 6 방송 결박 파킹) |
| [0108](0108-request-계약-수명-기한-초과에도-미회신은-오픈-유지-cap-은퇴는-mark-and-sweep-d-게이트-round-27.md) | request 계약 수명 — 기한 초과에도 미회신은 오픈 유지 + cap 은퇴는 mark-and-sweep (D 게이트 round 2~7) | 확정 (부분 폐기 by ADR-0114: 결정 2 REQUEST_CAPACITY 반려 층위 / ADR-0118: 결정 1 계약 종결 조건) |
| [0109](0109-그룹-v1-관리-semantics듀얼-입구-계약-전원-수정암묵-생성-예약정규화-단일점-settings-파일-주입.md) | 그룹 v1 관리 semantics·듀얼 입구 계약 — 전원 수정·암묵 생성·@ 예약·정규화 단일점·--settings 파일 주입 | 확정 (부분 폐기 by ADR-0111: 결정 1~5 그룹 관리 semantics) |
| [0110](0110-메시징-코어-lib-분리-완전-상호무지포트-소유-구조.md) | 메시징 코어 lib 분리 — 완전 상호무지·포트 소유 구조 | 확정 (부분 폐기 by ADR-0127: 결정 3의 TapHost 포트와 결정 4 분류 거처) |
| [0111](0111-메시징-발송-개편-부재-수신자-반려다중-수신자-직접-발송그룹-개인-경로-통일.md) | 메시징 발송 개편 — 부재 수신자 반려·다중 수신자 직접 발송·그룹 개인 경로 통일 | 확정 (부분 폐기 by ADR-0117: 결정 1 부재 수신자 범위 / ADR-0121: @all 명단 소스) |
| [0112](0112-메시징-발송-개편-연동-그룹-해석-소스-축소잠든-수신자-반려.md) | 메시징 발송 개편 연동 — 그룹 해석 소스 축소·잠든 수신자 반려 | 확정 (부분 폐기 by ADR-0116: 결정 2 잠든 수신자 반려 / ADR-0121: 결정 1 `@all` 명단 소스 — 해석 seam의 *방향*은 그대로 존속하고, 그 소스가 내는 어휘가 둘로 늘었다) |
| [0113](0113-턴-상태busy-관측-공용-승격-사실은-공용정책은-소비자배선은-데몬.md) | 턴 상태(busy) 관측 공용 승격 — 사실은 공용·정책은 소비자·배선은 데몬 | 확정 (부분 폐기 by ADR-0119: 공용 시설 거처 / ADR-0127: 결정 3 배선 거처) |
| [0114](0114-발송-개편-보완-회수-폐지통지-상한-64주소-오류-전체-반려-층위동명-과도기-규칙.md) | 발송 개편 보완 — 회수 폐지·통지 상한 64·@주소 오류 전체 반려 층위·동명 과도기 규칙 | 확정 |
| [0115](0115-에이전트-이름-유일성-스폰-시-동명-자동-접미사후속-슬라이스.md) | 에이전트 이름 유일성 — 스폰 시 동명 자동 접미사(후속 슬라이스) | 확정 (부분 폐기 by ADR-0120: 유일성 검사 범위 / ADR-0123: 번호 재사용 금지 철회) |
| [0116](0116-메시징-입구-3분기-잠든-수신자-파킹-부활회신-계약-실패-종결프로필-삭제-정리.md) | 메시징 입구 판정 재편(3분기) — 잠든 수신자 파킹 부활·턴 신호 없는 상대 즉시 주입·회신 계약 실패 종결·프로필 삭제 정리 | 확정 (부분 폐기 by ADR-0121: @all 정의와 턴 신호 없는 부류의 배달 순서 / ADR-0120: 잠듦 층 동명 차단) |
| [0117](0117-발송-입구-3분기-연동-부재-판정-개정본체-adr-0116.md) | 발송 입구 판정 재편 연동 — 반려 범위 개정(본체 ADR-0116) | 확정 |
| [0118](0118-회신-계약-실패-종결-연동-계약-수명-조항-개정본체-adr-0116.md) | 회신 계약 실패 종결 연동 — 계약 수명 조항 개정(본체 ADR-0116) | 확정 |
| [0119](0119-에이전트-명부-단일-입구-프로필-비노출이름-유일성에이전트-사실-계층은-코어.md) | 에이전트 명부 단일 입구 — 프로필 비노출·이름 유일성·에이전트 사실 계층은 코어 | 확정 |
| [0120](0120-에이전트-이름-유일성-범위-확장-잠든-프로필까지-포함한-전역-유일.md) | 에이전트 이름 유일성 범위 확장 — 잠든 프로필까지 포함한 전역 유일 | 확정 (부분 폐기 by ADR-0123: 번호 재사용 금지 철회) |
| [0121](0121-우편-배달-규칙-조정-all과-here-분리-턴-신호-없는-부류의-순서-보장.md) | 우편 배달 규칙 조정 — @all과 @here 분리, 턴 신호 없는 부류의 순서 보장 | 확정 (부분 폐기 by ADR-0124: 거부한 대안 정통 FIFO 채택 전환) |
| [0122](0122-트리-항목-삭제-에이전트-생애-종료-고아-산-세션-금지안전-종료는-전면-차단.md) | 트리 항목 삭제 = 에이전트 생애 종료 — 고아 산 세션 금지·안전 종료는 전면 차단 | 확정 |
| [0123](0123-접미사-번호는-현재-명부-관측-최대치-기준-다-빠지면-재사용-허용.md) | 접미사 번호는 현재 명부 관측 최대치 기준 — 다 빠지면 재사용 허용 | 확정 |
| [0124](0124-발송은-전부-큐에-적재하고-flush-레인이-뺀다-직발송-지름길-폐지.md) | 발송은 전부 큐에 적재하고 flush 레인이 뺀다 (직발송 지름길 폐지) | 폐기 (Superseded by ADR-0125) |
| [0125](0125-발송이-자기-턴에-큐를-끝까지-비운다-동기-드레인-delivered-복원.md) | 발송이 자기 턴에 큐를 끝까지 비운다 (동기 드레인 · delivered 복원) | 확정 (부분 폐기 by ADR-0142: 겹침 가드 단일 축 서술을 두 축으로) |
| [0126](0126-mcp-실패-시-cli-우회-교육-폐지-입구는-스폰-시-capability로만-갈리고-런타임-폴백은-없다.md) | MCP 실패 시 CLI 우회 교육 폐지 — 입구는 스폰 시 capability로만 갈리고 런타임 폴백은 없다 | 확정 (부분 폐기 by ADR-0128: 결정 3 배선 존치) |
| [0127](0127-턴-관측-명단-코어-승격-분류는-backend-seam-뒤-데몬은-중계만.md) | 턴 관측 명단 코어 승격, 분류는 backend seam 뒤, 데몬은 중계만 | 확정 |
| [0128](0128-우편-채널-하드-단일화-capability로만-갈리고-물리-배선도-교육과-등호.md) | 우편 채널 하드 단일화 — capability로만 갈리고 물리 배선도 교육과 등호 | 확정 (부분 폐기 by ADR-0132: 제어 CLI 우편 동사 금지 제약) |
| [0129](0129-데몬-lib-3층-분리-네트워크-lib-에이전트-시스템-lib-얇은-조립-바이너리.md) | 데몬 lib 3층 분리 — 네트워크 lib · 에이전트 시스템 lib · 얇은 조립 바이너리 | 확정 (부분 폐기 by ADR-0130: 결정 2와 3 보류) |
| [0130](0130-데몬-lib-추가-분리-보류-목적은-net-분리로-달성-응용-층은-소비자-없음.md) | 데몬 lib 추가 분리 보류 — 목적은 net 분리로 달성, 응용 층은 소비자 없음 | 확정 |
| [0131](0131-CI-도입-windows-단일-러너-검증-전용-릴리즈는-태그-트리거.md) | CI 도입 — windows 단일 러너 · 검증 전용 · 릴리즈는 태그 트리거 | 확정 |
| [0132](0132-제어-평면-cli-단일-실행파일에-우편과-제어를-담고-우편-격리를-데몬-거절로-옮긴다.md) | 제어 평면 CLI — 단일 실행파일에 우편과 제어를 담고 우편 격리를 데몬 거절로 옮긴다 | 확정 |
| [0133](0133-우편-노출은-스폰-시-주입한-표식으로-가리고-거절-응답은-대안-채널을-알리지-않는다.md) | 우편 노출은 스폰 시 주입한 표식으로 가리고 거절 응답은 대안 채널을 알리지 않는다 | 확정 |
| [0134](0134-릴리스-데이터-폴더를-실행-폴더-하위-engram-data-로-단일-인스턴스-스코프를-데이터-폴더-기준으로.md) | 릴리스 데이터 폴더를 실행 폴더 하위 engram-data 로 + 단일 인스턴스 스코프를 데이터 폴더 기준으로 | 확정 (부분 폐기 by ADR-0135: 연결키 파일을 잠금 파일에 통합 / ADR-0136: 릴리스 데이터 폴더 이름) |
| [0135](0135-단일-인스턴스-잠금-파일이-접속-정보를-함께-나른다.md) | 단일 인스턴스 잠금 파일이 접속 정보를 함께 나른다 | 확정 |
| [0136](0136-릴리스-데이터-폴더-이름에서-앱-접두어를-뗀다.md) | 릴리스 데이터 폴더 이름에서 앱 접두어를 뗀다 | 확정 |
| [0137](0137-dev-빌드에-별도-번들-identifier-릴리스-앱과-동시-실행.md) | dev 빌드에 별도 번들 identifier — 릴리스 앱과 동시 실행 | 확정 |
| [0138](0138-릴리스-로그를-파일에-동기로-남긴다-stdout-이-아무-데도-없다.md) | 릴리스 로그를 파일에 동기로 남긴다 — stdout 이 아무 데도 없다 | 확정 |
| [0139](0139-런처는-자기-배포판의-데몬만-죽인다-이미지-이름-kill-폐지.md) | 런처는 자기 배포판의 데몬만 죽인다 — 이미지 이름 kill 폐지 | 확정 |
| [0142](0142-우편-배달-병렬화-축과-같은-세션-배제-축.md) | 우편 배달 병렬화 축과 같은 세션 배제 축 | 확정 |
