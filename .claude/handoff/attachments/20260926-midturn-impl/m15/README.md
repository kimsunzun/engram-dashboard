# M15 측정 도구

★**전제: 제품의 steer 는 켜져 있다**(P2e2b 에서 `STEER_ENABLED` 상수를 걷었다 — 하한 판정 `Above` 인 화신이면 steer 한다). 하한 미달 codex(0.140 미만)로 돌리면 steer 줄이 0건이라 판정은 INCOMPLETE 로 나온다.★

★**`lag_us` 는 P2g 에서 뜻이 바뀌었다(칸 이름은 그대로)**★ — 그 전 로그는 리더가 `read()` 안에서 codex 를 기다린 시간(턴 사이 · 모델 사고 — 최대 101 초 관측)까지 밀림으로 셌다(M15 A단계 결함). P2g 이전 빌드의 로그로 판정하지 말 것.

## M15 가 재는 것

codex 통로가 턴 안 경계(도구 끝)에서 쥔 입력을 넘기는 반응 지연이다. 한 번에 도는 경로가 없어서 세 항의 최댓값을 더해 판정한다(TRD `trd-mid-turn-input-queue.md` L161).

- **리더 밀림**(`phase="signal"` 의 `lag_us`): 리더가 완료 줄을 집기 전에 그 줄이 파이프에서 기다렸을 수 있는 시간의 상한 — 파이프가 비어 있었다고 아는 마지막 순간(짧은 읽기)부터 리더가 `read()` **밖에서** 바빴던 시간만 센다. 한가한 뒤 첫 줄은 대개 앞 청크 꼬리 처리 시간만큼(0 이 아니다 — 오차 정의는 `transport.rs` 의 `DrainMark` doc).
- **① 리더 신호 → 라이터 깸**(`phase="wake"` 의 `wake_us`): 짝은 `seq` 로 짓는다. `coalesced=N` 이면 `seq+1..seq+N` 신호를 그 깨어남 하나가 덮은 것이다.
- **② 라이터 깸 → steer 쓰기 끝**(`phase="steer"` 의 `steer_us`): `hop="signal"`(턴 id 홉 — `seq` 로 `wake` 와 짝)과 `hop="announce"`(출력 폭주 중 곧바로 넘기는 홉), `hop="zone"`(P8 — 쥐던 답 구간이 신호 없이 풀린 홉: 답 뒤에 그 밖 항목이 시작된 줄. `wake` 줄이 없고 `hop_us` 는 그 줄을 집은 순간부터 잰다)을 따로 집계하고, 판정에는 셋을 합친 최댓값을 쓴다. P8 이전 로그엔 `zone` 이 없을 뿐 그대로 읽힌다.

**판정: 도구 끝 `max(①) + max(리더 밀림) + max(②) < 6 ms` 이면 녹색이다.** 답 끝(`answer_end`)도 같은 식으로 더하지만 판정이 아니라 M16 창과 견줄 값이다. 칸 계약의 정본은 `crates/engram-dashboard-agent/src/backend/codex/transport.rs` 에서 `HANDOVER_TRACE` 에 달린 문서 주석이다. 칸 이름이 바뀌면 파서의 `INT_FIELDS` 와 집계도 같이 고친다.

## 파일

- `parse-handover-trace.mjs`: 데몬 로그 파서. Node 내장 모듈만 쓴다.
- `fixture-sample.log`: 합성 로그(seq 6개, 합쳐진 깨어남 1건, 신호보다 먼저 찍힌 wake 1건, span 접두 1줄, announce 홉 2건, 짝 없는 신호 1건)
- `selftest.mjs`: 위 fixture 와 ANSI 섞인 줄, FAIL·INCOMPLETE 경로, `hop="zone"` 집계를 단언한다. `node selftest.mjs` → `selftest OK`(실측 2026-09-26).

## 파서 실행

```bash
node .claude/handoff/attachments/20260926-midturn-impl/m15/parse-handover-trace.mjs <데몬 로그> [--json] [--extract <계측줄만.log>]
```

- 종료코드는 0 = PASS, 1 = FAIL, 2 = INCOMPLETE(판정 항 중 표본이 0인 것이 있음), 3 = 사용법·읽기 오류다.
- 출력 내용
  - 신호 종류별 건수(`tool_end` / `answer_end` / `token_usage` / `turn_id`)
  - 항별 count / min / p50 / p95 / max(µs)
  - 판정과 지배 항
  - 짝 없는 줄
  - 경고: 표본이 20 미만인 항, 한 로그에 통로가 여럿 섞였을 때의 `seq` 중복
- `--extract` 는 계측 줄만 따로 떼어 쓴다. 원시 로그를 `.claude/handoff/attachments/` 에 남길 때 이 산출물을 쓴다(계획 §5 의 「scrubbed raw logs」).
- 로그 형식은 tracing-subscriber 0.3.23 기본 `fmt` 형식에 맞췄다: `<UTC 마이크로초> <레벨> [span…: ]engram::codex_handover: <메시지> k=v …`. 설정은 `crates/engram-dashboard-base/src/logging/mod.rs` 의 `init_subscriber`(파일 sink 는 ANSI OFF)에 있다. stdout 사본에 섞인 ANSI 색 코드는 파서가 벗긴다.

