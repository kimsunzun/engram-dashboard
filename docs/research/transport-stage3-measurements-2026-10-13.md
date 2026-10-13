# 3단계 transport 결정 전 실측 (2026-10-13)

- 상태: 실측 기록 · 결정 없음 — TRD `docs/process/S21-crate-boundaries/trd-3-transport.md`(5판)의 미확인 항목 중 결정에 쓰이는 것만 쟀다.
- 방법: 경량 워커 둘(측정 · 격리 작업 사본 실험). 앱 · 데몬은 띄우지 않았다. 토큰 파일 내용은 읽지 않았다.

## M1. Origin 헤더 (→ D21)

| 클라이언트 | Origin | 근거 · 확신도 |
|---|---|---|
| 셸 Rust 클라(연결 · 끄기) | 안 보냄 | URL 문자열로 `connect_async` · `tungstenite::client` — tungstenite 0.26.2 가 붙이는 헤더는 Host · Connection · Upgrade · Version · Key 뿐(레지스트리 소스 판독) · 확실 |
| `scripts/engram.mjs`(Node 내장 WebSocket) | 안 보냄 | Node v24.14.0 로 실측(업그레이드 헤더 덤프) · 확실(이 판에 한해) |
| CLI `engram.exe` | 해당 없음 | 웹소켓을 쓰지 않는다(손조립 HTTP POST) · 확실 |
| 브라우저 직결 경로 `src/api/wsTransport.ts`(운영 아님) | 보냄 | 브라우저는 늘 보낸다 · 예상 값 = 배포 `http://tauri.localhost` · 개발 `http://localhost:1420`(`useHttpsScheme` 없음) · 미실측 |

→ 「배포판은 Origin 붙은 업그레이드를 거절」(D21 ①)은 운영 경로를 깨지 않는다. 깨지는 것은 운영 아닌 브라우저 직결 경로뿐.

## M2. 데이터 폴더 권한 (→ D26 · D22)

- ★개발 데이터 폴더 `.engram-dev`(I: 드라이브)와 그 안 `daemon\run\daemon.json` = 상속 권한으로 **Authenticated Users 수정(M) · Users 읽기(RX)**★ — 같은 PC 의 다른 로그인 사용자가 토큰을 읽고 파일을 바꿀 수 있다(`icacls` 실측 · 확실).
- 원인 = `I:\` 루트가 Authenticated Users 수정 · Users 읽기를 하위로 상속한다(새 폴더 실측 확인). `C:\` 루트도 새 하위 폴더에 Authenticated Users 수정을 상속한다(쓰지 않고 ACL 만 읽음).
- `%LOCALAPPDATA%` 아래 = SYSTEM · Administrators · 본인만(+ 정체 모를 도메인 SID 둘이 수정 — 미규명).
- 배포판 데이터 루트 = `<exe 폴더>\data` → 압축을 푼 자리의 상속을 그대로 탄다(설치본이 없어 미실측).

→ TRD D26 의 「3단계에선 범위 밖」 추천은 「지금도 실제로 열린 구멍」이라는 실측을 놓고 다시 볼 것.

## M3. 에이전트 띄우기 소요 (→ D15) — 새로 재지 않음, 기존 기록

- 재활성화 보통 약 2.2 s(GUI 실측 — step-log · ADR-0206).
- codex 첫 기동(데몬 재시작 뒤 첫 resume) `initialize` 약 9.7 s(step-log 기록).
- 막힌 활성화의 상한 = 15 s(백스톱 — 측정값 아님).

→ 요청 시한 8 s 는 codex 첫 기동보다 짧다. D15 ①(30 s 공용 + 측정)과 맞는다.

## M4. 명령 필드 `Box<RawValue>` 전환 (→ 3-1 U1)

- 격리 작업 사본에서 protocol 의 명령 버스 칸 여섯(ADR-0272 결정 3 의 네 자리)을 `Box<RawValue>` 로 바꾸고 `cargo test -p engram-dashboard-protocol` → lib 102 통과 · 0 실패 · 기존 골든 전부 바이트 동일.
- 바꿀 것: `serde_json` 의 `raw_value` 기능 한 줄 · 짝짓기 번호 함수 둘이 원문을 한 번 파싱(깨진 봉투 = `None`) · 시험 19곳 기계적 수정 · TS 바인딩 내용 변화 없음(기존 `#[ts(type)]` 로 충분).
- ★주의★: `serde_json::Value` 를 거쳐 들어오는 길(`from_value`)은 바이트를 보존하지 않는다(키 정렬 · 이스케이프 정규화 — 뜻은 보존). 셸 `forward_daemon_command` 가 `Value` 를 받는다 — 지금 프론트가 `Command` 를 만들지 않아 잠재 경로.
- 미시험: RawValue 가 `untagged` · `flatten` · 내부 태그 enum 안에 있을 때의 알려진 실패 — 여섯 칸 중 그런 자리는 없다.
