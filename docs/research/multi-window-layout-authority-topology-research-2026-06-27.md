# 리서치 — 멀티 윈도우 레이아웃 권위 위치 & 연결 토폴로지 (2026-06-27)

**상태:** 완료 (deep · 설계-결정 모드)
**방법:** Claude(Sonnet) 3갈래 BLIND 팬아웃 + Codex 2회 BLIND 독립 교차 → Opus aggregator 교차 대조 + 적대 검증
**확신도 범례:** 확실 / 가능성 높음 / 불확실
**계기:** S14 멀티 페이지 레이아웃 F1(ViewManager 소유권) — 기존 리서치 3건이 "레이아웃 상태 동기화"만 다루고 **권위 위치 + 연결 토폴로지**를 미커버(갭). 이 보고서가 그 갭을 채운다.

---

## 결정 질문

- **Q-A (레이아웃 권위):** Views·split/pane 트리·slot→agent 매핑의 single source of truth를 (a) 에이전트 데몬 / (b) src-tauri Rust / (c) 각 창 JS 중 어디 두나?
- **Q-B (연결 토폴로지):** 창마다 데몬에 직접 WS vs src-tauri가 단일 WS 쥐고 창들에 멀티플렉싱/중계?

---

## 핵심 발견 (Claude ↔ Codex 전면 수렴)

### F-1. 터미널 멀티플렉서 = **서버(데몬)가 레이아웃 권위** — 확실
tmux("all its state in a single main process, the server"), Zellij(서버 `Screen`이 tab→pane 소유), WezTerm Mux(중앙 싱글톤이 Window→Tab→Pane 소유), GNU screen(데몬이 region layout 저장/복원), mosh(서버가 터미널 화면 state 권위, 클라는 동기화 사본 + predictive echo). 클라이언트 = thin renderer/input. **근거: detach/reattach·다중 클라 동시 뷰·crash 격리는 "서버만 state를 안다"는 전제 위에서만 성립.**
- **Codex 보강(가능성 높음):** WezTerm은 **local 도메인이면 GUI 프로세스, Unix/SSH/TLS 원격 도메인이면 mux 서버**가 소유 — 권위 위치가 도메인에 따라 갈린다. 즉 *원격이 되는 순간 서버 권위가 강제*된다.

### F-2. GUI 에디터 = **레이아웃은 창/프로젝트 로컬** — 확실
VS Code(EditorPart는 창 로컬, SharedProcess는 파일감시·터미널·확장만 중앙화, 레이아웃 아님), Zed(Workspace/PaneGroup가 창 로컬, App은 메모리 소유일 뿐 아키텍처 권위 아님), JetBrains(프로젝트별 workspace.xml, 단일 JVM). Electron "main = single source of truth"는 **비즈니스 상태(파일·세션·설정)** 권위지 *레이아웃 그리드*가 아니다. 창 간 실시간 레이아웃 동기화는 없거나 드묾(영속은 workspace/project 키 공유 스토리지 경유).

### F-3. 두 계열의 **분기(divergence)와 그 이유** — 양 family 동일 진단, 메타-수렴
- 멀티플렉서: 내구 객체가 **서버측 PTY·pane·화면 state** → 서버 권위가 detach/멀티클라/대역폭 렌더를 가능케 함.
- 에디터: 에디터 그리드는 **그 창이 그리고 상호작용하는 UI state** → 로컬. 중앙은 lifecycle·파일·서비스만.
- **함의:** "우리가 어느 계열이냐"가 답을 가른다.

### F-4. Chromium browser process = 탭/윈도우 배치 권위 — 확실
`Browser`+`TabStripModel`이 탭 컬렉션·순서·"tear off tab into new window"(`DetachWebContentsAt`)를 소유. renderer는 콘텐츠 뷰, 자기가 어느 창/탭인지 결정권 없음. browser→renderers = 단일 권위 + Mojo fan-out.

### F-5. Wayland compositor = 윈도우 배치 단일 권위 — 확실
xdg_toplevel에 절대 위치 설정 API 자체가 없음. 클라는 단일 소켓에 `wl_surface`만 제출, 최종 배치는 compositor 결정(보안·다중모니터 유연성). (X11은 다름 — 클라가 위치 요청 가능, WM이 가로챔.)

### F-6. LSP = 단일 연결로 다중 문서 멀티플렉싱 — 확실
에디터↔서버 단일 transport(stdio/socket)에 모든 문서의 didOpen/didChange를 URI로 구분해 multiplex. 문서마다 연결 안 만듦. (`lspmux`가 별도 존재 = 기본은 클라 1↔서버 1.)

---

## 교차검증표

| 클레임 | Claude | Codex | 판정 |
|---|---|---|---|
| 터미널 멀티플렉서 = 서버 권위 | 확실 | high | **수렴** (Codex가 WezTerm local/remote 도메인 nuance 추가) |
| GUI 에디터 = 창 로컬 레이아웃 | 확실 | high | **수렴** |
| 두 계열 분기 + 이유 | 명시 | 명시 | **수렴(동일 진단)** |
| Chromium browser process 배치 권위 | 확실 | high | **수렴** |
| Wayland compositor 단일 권위 | 확실 | high | **수렴** |
| LSP 단일 연결 멀티플렉싱 | 확실 | high | **수렴** |
| **Q-A 권고: engram→데몬 권위** | (미질문) | **high** | Codex 단독 + Opus 종합 일치 |
| **Q-B 권고: src-tauri 단일 연결 중계** | (미질문) | medium-high | Opus: 장기 정합하나 범위 큼 → 분리 |

