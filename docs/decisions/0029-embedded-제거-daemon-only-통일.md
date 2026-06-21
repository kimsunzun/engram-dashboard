# ADR-0029: embedded(싱글) 모드 제거 — daemon-only 통일 (ADR-0027 폐기, 0020/0026 일부 정리)

- 상태: 확정 (2026-06-21, dashboard10 세션 — 사용자 결정)
- 관련: ADR-0020(프로토콜+transport seam — InProc carrier 제거)·ADR-0026(2프로세스 토폴로지 — daemon-only 로 좁힘)·ADR-0027(모드별 인스턴스/데이터 — **본 ADR이 폐기**)·ADR-0028(이벤트버스)·ADR-0013(tmux/Docker daemon-always 참조) · `docs/process/step-log.md` dashboard10
- 범위: embedded(in-process 호스팅)와 daemon(별도 프로세스 호스팅) 두 모드를 유지하느냐. **embedded 를 코드베이스에서 제거하고 daemon-only 로 통일한다.**

## 맥락
스텝3에서 모드 시스템(`--mode` 인자, `default_data_dir(mode)`, 트레이/X/single-instance/data 위치 게이트)을 넣다 보니 `if mode == Daemon` **비교 분기가 코드 전반에 번식**했다. 이는 프로젝트 1조(추상 인터페이스 + 교체 가능 구현, 비교 분기 금지)를 어긴다.

원인 분석: transport seam 은 **데이터 평면**을 깔끔히 교체하지만, embedded/daemon 은 *운반만 다른 게 아니라* **제품 형태가 다르다**(VS Code식 폴더별 인프로세스 앱 vs Docker Desktop식 상주 서비스). 그래서 lifecycle/topology(인스턴스 스코프·데이터 위치·트레이·창 닫기·autostart)에서 분기가 번식한다. 로드맵 예측 결과 **거의 모든 미래 기능이 daemon-native**(원격·모바일·멀티클라이언트·오케스트레이션·에이전트 영속·레이아웃 영속·updater) — embedded 는 점점 "미지원/열화" 목록이 길어지는 음의 자산.

참조 아키텍처(tmux·Docker·ttyd, ADR-0013)는 **전부 daemon-always**(클라가 로컬이든 원격이든 attach). embedded 같은 인프로세스 모드는 선례가 없다.

안전 확인(recon): 데몬은 **이미 완전한 에이전트 호스트**다 — `crates/engram-dashboard-daemon` 의 `ConnectionCore`+`AgentManager` 가 spawn/kill/write/resize/profile CRUD 전 variant 를 처리하고, `tests/ws_e2e.rs` 38개 in-proc WS E2E + 6개 실프로세스 케이스가 spawn→출력→kill 을 입증. 프론트 daemon 경로(wsTransport)도 discover→connect→spawn→output 완비. `connection superseded` 경고는 부팅 시 connect 재트리거의 의도된 supersede 신호(무해). → **embedded 제거해도 에이전트 기능 손실 없음.**

## 결정
**embedded 모드를 제거하고 daemon-only 로 통일한다.**

- **에이전트 호스트 = 항상 데몬**(별도 프로세스). 앱(Tauri)은 데몬의 **상주 클라이언트**: 창·트레이·로컬 제어 + 데몬 연결. in-process AgentManager 호스팅 제거.
- **모드 축(embedded/daemon) 소멸 → 데몬 위치 축(로컬/원격)으로 흡수.** "로컬 사용" = 자동 spawn 된 localhost 데몬 + 로컬 WS. "원격/모바일" = 원격 데몬 + WSS. **이 변이는 이미 transport seam(wsTransport URL)** 이라 새 분기 아님. 앱은 항상 WS 로 데몬에 attach.
- **제거 대상:** 프론트 `inProcTransport`·`EmbeddedDaemonControl`·`clientFactory` mode 분기. src-tauri `embedded_carrier`·in-proc AgentManager/store/tracker 배선·`AppState.{manager,embedded,mode}`·`shutdown_all`. 스텝3 mode 시스템 전부(`AppMode`·`parse_mode`·`resolve_mode`·`default_data_dir(mode)` embedded 분기·`set_mode`·`instance.rs` embedded mutex·`__ENGRAM_MODE__` 주입·트레이/X/single-instance/`--hidden` 게이트).
- **단순화(분기 소멸):** 트레이 **항상 on**, X **항상 hide**(상주), single-instance = 데몬 전역 mutex + 앱 tauri-plugin 무조건 등록, `default_data_dir` 무인자(release=appdata / debug=repo-root), `ENGRAM_DATA_DIR` override 유지(테스트 격리 — load-bearing).
- **코어(`engram-dashboard-core`) 유지** — 데몬이 호스트 엔진으로 의존. src-tauri 의 core re-import 만 제거.

## 거부한 대안
- **embedded·daemon 둘 다 유지 + AppModeRuntime seam(2구현)** — 분기를 seam 뒤로 정리하나 영구적 2벌 유지 세금. 로드맵상 daemon 만 자라 비대칭 심화. embedded 가 음의 자산이라 seam 으로 감싸느니 제거가 옳다.
- **흩뿌린 `if mode ==`(스텝3 현행)** — 1조 위반. 모드 추가/기능 추가마다 N군데 수동 분기. 기각(이게 문제였음).
- **embedded-only(데몬 제거)** — 원격·모바일·멀티클라이언트·오케스트레이션(제품 비전·ADR-0013 전부)이 daemon-native라 불가.
- **지금 즉시 삭제(데몬 검증 없이)** — embedded 가 유일 동작 모드였으면 앱이 죽음. recon 으로 데몬 호스팅 입증 후 제거 순서(데몬 solid 확인→삭제)로 회피.

## 근거
- 1조(swappable·비교분기 금지) + ADR-0013(daemon-always 참조) + 로드맵 daemon-native 편향. 사용자 결정.
- recon: 데몬 = 완전 호스트(ws_e2e 입증), 프론트 daemon 경로 완비, connection superseded 무해. 제거 안전.

## 영향 / 불변식
- **ADR-0027 전체 폐기** — 모드가 하나면 "모드별 인스턴스/데이터"가 무의미. (single-instance=데몬 전역, data_dir=무인자.)
- **ADR-0020 의 InProc carrier 제거** — 프로토콜+transport seam 개념은 유지(이제 로컬 WS vs 원격 WSS 교체). 단일 프로토콜·ProtocolClient·ts-rs 계약 그대로.
- **ADR-0026 은 daemon-only 로 좁혀짐** — 2프로세스(앱+데몬)·로컬 제어 vs 원격 데이터 평면·트레이 통합·X=hide 는 유지(이제 무조건). "embedded=평범한 창 앱" 분기만 소멸.
- ADR-0028(이벤트버스)·ADR-0021/0024/0025(데몬 lifecycle)·ADR-0001~0019(코어) 유효.
- **데몬이 데이터 단일 소유** — in-proc store 이중소유 위험 원천 소멸.
- 제거는 **데몬 호스팅 검증 후** 순서로(데몬 solid→프론트 daemon 고정→embedded 삭제). 매 단계 앱 동작 유지.