## 측정 절차 초안

★**데몬에 `RUST_LOG` 를 넘기는 길이 앱 경유로는 없다.**★ 앱이 WMI 로 띄운 데몬은 부모 환경을 물려받지 않는다. 근거는 `logging/mod.rs` 머리 주석과 `crates/engram-dashboard-discovery/src/lib.rs` 의 `ENGRAM_DATA_DIR` 주석 두 곳이다. 앱에 `-EnvVars 'RUST_LOG=…'` 를 줘도 데몬에는 닿지 않는다. 런타임에 레벨을 바꾸는 명령도 없다(`set_log_level` 을 부르는 곳이 코드에 0건이다). 그래서 **데몬을 먼저 직접 띄우고, 같은 데이터 폴더를 가리키는 앱이 그 데몬에 붙게 한다.**

1. **빌드**(분리 실행 — `.claude/skill-bindings/qa.md` 「분리 실행」·§full): `cargo build -p engram-dashboard-daemon` 과 `node scripts/build-client-shell.mjs`. 이 워크트리의 다른 debug 데몬이 떠 있으면 데몬 빌드가 os error 5 로 실패한다(§full).
2. **격리 데이터 폴더를 정한다.** 예: 스크래치 폴더 아래 `m15-data`. `ENGRAM_DATA_DIR` 이 곧 단일 인스턴스 범위라서 운영 `.engram-data` 의 데몬과 섞이지 않는다.
3. **데몬 기동**(Git Bash, 경로는 슬래시로 쓴다):
   ```bash
   powershell -NoProfile -Command "& './scripts/launch-detached.ps1' -Exe '<target-dir>/debug/engram-dashboard-daemon.exe' -EnvVars 'RUST_LOG=warn,engram::codex_handover=debug','ENGRAM_DATA_DIR=<격리 폴더>'"
   ```
   `-File` 이 아니라 `-Command` 로 부르고, 값마다 작은따옴표를 따로 붙인다. 그렇지 않으면 값이 하나로 뭉개진다(§full). 출력의 `PID=` 는 teardown 에 쓰니 적어 둔다.
4. **앱 기동**: §full 1) 과 같게 하되 `-EnvVars` 에 `'ENGRAM_DATA_DIR=<같은 격리 폴더>'` 를 더한다.
5. **로그 위치**: 측정에 쓸 로그는 **`<격리 폴더>/logs/daemon-<UTC>-<pid>.log`**(파일 sink, ANSI 없음 — `logging/mod.rs` 의 `open_run_log`)이다. launch-detached 의 `LOG=` 파일에도 stdout 사본이 가지만 ANSI 가 섞일 수 있다(파서가 벗긴다).
6. **codex 에이전트는 하나만** 띄운다. `seq` 는 통로마다 0부터 다시 세서, 둘 이상이면 짝짓기가 모호해진다(파서가 경고한다).
7. **몰아 줄 것**(항마다 20건 이상):
   - **도구 끝 20건 이상**: 수천 줄을 찍는 명령(예: 5000줄 출력 루프, 긴 빌드·시험 출력)을 반드시 넣고, 병렬 도구 호출도 넣는다 → ① + 리더 밀림
   - **턴 id 홉 20건 이상**: 한 턴 도중 글을 두 개 이상 쌓아 턴 끝에 둘째부터 쥐게 하거나, 보낸 직후 턴 id 가 오기 전에 글을 친다 → `hop="signal"` steer
   - **출력 폭주 중 steer 20건 이상**: 명령이 출력을 쏟는 동안 글을 친다 → `hop="announce"` steer
   - 답 끝(`answer_end`)은 위를 도는 동안 저절로 쌓인다. M16 비교용이다.
8. **파서 실행** → 판정, 지배 항, 경고를 `docs/research/mid-turn-m15-m16-measurements-<date>.md` 에 옮긴다. 원시 로그는 `--extract` 산출물로 남긴다.
9. **teardown**: 앱과 데몬 모두 자기가 띄운 PID 로 치운다. 데몬은 앱의 `/T` 에 걸리지 않고, 죽이기 전에 `ExecutablePath` 를 확인한다(§full).

## 미검

- 3–4 단계 전체. 직접 띄운 데몬에 앱이 새로 띄우지 않고 그대로 붙는지 확인하지 않았다(버전·exe 대조로 거부할 수 있는지 모른다). debug 데몬은 콘솔 앱인데, launch-detached 로 띄울 때 창이 뜨는지도 확인하지 않았다.
- `RUST_LOG=warn,engram::codex_handover=debug` 가 데몬에서 실제로 계측 줄을 내는지. 실제 데몬 로그로 파서를 돌려 본 적이 없다. 형식은 tracing-subscriber 0.3.23 의 기본 `Full` 형식을 코드에서 추론한 것이다. 필드 순서(메시지가 앞인지)와 span 접두가 붙는지는 실물로 확인해야 하고, 파서는 순서와 span 에 무관하게 짰다.
- 7 단계에서 codex 에이전트를 띄우는 cdp 호출과, 턴 id 전에 글을 끼우는 조작법. 구체 명령은 적지 않았다.
- `vendor emit→pick` 은 벤더 프로세스와 데몬의 벽시계 차이다(ms, 참고용). 판정에는 쓰지 않는다.
