# LLD 요청 가이드 — fable (검증자 관점)

**작성:** fable (pane 8), 2026-06-11
**대상:** engram-dashboard 설계 에이전트(pane 12)에게 상세 코딩 계획서(LLD)를 요청할 때의 형식 가이드
**근거:** `backend-architecture-final.md` 직접 검토 후 작성

---

## (2) 형식 추천 — 2단계 (구도검증 → 코드검증), 코드 단계는 모듈별 분할

**추천: 2단계 + 2단계 내부는 모듈 단위 분할.** 확신 수준: 확실.

| 방식 | 문제 |
|---|---|
| 한 방 전체 풀코드 | 수천 줄 일괄 리뷰는 adversarial 품질이 급락한다(주의 희석). 구조 결함이 코드 디테일에 가려지고, 코드 작성 후 구조 결함이 발견되면 재작업 비용이 최대가 된다. |
| 단계별 분할만 (구조 확정 없이) | 각 조각이 전역 컨텍스트 없이 와서 cross-module 결함을 못 잡는다. 이 프로젝트의 실제 위험(락 순서, drain thread 수명, subscriber 정리)이 정확히 모듈 경계에 있다. |
| **2단계** | 1단계에서 인터페이스·상태머신·동시성 모델을 검증해 **확정(freeze)**, 2단계에서 모듈별 코드를 "확정 스펙 적합성 + 로컬 버그"로 검증. 구조 결함을 가장 싼 시점에 잡고, 3자 검증자가 동일한 확정 스펙을 기준으로 보므로 판정 일관성도 올라간다. |

운영 순서: ① LLD(구조) 산출 → ② Gemini/GPT/fable 3자 검증 → ③ 반영·확정 → ④ 모듈별 코드 산출(각 모듈에 "확정 스펙 어느 항목을 구현하는지" 표기) → ⑤ 모듈별 3자 검증.

---

## (1) LLD 에 반드시 들어가야 할 것 (검증 친화 구성)

1. **모듈 맵 + 의존 방향** — 실제 파일 경로 단위 (`src-tauri/src/pty/manager.rs` 등). 다이어그램 1개.
2. **public 인터페이스를 실제 Rust 시그니처로** — prose 설명 금지. 함수 시그니처, 에러 타입, 구조체 필드 전체. "대략 이런 모양" 은 검증 불가.
3. **의존성 버전 고정** — `Cargo.toml` 스니펫 (`tauri = "2.x.y"`, `portable-pty = "0.x"`). 두 crate 모두 버전별 API 차이가 커서 버전 없이는 API 실재성 검증이 불가능하다.
4. **상태머신 전이표** — `AgentStatus` 각 전이의 트리거, 전이 수행 주체(어느 스레드), 전이 시 정리되는 자원.
5. **동시성 명세** — 락 목록 + **락 획득 순서 규칙**, 스레드 목록(누가 spawn 하고 누가 join/detach 하나), 채널 토폴로지(bounded/unbounded, 가득 찼을 때 정책).
6. **자원 수명 표** — child process / drain thread / writer / Channel 구독 / Job Object handle 각각: 생성 시점, 소유자, 해제 트리거, 해제 주체.
7. **에러·종료 경로 워크스루 3종** (시퀀스로): (a) child 비정상 종료 (b) 앱 전체 종료 시 전 PTY 정리 (c) 창 닫힘/reload 시 구독자 정리.
8. **1차 리뷰 반영 추적표** — emit_all→Channel, Job Object 등 기확정 결정이 LLD 어느 절에 반영됐는지. 검증자가 같은 지적을 반복하는 낭비를 막는다.
9. **결정–근거–기각한 대안** — adversarial 검증자에게 공격 표면을 제공.
10. **비목표(non-goals)** — scope 밖을 명시해 검증 낭비 방지.
11. **검토 요청 질문** — final.md 의 "검토 요청 사항" 절은 좋은 관행. LLD 에도 유지.

---

## (3) fable 이 검증자로서 "이게 없으면 제대로 못 본다" 하는 항목

- **drain thread 종료 메커니즘 의사코드.** `portable-pty` reader 는 blocking read 라서 깔끔한 종료가 이 설계의 최대 난제다. "`kill_agent` 호출 시 blocking read 중인 drain thread 가 어떻게 깨어나서 종료되는가" 가 명시되지 않으면 통과시킬 수 없다.
- **락 순서 규칙 명문화 + send 시점의 락 보유 여부.** drain thread 가 session lock 을 잡은 채 `subscribers` 순회하며 `channel.send` 하는지 — 잡은 채라면 send 지연 시 `write_stdin` 까지 막힌다. 현재 final.md 에는 이 부분이 비어 있다.
- **subscriber 제거 메커니즘.** final.md 는 "channel drop → 자동 제거" 라고 하는데, 실제로는 send 실패 감지로 제거해야 한다. 감지·제거 코드 흐름 명시 필요.
- **replay → live 전환의 무결성.** 후발 attach 시 replay 전송과 live stream 사이 gap/중복 없음을 `seq` 로 어떻게 보장하는지 의사코드.
- **Windows 전용 절.** Job Object 생성·할당 코드 지점, ConPTY 특이사항. 타깃이 win32 이므로 별도 절로 분리.
- **각 모듈 끝에 "이 모듈의 검증 포인트" 1~2줄** — 검증자 attention 을 위험 지점으로 유도.

---

## 설계 에이전트에게 보낼 요청문 초안

> backend-architecture-final.md 기반으로 LLD(상세 코딩 계획서)를 2단계로 작성해줘.
> **1단계 (지금):** 코드 없이 구조 확정본 — 모듈 맵(실제 파일 경로), public 인터페이스 실제 Rust 시그니처, Cargo.toml 버전 고정, AgentStatus 전이표(트리거/수행 스레드/정리 자원), 락 순서 규칙, 스레드·채널 토폴로지, 자원 수명 표, 종료 경로 워크스루 3종(child 비정상 종료/앱 종료/창 닫힘), 1차 리뷰 반영 추적표, 기각한 대안, 비목표, 검토 요청 질문. 특히 ① blocking read 중인 drain thread 의 종료 메커니즘 ② channel.send 시점의 락 보유 여부 ③ subscriber send-실패 감지 제거 ④ replay→live seq 무결성은 의사코드 수준으로.
> **2단계 (1단계 3자 검증·확정 후):** 모듈별 실제 코드. 각 모듈에 확정 스펙 대응 항목과 검증 포인트 표기.