**사실 차원 cross-family 불일치 0건.** 만장일치 경계(공통 편향 의심) 적용해도, 1차 출처(소스코드·man page·공식 문서)가 받치므로 환각 가능성 낮음.

---

## 제약 적합도 표 — Q-A (engram 제약 × 옵션)

| 제약 | (a) 데몬 권위 | (b) src-tauri 권위 | (c) 각 창 JS |
|---|---|---|---|
| ADR-0029 "데몬=데이터 단일 소유, in-proc 이중소유 금지" | ✅ 정합 | ❌ **앱-프로세스 상태 소유 부활**(ADR-0029가 죽인 것) | ❌ |
| §5 LLM-우선 제어(두뇌=데몬측이 레이아웃 제어) | ✅ co-located | △ 역채널 필요(daemon→src-tauri) | △ |
| 원격 데몬 / detach·reattach(워크스페이스 따라옴) | ✅ tmux식 무료 획득 | ❌ 레이아웃이 원격 안 따라감 | ❌ |
| 우리 계열의 관행(=터미널 멀티플렉서, ADR-0013 앵커) | ✅ 일치 | ❌ 에디터 계열 가정 | ❌ |
| 기존 WS broadcast 경로 재사용 | ✅ (`onLayoutUpdated`=`onProfileListUpdated` 동형) | △ Tauri emit 2번째 채널 | — |
| 창 격리 JS split-brain 회피 | ✅ | ✅ | ❌❌ (rev.1·rev.2 2회 BLOCK 원인) |
| 범위/리스크 | 중(데몬에 ViewManager+프로토콜 추가, **transport 불변**) | 저(빠름)·**구조 퇴행** | — |

**결론(확실에 가까움): Q-A = (a) 데몬 권위.** 우리 계열(터미널 멀티플렉서) 관행 + §5 + ADR-0029 "단일 소유" 3중 정합. (b)는 ADR-0029가 제거한 이중소유를 부활시키는 퇴행.

---

## Q-B (연결 토폴로지) — 장기 정합 vs 현 범위

- **관행(LSP·Chromium·Codex medium-high):** src-tauri가 단일 연결을 쥐고 창들에 멀티플렉싱/중계 = 인증·재연결·순서·세션 중복 제거 + LLM 단일 제어 표면 + 원격 자격증명/TLS를 Rust 한 곳에.
- **반론(Codex 자기반박):** 창마다 직결이 **실패 격리 + 고대역 터미널 출력 직통**에 유리. 중계는 daemon→Rust→WebView 한 홉 + PTY 바이트 복사.
- **현 코드 충돌:** 현재 `wsTransport.ts`는 **창마다 데몬 직접 WS**(ADR-0020 transport seam). Q-B 중계로 가면 transport 재설계 = S14 범위 초과.
- **판정:** Q-B(완전 중계)는 **별도 결정/ADR로 분리.** S14는 데몬 권위(Q-A=a) + **현행 창-직결 유지** — 데몬이 레이아웃을 각 창의 기존 WS로 broadcast(F2/F3 자동 해소). 중계 일원화는 후속.

---

## 거부 후보 → ADR 거부 대안 (S14 ViewManager ADR로 이관)
- **(b) src-tauri가 레이아웃 권위 소유:** ADR-0029 이중소유 부활 + §5 역채널 + 원격 미추종. (핸드오프 A안 = 이것 → 기각.)
- **(c) 각 창 JS 권위:** 창 격리로 split-brain(rev.1·rev.2 2회 BLOCK 실증).
- **(Q-B) 지금 src-tauri 완전 중계:** 장기 정합하나 transport(ADR-0020) 재설계라 S14 범위 초과 — 후속 ADR로.

---

## 공백 / 한계
- WezTerm "local 도메인=GUI 소유" nuance는 우리(항상 데몬 attach)엔 비적용 — 우리는 항상 원격형(=서버 권위).
- Zed/JetBrains 멀티-윈도우 *동일 프로젝트* 동기화 동작은 양 family 모두 "불확실"로 미확정(우리 결정엔 비영향).
- Q-B 중계의 고대역 출력 오버헤드는 **실측 전 정량 미상** — 후속 ADR에서 프로파일링 근거로 결정.

## 출처 (1차 우선)
tmux man7 · tmux Control Mode wiki · Zellij DeepWiki(client-server·layout)·issue#4253 · WezTerm multiplexing 공식·mux/src/lib.rs · GNU screen manual(Layout/Displays) · mosh.org techinfo · VS Code Source-Code-Organization·layout.ts·Sandbox 블로그 · Zed workspace.rs·project.rs·GPUI ownership · JetBrains Remote Dev · Electron process-model/ipc · Chromium multi-process·TabStripModel · Wayland Protocol Book·xdg-shell · LSP 3.18 spec·overview · lspmux.
