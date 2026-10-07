# ADR-0282: 2-2 discovery 나누기의 세부 — 실행 파일 위치는 platform 한 곳, 쓰기 프로브는 base, 셸 사본은 같은 경로 시험으로 묶는다

- 상태: 제안 (2026-10-07, 근거: 번호 선점 — 본문은 2-2 착지 때 채운다 · 정본 설계 = `docs/process/S21-crate-boundaries/trd-2-2-discovery-split.md` §7)
- 관련: ADR-0269(결정 7 입주 조건 — 아래 결정 2) · Amends ADR-0271 (결정 2의 실행 파일 위치 계산 자리)

## 맥락
TODO — 무슨 문제를 풀어야 했나.

## 결정
(사용자 위임 2026-10-07 아래의 메인 결정 · 2-2 착지 때 나머지를 채운다 — 지금은 U1 ~ U4 가 기대는 것만 적는다)

1. **실행 파일 위치 계산은 platform `env::sibling_exe` 한 곳이다** — ADR-0271 결정 2 가 이것을 「데몬으로」 두었던 자리를 고친다. 데몬의 CLI 찾기 · 셸의 데몬 찾기 · `roundtrip_smoke` 가 모두 그것을 부른다(데몬과 셸은 서로의 코드를 못 나눠 쓰고, 둘 다 platform 에는 닿는다).
2. **쓰기 프로브 도우미는 base `writable` 이다 — 입주를 2-2 U1 로 당긴다.** ADR-0269 결정 7 의 「사본 둘」 조건은 같은 단계 안의 U2 · U3 에서 성립한다. 당긴 이유 = 임시 사본을 만들지 않는다 · 프로브 이름 카운터(`probe_path` 의 static)를 데몬 판과 셸 판이 한 프로세스에서 공유해야 이름이 안 겹친다. 옮기면서 그 정리 경고의 tracing target 이 `engram_dashboard_discovery` → `engram_dashboard_base::writable` 로 바뀐다(문구 · 레벨 그대로 · 저장소 안에 그 target 을 쓰는 `RUST_LOG` 지시 0건).
3. **같은 경로 시험이 셸 사본을 데몬 정본에 묶는다** — 셸이 데몬 crate 를 운영 의존하지 않으므로(ADR-0271) 셸은 루트 찾기 · `daemon.json` 자리 · `logs\` 를 사본으로 갖고, 셸 → (dev) 데몬 간선으로 그 시험을 돌린다. 재는 것 = 루트(환경 변수 미설정 · 빈 값 · 경로 세 상태 — 변수 이름은 리터럴) · `release_data_dir` · `daemon_file` · `logs_dir`. 못 재는 것 = 루트 규칙의 release 분기 본문(시험은 늘 debug — release 1회 실측이 덮는다) · walk-up 표지 하나(`.git` · `[workspace]`)를 빠뜨린 셸 사본(이 저장소 루트에 둘 다 있다 — 표지 시험은 데몬에만 산다). 셸 → (dev) 데몬 간선을 걷거나 좁히는 날(작업 순서 2-4 이후) 이 시험과 경합 시험의 자리를 다시 정한다.
4. **환경 락은 crate 마다 하나** — `ENGRAM_DATA_DIR` 을 바꾸거나 읽는 시험은 데몬 `data_dir` 의 락 하나 · 셸 `discovery` 의 락 하나 아래에 있다(실 WMI `#[ignore]` 둘도 셸 락을 쥔다). 그 실 WMI 둘은 데몬 exe 를 시험 exe 위치에서 찾는다(운영 `locate_daemon_exe` 의 후보는 그대로).
5. **`send_stop` 의 2-2 ~ 3-2 자리 = 셸 `daemon_client/stop.rs` 글자 그대로** — 3-2(transport 셸 부착)가 다시 쓴다(ADR-0271 결정 5). 그 때문에 `pub(crate)` 로 연 discovery 내부(`check_acceptable` · `AcceptCheck` · `FileReader` 와 그 `path` · `RealLiveness`)는 discovery 밖에서는 stop.rs 만 쓴다. 셸에 `tungstenite` 직접 줄이 들지만 같은 판이 이미 lock 에 있어 새 패키지는 0. 옮긴 discovery · stop 코드의 로그 target 이 `engram_dashboard_discovery` → `engram_dashboard_lib::discovery` · `engram_dashboard_lib::daemon_client::stop` 으로 바뀐다(문구 · 레벨 그대로 · 그 target 을 쓰는 지시 0건).
6. **discovery 의 async 반입 게이트는 지우지 않고 platform 게이트 ⑦ 로 바꾼다** — 대상 = agent(`-e normal` · 동기 소비자 — base · command · platform 을 함께 덮는다) 0줄 · net 기본 feature 0줄(net `default = []` 의 성질) · 짝 = net `--features server` 1줄 이상(ADR-0278 꼴 — 패턴이 깨지면 짝이 빨개진다). 지우기만 하는 안을 버린 이유 = 그 게이트가 덤으로 지키던 platform 의 async 금지(동기 소비자 agent 가 남는다)와 net 기본 feature 축에 대상이 남는다. protocol 은 그 덮임에서 빠진다 — 2-2 뒤 소비자(데몬 · 셸 · net `server`)가 전부 async 다. 정본 = `ci.yml` 의 그 스텝 · platform `src/lib.rs` 헤더 게이트 ⑦. ★이 결정이 ADR-0271 의 두 문장을 고친다★ — 「거부한 대안」 첫 항목의 「나눈 뒤 지킬 대상이 없다」와 「영향」의 「사라지는 게이트 — `Gate: discovery has no async-runtime ingress`」(그 ADR 상태줄의 부분 폐기 도장은 결정 2 몫만 적혀 있다 — 같은 두 ADR 사이에 조항을 더하는 길이 서기 스크립트에 없다).

## 거부한 대안
- TODO 대안 A — 왜 버렸나.
- TODO 대안 B — 왜 버렸나.

## 근거
TODO — 실측·리뷰 등 결정의 뒷받침.

## 영향 / 불변식
TODO — 이 결정이 묶는 코드·게이트. 어기면 무엇이 깨지나.
