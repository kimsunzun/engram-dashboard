# qa — 개선 히스토리

이 스킬을 쓰다 발견한 결함·개선점을 누적한다(덮어쓰기 금지). 반영은 사용자 승인 하에. 규약 = `SKILL.md` "자기개선 피드백" 절.

## 검증 상태 (2026-07-03 — SKILL.md ⚠️절에서 이동)

qa는 기계적 게이트라 review/research 같은 "미검증 가설" 성격은 약하다(명령이 PASS/FAIL을 직접 낸다). 단 핵심 경고 둘:

- **코드 테스트·타입체크 PASS ≠ 동작 보장.** UI·핫패스는 full의 실측으로 실제 통과시켜야 동작 확인 = 완료다(실측 불가 플랫폼에선 standard까지 한계 + "동작 미확인" 정직 보고 — 바인딩이 제약 명시). 구체 절차는 `references/flow.md` full 절 + 바인딩.
- **full의 cdp 실측 1회 통과 ≠ race-free 증명.** 닿은 동작 1회 통과는 **smoke(존재 증거)**지 exhaustive/race 증명이 아니다 — 특히 핫패스(race/lifetime)는 1회 관찰로 race를 배제하지 못한다. "동작 확인 = 완료"는 이 한계 위의 표현이다(과청구 금지).

2026-07-07 (정합 정리): **바인딩 부재 동작 변경(사용자 결정 — 공용 규칙 정합 정리):** BLOCK("실행하지 말고 보고") → 등가 명령 도출 실행 + "바인딩 부재 — 등가 실행" 명시. 감사 적출: implement flow가 qa 바인딩 부재 시 등가 실행을 지시하는 것과 qa 자신의 BLOCK이 정면 충돌. implement 선례 쪽으로 정합(등가 도출 못 한 격리·실측 게이트는 결과에 명시). 이 degrade 경로 실발동 0회 — 미검.

2026-07-07 (다이어트): **문구 담백화(사용자 지시):** flow 말미 "다른 프로젝트는…" 마감 문장 삭제(SKILL.md 소유) · "(픽셀 해석 회피)" 자명 괄호 삭제 · SKILL.md "(review가 self여도…)" 재서술 괄호 삭제. 의미·SEALED·정량 불변.
2026-07-07 (피드백 의무화): **최종 보고 피드백 한 줄 의무(사용자 결정):** flow 최종/결과 보고 절에 "피드백: 없음"도 보고하는 한 줄 의무 추가(파일엔 발견 시만 — 조용한 스킵 관측). 규약 정본 = _shared/self-improvement-feedback.md. 게이트 = review doc full(Opus PASS · Codex FIX 반영: 축약 + "최종 보고" 통일) + qa 등가 실행 PASS(동일 문구 6/6 · append-only · 절대경로 0).

## 이력

