# 원자적 쓰기의 찌꺼기 임시 파일 — 언제 · 어떻게 치우나 (관행 조사 · light)

- **상태:** light · 2026-10-31 · 수집자 1(worker-scout) · 메인 grounding 스팟체크 2건(Syncthing · VS Code 원문 대조 — 지지). 적대 리뷰 없음(light).
- **계기:** S25 TRD — 프리셋 파일을 같은 폴더 임시 파일에 쓰고 바꿔치기(G38)할 때 꺼지면 남는 찌꺼기를 「데몬 기동 때 쓸기」로 적었는데, 사용자 결정 「데몬이 켜질 때 미리 훑지 않는다」(2026-10-17)와 부딪침(TRD §1-11a 발견 15).
- **확신도 범례:** 확실 = 1차 출처(소스 · 공식 문서) 확인 · 가능성 높음 = 단일 출처/포럼 직원 답 · 불확실 = 출처 없음.

## 결론

- **표본 중 앱 기동 때 훑어서 찌꺼기를 치우는 곳은 없다.** 두 갈래가 흔하다:
  1. **안 치움 — 고정 이름이라 같은 파일의 다음 쓰기가 덮는다**(VS Code · git `index.lock` · Rust crate 들 · JetBrains 는 손으로 지우라고 안내).
  2. **폴더 훑기 · 색인 때 게으르게 치움 — 알아볼 접두 + 나이 문턱**(Syncthing — 접두 + 24시간 · 다시 훑기 때 / git `prune` — `tmp_obj_*` 를 만료 지난 것만).
- 공통 모양 = **임시 파일은 같은 폴더**(바꿔치기가 원자적이려면 같은 볼륨이어야 함) · **이름에 앱 표식**(접두나 꼬리) · **훑는 쪽이 그 이름을 건너뛰고, 오래된 것만 지움.**
- 찌꺼기 자체를 없애는 길은 리눅스 `O_TMPFILE`(이름 없는 파일)뿐 — Windows 에선 이름 있는 임시 파일을 피할 수 없다.

→ S25 후보 대입: **(a) 첫 훑기 때 정리**가 Syncthing · git prune 꼴과 같다(나이 문턱을 더하면 그대로). (b) 별도 숨김 폴더는 표본에 선례 없음. (c) 안 치움도 흔하다(고정 이름이면 다음 쓰기가 덮음).

## 도구별

| 도구 | 자리 · 이름 | 언제 치우나 | 확신도 · 출처 |
|---|---|---|---|
| Syncthing | 같은 폴더 · Windows `~syncthing~<이름>.tmp` · Unix `.syncthing.<이름>.tmp` · 160자 넘으면 해시 | 폴더 다시 훑기 때 `keepTemporariesH`(기본 24h)보다 오래된 것 삭제(검사는 약 1시간마다 → 최대 약 2배 지연). 일부러 남겨 재시도 때 블록 재사용 | 이름 · `IsTemporary()` = 확실(`lib/fs/tempname.go` — 메인 원문 확인) · 정책 = 확실(포럼 · issue #3215) · 훑기가 접두로 건너뛰는 호출 자리 = 가능성 높음 |
| VS Code 파일 서비스 | 같은 폴더 · 고정 꼬리 `foo.txt.vsctmp` | 정리 코드 없음 — 고정 이름 + 덮어쓰기라 같은 파일의 다음 원자적 쓰기가 덮음. 심볼릭 링크엔 원자적 쓰기 거부 | 확실(`src/vs/platform/files/common/files.ts` 약 314–323 — 메인 원문 확인 · `diskFileSystemProvider.ts` `doWriteFileAtomic`) · 내부 상태 파일에만 쓰는지 = 가능성 높음 |
| JetBrains 「safe write」 | 같은 폴더 · `이름___jb_tmp___` → 원본을 `___jb_old___` 로 → tmp 를 원본으로 → old 삭제 | 기동 정리 증거 없음 — 손으로 지우거나 설정 끄기 안내 | 가능성 높음(지원 포럼 직원 답) · 정리 = 불확실 |
| git | 같은 폴더 `<경로>.lock` · `objects/` 의 `tmp_obj_*` | 잠금: 정상 종료 · 신호는 지우지만 강제 종료 뒤 남은 `index.lock` 은 **자동으로 안 지우고** 거부 + 사람에게 맡김(PID 재사용 때문에 낡음 판정 불가) · 데이터 임시: `git prune`/`gc` 가 나이로 삭제 | 확실(`lockfile.h` · `lockfile.c` · `builtin/prune.c`) |
| Logseq | 원자적 쓰기 아님(제자리 쓰기) · 실패 때만 `<graph>/logseq/bak/` 백업 | 찌꺼기 없음 — 대신 꺼지면 잘린 파일 가능 | 가능성 높음(`handler.cljs` 103–130) |
| Obsidian | 불명(비공개 소스) — 원자적 쓰기 요청 글(2023)만 있고 직원 답 없음 | 불명 | 불확실 |
| Rust `atomic-write-file` · `tempfile` | 같은 폴더 · `.이름.XXXXXX` / `.tmp` + 무작위 | 정리 장치 없음 — 문서가 「갑자기 끊기면 남을 수 있다」고 명시 | 확실(docs.rs) |

## 공백

- Obsidian · JetBrains 가 기동 · 열기 때 치우는지 1차 출처 없음.
- VS Code 감시자가 `.vsctmp` 를 어떻게 거르는지, 편집기 저장이 기본으로 원자적인지 미확인.
- Windows `ReplaceFile` vs `MoveFileEx(MOVEFILE_REPLACE_EXISTING)` 도구별 선택 미조사.
- 별도 숨김 폴더에서 만들어 옮기는 방식(후보 b)의 선례는 못 찾음.

## 출처

- https://github.com/syncthing/syncthing/blob/main/lib/fs/tempname.go
- https://forum.syncthing.net/t/when-sync-error-how-long-will-remove-or-keep-the-temp-file-syncthing-xxx/1126 · https://github.com/syncthing/syncthing/issues/3215
- https://github.com/microsoft/vscode/blob/main/src/vs/platform/files/common/files.ts · `src/vs/platform/files/node/diskFileSystemProvider.ts`
- https://intellij-support.jetbrains.com/hc/en-us/community/posts/206864695/comments/206381175
- https://github.com/git/git/blob/master/lockfile.h · `lockfile.c` · `builtin/prune.c`
- https://forum.obsidian.md/t/atomic-file-writes/56065
- Logseq `src/electron/electron/handler.cljs` · `backup_file.cljs`
- https://docs.rs/atomic-write-file/latest/atomic_write_file/ · https://docs.rs/tempfile/latest/tempfile/
