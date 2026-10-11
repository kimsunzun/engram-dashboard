# A TRD 2판(`1bc224ad`) 리뷰 1 라운드 — 결과와 메인 · 사용자 결정 (2026-10-10)

대상 = `docs/process/S21-crate-boundaries/trd-A-data-file-unification.md` 2판. `/review trd full` — codex(설계 렌즈 · blind) **BLOCK** · Claude(불변식 렌즈 · doc-aware) **FIX 13**. 판정 취합 = 재작업(3판) 후 재리뷰. 줄 번호는 `1bc224ad` 기준 — 쓰기 전에 다시 본다.

## 사용자 결정 (2026-10-10)

- **공용 코드 자리 = base**(「Base에 넣어야지. 나중에 쪼갤려면 base에서 쪼개야지.」) — 새 crate `engram-dashboard-datafile`(TRD 3-1 의 A안) 철회. 파일 규칙 전부(읽기 · 버전 판정 · 원자 쓰기 · 손상 사본 · 임시 쓸기 · 쓰기 정책)를 base `file` 입주자로. OS 에 따라 갈리는 잠김 판정 · 재시도는 platform `fs`(TRD U1 그대로)이고, base 는 platform 을 부르지 못하므로 **재시도를 호출부가 인자로 넘긴다**(TRD 가 「D」로 기각했던 모양 — 기각 사유를 다시 본다). 고칠 기록 = ADR-0269 지킬 것 6 의 「base 가 serde 를 안 끌어옴 · `file` 은 바이트만 받는다」(serde_json 을 base 에 들인다). 결정 1 · 7 · 9 와는 맞는다. 새 ADR 에 담는다.

## 메인 결정 (위임 「성숙한 프로그램 관행대로」)

- **저장 거절 = 읽기 전용 모드**(codex 2 · Claude 4 수렴) — agent · preset 저장소가 새 버전 · 읽기 실패로 거절 상태면 registry 의 변경 요청을 **적용 전에 오류로 돌린다**(SQLite 읽기 전용과 같은 모양) + 그 상태를 버스 · 명부에 실어 LLM 이 본다. 지금 설계(로그만 남기고 메모리 변경은 성공처럼 보임 → 재시작하면 사라짐)를 버린다. ReadFailed 가 부팅 시 일시 잠김(백신 등)으로 들어오는 경우도 같은 모드다 — 재시도 예산 안에서 못 풀리면.
- **저장 직전 재판정**(codex 1) — 쓰기 정책을 load 때 한 번 정해 두지 말고 저장 직전에 현재 파일을 다시 판정한다. 같은 데이터 폴더에 데몬은 하나(단일 인스턴스 가드)라 프로세스 간 잠금까지는 두지 않는다 — 그 판단을 TRD 에 적는다.

## 3판에 반영할 나머지 (Claude 리뷰)

1. §3-1 · §7 — base 로 바뀌었으니 ADR-0269 결정 1 · 6 · 7 · 9 행을 다시 쓴다(6 = serde 허용으로 개정 · 인용 「결정 2」→「결정 6」 정정). CLAUDE.md base 항목 입주자 수 · 입주자 무참조 정규식(`(logging|text|time|path|sync|testing|writable)` 네 사본)에 `file` 추가.
2. U6 순서 — mcp_config 의 `create_dir_all` 을 걷는 것은 R11(첫 쓰기가 폴더를 만든다)이 들어온 뒤라야 한다(지금 U6 ∥ U3 라 새 데이터 루트에서 에이전트 스폰이 실패한다 · `mcp_config.rs:112-114` · `data_dir.rs:53`).
3. U4 — `restore.status` 의 `state_file` 에 `newer` 를 더하면 LLM 이 읽는 표면도 함께: `src-tauri/src/layout/commands.rs:468-480`(명령 설명 · saves=false 설명) · 같은 파일 `catalog_version` 15 → 16(:91-92 규칙 · :99) · `prompts/engram-help.md:79-80` · `src-tauri/tests/layout_commands.rs:2072-2075` · `state/restore.rs:65-70`.
4. R10 폴더 fsync — `cfg` 없는 런타임 OS 가름이라 platform `fs::sync_dir` 로(Windows = 명시 무동작 등).
5. R11 서술 정정 — 부팅 쓰기는 이미 폴더를 만든다(`boot.rs:80-82,90-91`) · 새로 생기는 것은 기록기(saver)와 `copy_atomic` 이 실행 중 지워진 폴더를 루트까지 다시 만드는 것이다(`saver.rs` 의 「기록기는 만들지 않는다」 주석과 부딪힘) — 의도적으로 정한다.
6. 자식 띄우기 가드는 직속 자식만 죽인다 — Windows claude = `cmd.exe /c claude …` 라 손자가 남는다(`stdio.rs:102-104` · `codex/transport.rs:6395-6409` 시험도 cmd 만 본다). 남는 누수를 §3-7 · §4 · ADR 에 적고, 가입 실패 때 `process::kill_tree` 로 물러서는 안을 본다.
7. R12 의 `write_lock` 제거 — 락 순서 사슬 「래치 → expected 칸 → profiles → store write_lock」가 적힌 곳(`session_id_latch.rs:24-25` · `manager.rs:516` · `profile.rs:364-365` · `preset.rs:58-59` · ADR-0071:26)을 U5 에 넣고, 저장소가 새로 갖는 상태(쓰기 정책 · R15 첫 오류 표시)용 잎 락을 이름 붙인다.
8. 메시징 정규식 다섯째 사본 `docs/reference/architecture-overview.md:498`.
9. 시험 수 — fnv1a 시험은 codec 으로 가므로 셸 → 공용 이사 = 21(22 아님).
10. 옛 고정 임시 이름(`agents.json.tmp`) 쓸기 — `temp_owner` 가 번호 없는 이름을 거부한다(`fsutil.rs:208-215`) · 저장소가 직접 지우거나 `sweep_temps` 에 인자를 더한다(웨이브 4 전에 API 확정).
11. settings 의 `LoadNote::Newer` 변형(지금 `Unusable` 문구 「첫 쓰기가 옆에 떠 둔 뒤 새로 쓴다」가 Newer 에선 거짓).
12. 잔손질 — ChildGuard 줄(구조체 1406 · Drop 1414-1421) · 메모 줄(:334 → :336, `afc478d` 뒤) · U6 웨이브 표기 통일 · §2-2 state 폴더 만들기 서술 · ADR-0264 결정 2 그림(`.corrupt-*`)을 「확인」 행에.