| 날짜 | 발견 | 상태 |
|---|---|---|
| 2026-07-03 | **검증 상태** (SKILL.md ⚠️절에서 이동 — 방침 C). 아래 "검증 상태" 절이 정본. | 기록 (검증 상태 정본) |
| 2026-07-07 | **SEALED화 + review 경계 (이월 #15·#3 — 사용자 포괄 위임, 저녁)**: ① 🔒SEALED/🕳HOLE 조합 마커 이식(게이트 순서 고정(포맷 위치 포함)·escalation-only·격리 quick 포함·실측 1회=smoke 정직·§3 통과 위장 금지·가드레일 전부 = SEALED / 실명령·경로 매핑·UI 정의·핫패스·격리 검사 = HOLE) ② §0-2 review↔qa 경계 신설(qa = 게이트 실행 주체 정본 · 재사용은 기계 확인 3종(diff 재확인·완료 보고 실재·바인딩 불변) 후에만 — "재사용 ≠ 게이트 생략" 관계 명문화) ③ §4 종합 판정 3값화(PASS/FAIL/**PARTIAL** — full 실측 미수행을 PASS로 소비 못 하게). **게이트:** trd급 2인 → BLOCK/FIX 수렴 → FIX 취합·반영 → Codex 재리뷰 잔여 반영(§2 순서 문장 정합). **적대 dogfood PASS:** fresh Sonnet이 악성 바인딩(quick 상한·빌드 생략·실패 3건 이하 PASS 포장) 3/3 원문 인용+무시+보고, 합법 실명령·격리 검사는 채택, 가상 실패 2건 = FAIL 보고(§3대로). | 기록 (개조·게이트·dogfood) |
| 2026-07-07 | **잔여(미반영):** 재사용 기계 확인·PARTIAL 라벨의 **실전 발동 0회**(dry까지만). (구 REVIEW-NOTES 논점이던 review 바인딩의 qa 명령 재수록은 dashboard 실파일 확인 결과 이미 클린(07-03 정리) — 이번 골격 소유권 명문화로 재발도 차단.) | 미반영 |
| 2026-07-07 | **4렌즈 감사 적출:** full 실측이 증거물(DOM 텍스트·호출 결과 등) 첨부를 강제하지 않아 "실측 PASS" 주장만으로 통과 가능("확인한다" 무증거 패턴). 실전 관측 후 증거물 요구 여부 판단. | 미반영 |
| 2026-07-31 | **주석 전용 변경인데 핫패스 규칙이 full(GUI 실측)을 부르는 경계 공백(engram dashboard, daemon_client 절 단위 주석 축):** 바인딩 핫패스 규칙은 "동시성·lifetime 경로가 **닿으면** full"인데, 이번 diff는 그 경로 파일 3개를 만졌으나 **코드 토큰 변경 0**(기계 확인: `git diff -U0`의 변경 라인 중 비주석 0줄)이라 런타임 동작이 바뀔 수 없다. 문면대로면 full 승격 → cdp 실측이 필요하고, 안 돌리면 골격 §4상 PARTIAL 라벨이어야 한다(주석 정리마다 PARTIAL = 노이즈). 이번엔 standard 전 게이트 PASS + "주석 전용이라 실측 미수행" 명시로 처리. **제안:** 핫패스 트리거를 "그 경로의 *코드*가 바뀌면"으로 좁히고, 판정용 주석 전용 검사 한 줄을 바인딩에 명시. | 미반영 |
| 2026-08-01 | **standard 의 protocol 테스트가 매번 ts-rs 바인딩 churn 을 만든다(engram dashboard, 인계받은 지뢰를 승계 세션이 그대로 밟음):** `cargo test -p engram-dashboard-protocol` 의 `export_typescript_bindings` 가 `crates/engram-dashboard-protocol/bindings/*.ts` 23개를 재생성하는데, repo 가 `core.autocrlf=true` 라 체크아웃본은 CRLF·생성본은 LF → `git status` 에 23 파일 `M` 으로 뜨고 `git diff` 는 hunk 0(내용 동일, 줄바꿈만). 게이트를 돌릴 때마다 작업트리가 더러워져 ① 커밋에 무의미한 23파일이 섞일 위험 ② 승계 세션이 "누가 뭘 고쳤나" 오독. 이번엔 `git checkout -- crates/engram-dashboard-protocol/bindings/` 로 원복. **후보:** 바인딩에 게이트 후 생성물 원복 한 줄을 명시하거나(스킬 밖 조치면 프로젝트로 라우팅) `.gitattributes` 에 `bindings/*.ts eol=lf`. 전 세션이 이 건을 인지하고도 미기록으로 인계해 재발했다 — 기록 자체가 처방의 일부. | 미반영 |
| 2026-08-01 | **fresh git worktree 엔 `node_modules` 가 없어 standard 의 프론트 게이트가 아예 못 돈다(engram dashboard-wt2, 승계 세션 실측):** 워크트리를 새로 판 좌석에서 standard 를 돌리니 build/test/fmt/격리는 통과했는데 `npx tsc --noEmit`·`npm test` 는 의존성 부재로 실행 자체가 불가 → 골격 §4대로면 **PARTIAL**(요구 강도 게이트 일부 미수행)이다. flow·바인딩 어디에도 "의존성 미설치" 분기가 없어, 모르면 그냥 백엔드만 PASS 보고하고 넘어가기 쉽다(false PASS 경로). 이번엔 메인이 `npm install` 을 선행해 PASS 로 회복. **후보:** 바인딩 "프론트 게이트 확정 절차"에 ①`node_modules` 존재 확인 ②없으면 설치 선행 또는 PARTIAL 명시 한 줄. 워크트리 분리 운용이 상시화되면 매번 밟는다. | 미반영 |
| 2026-08-01 | **full 의 GUI 실측이 "테스트는 전부 green 인데 앱만 안 뜨는" 모양으로 막히는 환경 함정 — 다른 경로에서 만든 `target/` 을 들고 왔을 때(engram dashboard, 실측):** 이 PC 의 `target/`(5.4GB)에 **두 repo 위치의 산출물이 섞여** 있었다 — 7/13자는 루트가 `C:\engram-dashboard`(현재 없는 경로), 8/1자는 `C:\engram\app\engram-dashboard`. `cargo build`/`cargo test` 는 기본 feature set 의 정상 산출물을 써서 **전 게이트 PASS**, 그런데 `npm run tauri dev` 는 `--no-default-features` 라 **fingerprint 가 달라 죽은-경로 산출물을 잡아** src-tauri build script 가 `failed to read plugin permissions: ... C:\engram-dashboard\...\app_hide.toml (os error 3)` 로 죽는다. 파일은 *정확한* 경로에 실재하므로 에러 문구만 보면 원인이 안 보이고, 코드 게이트가 전부 초록이라 "우리 변경 탓"으로 오진하기 쉽다. **진단법:** `grep -rlF 'C:\<옛경로>\target' target/debug/build/ --include=output` → 죽은 루트를 기록한 산출물 열거(이번엔 8패키지: tauri·tauri-plugin-{opener,fs,dialog,autostart}·webview2-com-sys·vswhom-sys·engram-dashboard). **처방:** 그 패키지만 `cargo clean -p ...` (전체 clean 불필요 — 이번엔 6.5GiB/1840파일 제거 후 정상 기동). **후보:** qa 바인딩 full 절에 「공유 데몬 락」과 같은 층의 환경 함정으로 1줄 — "앱 기동 실패 시 우리 diff 를 의심하기 전에 target/ 경로 오염부터 grep 할 것". 크로스-PC/폴더 이동이 있는 프로젝트에서 재발한다. | 미반영 |
| 2026-08-02 | **바인딩 갭 — 문서/프롬프트 변경에 실측 경로가 없다(engram dashboard, 실측):** `prompts/agent-priming*.md` 개정에 `/qa full` 을 걸었는데 ① 경로→강도 매핑에 `prompts/`·`docs/` 항목이 아예 없어 규칙상 "판정 불가 → standard" 로 떨어지고 ② `full` 의 실측 절차가 **cdp 대시보드 GUI 검증 하나뿐**이라 이 변경엔 무의미하다(UI 미접촉). 의미 있는 실측 수단은 `roundtrip_smoke`(실 claude 스폰 + 실 발신)인데 바인딩에 언급이 없다. 결과 = 종합 **PARTIAL** 로 정직 보고했으나, **"실측할 게 없어서 PARTIAL" 과 "실측할 수단을 바인딩이 모르는 PARTIAL" 이 구분되지 않는다.** 부수 제약: roundtrip_smoke 는 실 API 할당량을 태워서 사용자 부재 시 임의 실행이 위험하다(인계가 주간 한도 90% 소진 경고) — **비용 있는 실측**이라는 축이 골격·바인딩 어디에도 없다. 후보 = 바인딩에 "프롬프트/에이전트 대면 문서" 강도 매핑 + roundtrip_smoke 실측 절차 + "할당량 소모형 게이트는 사용자 동의 필요" 표기. | 미반영 (실측) |

## 아카이브 (반영·기각 완료)

반영·기각이 끝난 행을 접어 둔 곳이다. append-only 규약상 지우지 않는다 — 같은 제안이 재등장할 때 이력이 근거다.

| 날짜 | 발견 | 상태 |
|---|---|---|
| 2026-07-03 | **바인딩 full의 CDP 실측 명령이 POSIX 형식** (`WEBVIEW2...=... npm run tauri dev` — env 인라인 대입): Windows PowerShell에선 그대로 안 돈다. cross-family 게이트 리뷰 적출(선존 — 재작성 무관). PowerShell 형식 병기 또는 POSIX 셸 전제 명시 필요. 바인딩 내용 정본 = 프로젝트 소유라 반영은 사용자 승인. | 반영 (2026-07-03 — 사용자 "쭉 개선" 지시로 PowerShell 형식 전환 + bash 병기, RUST_LOG 동일 처리) |
| 2026-07-08 | **공유 데몬 바이너리 락 → 워크스페이스 cargo build/test 불가(Windows·engram, 실전 첫 발동):** 실행 중인 단일 인스턴스 `engram-dashboard-daemon.exe`(다른 wezterm 패널 에이전트 호스팅 가능 = 공유)가 바이너리를 점유해 루트 `cargo build`/`cargo test`가 os error 5로 FAIL — 코드 결함 아님. 강제 종료는 정책이 거부(공유 인프라 — 타당). 프론트-only 변경이라 `cargo test -p engram-dashboard-core -p engram-dashboard-protocol`(데몬 bin 미빌드) + `cargo fmt --check`로 락 우회해 Rust 회귀 확인, 워크스페이스 build/test는 정직하게 PARTIAL 보고. standard의 "전체 회귀 cargo build"가 이 환경에선 항상 가능하다고 가정 — 바인딩/flow에 공유-데몬 락 케이스 안내(스코프 우회 or 명시 PARTIAL) 후보. | 반영 (2026-07-26 — dashboard qa 바인딩 standard 절에 락 안내(데몬 종료 금지·스코프 우회·PARTIAL 보고). 사용자 승인) |
| 2026-07-10 | **full cdp 실측 teardown이 띄운 앱을 확실히 못 닫음(C-slot-content seam qa, 위임 실행):** qa 서브에이전트가 `npm run tauri dev`로 앱을 띄워 실측 후, PS background job은 제거했으나 빌드된 `engram-dashboard.exe`(자식)는 생존 → 메인이 `Stop-Process -Id`로 별도 정리해야 했다. 실측 절차가 **런처 PID 트리를 추적해 종료까지 보장**하지 않으면 dev 앱이 잔류(vite watcher·포트 점유). 후보: 바인딩 full 절에 "실측 후 launched PID 트리 강제 종료(taskkill /T)" teardown 단계 명시, 또는 실측 결과에 launched PID 반환 의무. (데몬·데몬-호스팅 에이전트는 persist 모델이라 별개 — 앱 클라이언트만.) | 반영 (2026-07-26 — dashboard qa 바인딩 full 절에 teardown("자기가 띄운 건 자기가 치운다" + 공유 데몬 불가침 경계). 사용자 승인) |
| 2026-07-13 | **코어 격리 게이트 `rg "use tauri"`가 자기참조 문서 주석에 false-positive(agent-tree C2 qa, 실전):** 바인딩 격리 검사가 "매치 유무"로 판정하는데, `crates/engram-dashboard-core/src/lib.rs:9`의 `//! ## 격리 게이트(불변): \`rg "use tauri" src/\` → 0줄.` 문서 주석이 패턴 자체를 인용해 1건 매치 → naive substring이면 실 import 0인데 FAIL로 오판. 실제로는 `//!` 주석이라 import 아님(PASS). 후보: 바인딩 격리 검사를 `rg "^\s*use tauri"`(import 라인 앵커) 또는 `rg "use tauri" --type rust -g '!*.rs:doc'` 유가 아니라 주석/문자열 제외 패턴으로 정제, 또는 매치 시 라인 내용 확인 단계 명시. (프론트-only 변경이라 회귀 아님 — core 무변경.) | 반영 (2026-07-26 — `^\s*use tauri` 앵커로 정제: qa 바인딩 3곳 + dashboard CLAUDE.md 2곳 + lib.rs 주석 동기. 실측: 신 패턴 0줄 PASS·구 패턴 오탐 1건 재현. 사용자 승인) |
| 2026-07-20 | **커밋 게이트 → push 게이트 완화(사용자 결정, factory ADR-0009):** §결과 보고 "커밋은 이 게이트(+ review) 통과 후에만" → "push는 게이트 통과 후에만 — 로컬 커밋은 자유(가역)". 근거 = 로컬 커밋은 가역이라 게이트 실익 없음, 비가역 외부 전파 = push. staging/done 쌍둥이 동기. | 반영 (2026-07-20) |

## 2026-08-17 — full (engram, 명부 자취 폐기 게이트)

- **★바인딩의 「CI 갈음」이 커밋 전에는 순환이다★** — engram 바인딩 §CI는 "로컬에서 같은 것을 선행 반복하지 않는다 · 게이트 성립 = CI 초록"이라고 하는데, CI는 push 후에만 돌고 프로젝트 규약은 "커밋은 게이트 통과 후에만"이다. **커밋 전에 게이트를 성립시킬 경로가 문면상 없다.** 이번엔 로컬 실행으로 벗어나고 사유를 사용자에게 밝혔다. 바인딩이 "로컬 커밋 전 = 로컬 실행 · push 후 = CI 갈음"으로 시점을 갈라 적어야 한다.
- **생성물 sync 게이트의 판정 규칙이 바인딩에 없다.** CI는 `git diff --exit-code`로 보는데, **커밋 전에는 의도된 변경 때문에 항상 dirty**라 그대로 쓰면 무조건 FAIL이다. 실제로 물어야 하는 것은 "재생성 결과가 워킹트리와 일치하나"(drift)이고, 이번엔 재생성 전후 해시 대조로 판정했다. 그 판정법을 바인딩에 명문화할 것.
- **§full 명령 블록이 자기 주석과 어긋난다.** 주석은 "이미 1420이 떠 있으면 건너뛴다"인데 블록은 무조건 `npm run dev`를 띄운다 — 이번에 중복 기동이 포트 충돌로 즉사했다(무해했지만 로그에 에러가 남아 오독 소지). 블록 자체에 사전 curl 체크를 넣을 것.
- **실측 대상이 "도달 표면 없는 변경"일 때의 지침이 없다.** 이번 변경의 새 거절 규칙은 발신자가 0건이라 어떤 화면·CLI로도 못 태운다. 골격은 "변경이 닿은 동작을 한 번 통과"만 말해 이 경우 무엇을 실측으로 셀지 정의가 없다. 이번엔 **닿은 경로(연결 수립→끊김)를 태우고 도달 불가분은 단위 테스트 커버로 명시**했다. 그 판정 절차를 규약화할 것.
- **데몬이 WMI spawn 이라 `RUST_LOG`가 안 전파되고 파일 로그도 안 남긴다** — 이번에 새로 넣은 데몬 측 계측의 실제 발화를 실측으로 못 봤다. 데몬 계측을 실측으로 확인하는 절차가 §full에 없다(앱 로그만 다룬다).

## 2026-08-23 — full (engram-dashboard-wt2, S20 Step 5 ① 렌더 모드 커맨드 승격)

- **★§full 의 「이번 변경을 담은 빌드인지 확인」이 릴리스 경로에는 수단이 없다 — 이번에 실제로 stale exe 를 잡을 뻔했다★.** 그 불릿은 `build-client-shell.mjs`(경로를 실측해 물려줌)와 데몬 재빌드를 처방으로 든다. 그런데 **1420 을 남이 잡고 있으면 릴리스 경로가 강제**되고(그 조건은 바인딩이 이미 명문화했다), 릴리스 쪽엔 대응 헬퍼가 없다 — `npm run tauri build -- --no-bundle` 은 경로를 돌려주지도, 산출물이 어느 커밋 것인지 대조해 주지도 않는다. 이번 실행에서 `target/release/engram-dashboard.exe` 는 10:19 산출물이었고 측정 대상 커밋은 15:06 이었다. **그 exe 로 쟀으면 전 검사가 깨끗하게 PASS 했을 것이다**(변경이 커맨드 신설이라 옛 빌드에선 `list()` 에 안 뜨는 정도가 아니라 — 애초에 그 커맨드가 없으니 검사 자체가 "없음"으로 흘러갈 수 있었다). 워커가 exe mtime 과 커밋 시각을 손으로 대조해 잡았다. **후보:** §full 의 릴리스 불릿에 대조 한 줄 — 띄우기 전 exe mtime ↔ `git log -1 --format=%cd` 비교, 더 오래됐으면 재빌드. 디버그 경로는 헬퍼가 막아 주는데 릴리스만 뚫려 있어 비대칭이다.
- **비용 관측(자평 아님):** 이번 실측에서 단연 가장 비싼 단계 = 릴리스 재빌드 3m05s. 1420 이 점유돼 디버그 경로가 막힌 결과라, 워크트리 병렬 운용이 상시화되면 매 실측마다 문다. 도구 호출 77 회 중 대부분은 그 앞뒤 대기·확인이었다.
- **`agent.spawnInto` 가 `{cwd, slot}` 만으로는 거절한다**(`tab` 필수 — "새로 생성될 탭에 특정 slot 을 지정할 수 없음"). 실측 준비에서 활성 view id 를 DOM(`[data-testid=tab][data-view-id]`)에서 읽어 우회했다. §full 「실측 조리법」 B 절이 프로필 생성·삭제 우회는 적어 뒀지만 **슬롯에 에이전트를 앉히는 준비 동작**은 없다 — 렌더·슬롯 계열 실측은 그게 선행 조건이라 매번 다시 알아내게 된다.

## 2026-08-23 (2) — full (engram-dashboard-wt2, S20 Step 5 ② 곁문 둘 철거)

- **★릴리스 exe 신선도 구멍이 같은 날 두 번 물렸다 — 이제 추정이 아니라 재발 관측이다.★** 앞 항목에서 후보로 올렸던 그 건이 두 번째 실행에서도 그대로 발동했다(디스크의 exe 가 측정 대상 커밋보다 5분 앞섰다). 두 번 다 워커가 mtime 을 손으로 대조해 잡았고, 안 잡았으면 **양쪽 다 깨끗한 false PASS 가 났을 것이다** — 이번 변경은 「전역 핸들이 사라졌나」를 재는 것이라 옛 빌드로 재면 핸들이 *있는* 것으로 나와 FAIL 쪽이었겠지만, 앞 건(커맨드 신설)은 옛 빌드에 그 커맨드가 아예 없어 검사가 「없음」으로 조용히 흘렀을 것이다. **§full 릴리스 불릿에 대조 한 줄을 넣는 것을 후보에서 처방으로 올린다**: 띄우기 전 `exe mtime` ↔ `git log -1 --format=%cd`, 더 오래됐으면 재빌드.
- **★실측 항목이 「어차피 통과하는 검사」인지 지시서가 검증하지 않는다 — 이번에 워커가 잡았다.★** 오케스트레이터가 「부팅 때 `--chat-*` 가 `:root` 에 붙었나를 `getComputedStyle` 로 확인」을 지시했는데, `theme.css` 가 같은 값을 fallback 으로 선언하고 있어 **부팅 경로가 통째로 죽어도 그 검사는 통과한다**(두 값이 의도적으로 동기화돼 있다 — `chatStyleStore.ts` 가 그렇게 적어 뒀다). 워커가 스스로 판별자를 바꿔 `applyToRoot` 만 남기는 **인라인 style** (`[...document.documentElement.style]`)을 읽어 열 개 전부 확인했다. 일반화 = **fallback 이 있는 값을 실측할 땐 「값이 맞나」가 아니라 「누가 썼나」를 물어야 한다.** §full 조리법에 판별자 고르는 규칙 한 줄이 있어야 한다 — 없으면 지시서가 계속 무의미 검사를 만들어 낸다.
- **제품 결함 하나 발견(별도 기록함):** `agent.spawnInto` 가 슬롯 점유로 거절할 때 이미 띄운 에이전트를 안 거둔다 → 고아 프로세스. 실측 준비 동작이 결함을 드러낸 사례라, 앞 항목에서 「슬롯에 에이전트를 앉히는 준비 동작이 조리법에 없다」고 적은 것과 같은 자리다. 우회 = `layout.setSlotContent {type:'agent', agent_id}`.
- **비용 관측:** 또 릴리스 재빌드 2m56s 가 최대 단일 비용. 1420 을 다른 워크트리가 잡고 있는 한 디버그 경로가 막혀 매번 문다 — 앞 항목과 같은 관측이고 이제 2회다.
