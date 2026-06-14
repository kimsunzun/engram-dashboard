# spike #1 결과 — Windows Job Object breakaway 실측

날짜: 2026-06-14. 코드: `src-tauri/examples/spike_breakaway.rs`(보존). 판정: **GO (단 spawn = WMI Win32_Process.Create 경유).**

## 측정 환경
이 spike 가 도는 셸(Claude Code CLI/wezterm)의 부모 Job:
- `parent_in_job = true`
- `parent_job_limit_flags = 0x2000` = **KILL_ON_JOB_CLOSE only, BREAKAWAY_OK 없음** = worst-case.
- 즉 "부모 Job 이 자식을 KILL_ON_JOB_CLOSE 로 묶고 breakaway 도 불허" — 설계 §4 가 우려한 바로 그 환경.

## 결과 (worst-case Job 기준)
| spawn 방식 | 결과 | 증거 |
|---|---|---|
| `CREATE_BREAKAWAY_FROM_JOB \| DETACHED_PROCESS` 직접 | ❌ 실패 | `os error 5`(액세스 거부) — breakaway_ok 없는 Job 은 거부 |
| `cmd /c start /b` fallback | ❌ 실패 | selfkill 후 marker 에 `ALIVE`만, `SURVIVED` 없음(동반 사망) |
| **WMI `Win32_Process.Create`** | ✅ **성공** | `RV=0`, `checkjob in_job=false`, `ALIVE` 기록 — WmiPrvSE 가 부모라 호출자 Job 미상속 |

`in_job=false` = KILL_ON_JOB_CLOSE 의 영향권 밖 → 부모(IDE/셸) 종료에도 데몬 생존(논리적 확정 + ALIVE 실측).

## 결론 / 설계 반영
1. **표준 breakaway(CREATE_BREAKAWAY_FROM_JOB)와 start/b fallback 은 worst-case Job(IDE 통합터미널·일부 CLI 래퍼)에서 무력.** 설계 §4 의 fallback 가정 폐기.
2. **데몬 spawn = WMI `Win32_Process.Create` 채택**(WmiPrvSE 경유 = 호출자 Job 미상속, 가장 robust). winmgmt 서비스 의존(상시 가동, 비활성 가능성 낮음).
3. **적응형 spawn 전략(권장):**
   - 부모 Job flags 조회(`QueryInformationJobObject(None)`) →
     - KILL_ON_JOB_CLOSE 아님 → normal spawn(상속돼도 안전).
     - BREAKAWAY_OK 있음 → `CREATE_BREAKAWAY_FROM_JOB`(가벼움).
     - worst-case(KILL_ON_JOB_CLOSE + breakaway 불허) → **WMI Create**.
   - 단순화 대안: 항상 WMI Create(분기 제거, 환경 무관 일관). 1차 권장.
4. **파생 제약(중요): WMI Create 는 환경변수 주입 불가**(CommandLine 문자열만). → 토큰 전달은 설계 §5 의 **ACL port.json 방식으로 강제**(env 경로 폐기). daemon-design §1-4/§5 갱신.
5. WMI Create 는 stdout/stdin 핸들 미반환 — 데몬은 WS 통신이라 무관(PTY child 는 데몬이 직접 openpty).

## 미검증(후속, phase 2 하네스로 흡수)
- 실제 배포(explorer 더블클릭) 환경의 부모 Job flags — 보통 Job 없음/breakaway 허용이라 더 쉬움. worst-case 가 통과했으므로 best-case 는 자동 통과 예상.
- WMI Create 호출을 Rust 에서 직접(COM IWbemServices, `wmi` crate) vs PowerShell 경유 — phase 2 구현 시 결정. 1차는 COM 직접(PowerShell 의존 제거).
- 데몬이 PTY child 를 자기 KILL_ON_JOB_CLOSE Job 에 담아 데몬 crash 시 동반 정리 — 기존 `platform/windows.rs` 그대로 재사용(이미 검증됨).
