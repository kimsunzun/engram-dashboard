# ★ 파일이 바뀌었나 — 내용을 안 열고 가려내는 관행 — light 조사

| 항목 | 값 |
|---|---|
| 상태 | 조사 완료 · 적용 = 관행대로(사용자 위임 2026-10-17 「관행 확인해보고 이부분은 그냥 맞춰」) |
| 날짜 | 2026-10-17 |
| 방법 | `/research` light — 수집자 1명(검색 · 가져오기 약 13회) · 메인 grounding 스팟체크 1건(git 「racy-git」 문서 WebFetch) · 적대 리뷰 없음 |
| 질문 | 데몬의 라이브러리 읽기 캐시 꼬리표 = (경로, 수정 시각, 크기) — 다시 훑을 때 꼬리표가 다른 파일만 다시 연다. 이게 관행인가, 함정(같은 순간 고침 · 시각 정밀도 · 바꿔치기)은 어떻게 막나 |

## 결론

- **(수정 시각, 크기)는 「바뀌었을 수 있다」 를 거르는 싼 문이다** — 정확해야 하는 곳은 여기에 「아슬아슬한 시각」 규칙이나 내용 해시를 더한다(git · Bazel · Unity). Cargo · Ninja · make 는 수정 시각만 보고 함정을 받아들인다. (가능성 높음)
- **git 이 비교하는 칸** — 원문: 「Git compares the file type … and executable bits … `st_mtime` and `st_ctime` timestamps, `st_uid`, `st_gid`, `st_ino`, and `st_size` members.」 (확실 — 메인 확인)
- **같은 순간 고침(racy git)과 그 해법** — 원문: 꼬리표는 그대로인데 「the `st_mtime` is the same as (or newer than) the timestamp of the index file itself … it also compares the contents」 · 그런 항목은 「racily clean」 · 다시 쓸 때 「cached `st_size` information is truncated to zero」 해서 다음에 반드시 내용 비교가 일어나게 한다. (확실 — 메인 확인)
- **Bazel** = 내용 해시가 진짜 기준이고 (경로 · inode · mtime · 크기 + 6.1 부터 ctime) 꼬리표는 해시를 다시 안 내려고 쓰는 것 — ctime 을 더한 이유: 같은 mtime 을 가진 파일이 `mv` 로 바꿔치기되는 경우. (가능성 높음 — 검색 요약)
- **Windows 시각** — NTFS 100ns · FAT 쓰기 시각 2초 · 쓰기 시각은 쓰던 손잡이를 닫아야 정확하다고 Microsoft 문서가 적는다. (확실 — 수집자 가져옴)

## 발견

| 도구 | 비교하는 칸 | 내용 해시 | 아슬아슬 · 정밀도 처리 | 출처 | 확신 |
|---|---|---|---|---|---|
| git 인덱스 | mode · mtime · ctime · uid · gid · ino · size | 칸이 다르거나 racy 일 때만 | mtime ≥ 인덱스 파일 시각 → 내용 비교 · racy 항목은 크기 0 으로 적어 다음에 강제 비교 | https://git-scm.com/docs/racy-git (메인 확인) | 확실 |
| make | 대상 mtime 대 재료 mtime(더 새것이면 다시) | 없음 | 없음 — 시계 어긋남 · 거친 시각이 알려진 문제 | — | 가능성 높음(기억) |
| Ninja | 출력 mtime 대 입력 · `.ninja_log` 에 명령줄 · `restat` | 없음 | 매뉴얼에 정밀도 처리 없음 | https://ninja-build.org/manual.html | 확실 |
| Cargo | 로컬 소스 mtime 대 dep-info mtime | 없음 | 빌드 시작 시각을 적어 빌드 도중 고친 것을 잡는다 | https://doc.rust-lang.org/nightly/nightly-rustc/cargo/compiler/fingerprint/index.html | 가능성 높음(검색 요약) |
| Bazel | (경로 · inode · mtime · 크기 · ctime) → 해시 캐시 | 예 — 해시가 기준 | ctime 추가(바꿔치기) | https://bazel.googlesource.com/bazel/+/refs/heads/release-6.1.2 | 가능성 높음 |
| Unity AssetDatabase | 마지막 수정 시각 + 내용 해시 | 예 | 못 찾음 | https://docs.unity3d.com/ru/Manual/AssetDatabase.html | 가능성 높음 |
| Windows 시각 | NTFS 100ns · FAT 2초(지역 시각) · 접근 시각은 1시간까지 늦을 수 있음 | — | 쓰기 시각은 손잡이를 닫은 뒤 정확 | https://learn.microsoft.com/en-us/windows/win32/sysinfo/file-times | 확실 |

## 적용(관행대로 — 메인 결정)

1. 꼬리표 = (경로 · 수정 시각 전체 정밀도 · 크기 · 파일 신원[볼륨 번호 + 파일 번호 — inode 에 해당]) · **같음 비교**(「더 새것」 비교 아님 — 시계 어긋남 · 복원 파일) · 하나라도 다르면 다시 읽기.
2. 아슬아슬 규칙(git 꼴): 훑기 시각 T 를 함께 적고, 수정 시각이 T − 여유 이상인 항목은 「아슬아슬」 로 표시해 다음 훑기 때 무조건 다시 읽는다 · 여유 ≈ NTFS 수 ms · FAT · exFAT · 네트워크 볼륨 2초 · 이후 훑기에서 충분히 오래된 것으로 보이면 표시를 푼다.
3. 모든 파일 해시는 하지 않는다 — 아슬아슬하거나 파일 신원만 바뀐 항목만 다시 읽는다(다시 읽기가 해시보다 직접적 — 미측정).

## 공백

- Cargo 함정 목록 원문 · git `core.trustctime` / `core.checkStat` 뜻 · Unity Auto Refresh 세부 미확인.
- Windows 변경 알림(ReadDirectoryChangesW · USN 저널)로 훑기 자체를 없애는 길은 안 봄 — 감시는 이미 거부한 결정(PRD R19).
- NTFS 쓰기 시각이 쓸 때마다 바로 바뀌는지(손잡이 닫을 때인지)는 일부만 확인.
