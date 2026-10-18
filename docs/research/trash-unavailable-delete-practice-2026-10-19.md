# 휴지통을 못 쓰는 위치에서 지울 때 관행 (2026-10-19)

| 항목 | 값 |
|---|---|
| **상태** | 조사 완료 · 결정 대기(라이브러리 삭제 — 시스템 휴지통을 못 쓰는 위치 · PRD R18) |
| **방법** | `/research` light — 수집자 1(검색 6 · 페이지 6) + 메인 grounding(Obsidian 설정 — 공식 도움말 사본 검색 결과) |
| **질문** | 네트워크 드라이브 · 휴지통 없는 볼륨 · 휴지통 용량 초과에서 앱은 ① 앱 전용 휴지통을 쓰나 ② 영구 삭제를 묻나 ③ 거절하나 |

## 결론

1. **앱 전용 휴지통 선례 = Obsidian**(가능성 높음): 설정 「Deleted files」 = 시스템 휴지통(기본) · Obsidian 휴지통(볼트 안 숨은 `.trash` 폴더) · 영구 삭제 셋 중 사용자가 미리 고른다. `.trash` 는 시스템 휴지통과 따로 비우고, 자동 비우기 · 되살리기 화면은 본체에 없고 커뮤니티 플러그인(Empty Trash · Trash Explorer)이 있다(그래서 본체엔 없다고 추정). 시스템 휴지통이 실패할 때 저절로 `.trash` 로 넘어가지는 않는다(포럼 — Linux 에서 영구 삭제된 사례).
2. **실패하면 영구 삭제를 묻는 선례 = VS Code**(확실 — 관리자 답변): 휴지통 이동이 실패하면 모달로 「영구 삭제할까요」 를 묻는다 · `files.enableTrash` 를 끄면 「Delete Permanently」. 사람에게 묻는 꼴이라 에이전트 호출에는 답할 사람이 없다.
3. **Windows 탐색기**: 네트워크 위치 삭제 = 영구 삭제 + 확인(가능성 높음 — 공식 문서 못 찾음 · 포럼).
4. ★**Windows API 는 휴지통 대신 영구 삭제로 넘어갈 수 있다**(확실 — Microsoft 문서): `IFileOperation` 의 `FOF_WANTNUKEWARNING` = 「재활용 대신 파괴될 때 경고」 · `FOFX_RECYCLEONDELETE` = 「영구 삭제 대신 휴지통으로」. Rust `trash` crate 는 `FOF_NO_UI | FOF_ALLOWUNDO | FOF_WANTNUKEWARNING` 로 부르는 것으로 보여(가능성 높음 — 요약 읽음) UI 없음 때문에 경고가 묻혀 **말없이 영구 삭제될 위험**이 있다(불확실 — 실 SMB 공유에서 재 봐야 한다). 우리 구현(TRD — platform 휴지통 · `IFileOperation`)은 영구 삭제로 넘어가는 것을 스스로 잡아 거절해야 한다.
5. 언리얼 에셋 삭제는 OS 휴지통이 아니라 소스 컨트롤로 되살린다(불확실 — 스니펫). JetBrains 는 보지 않았다.

## 피어

| 피어 | 휴지통을 못 쓸 때 | 앱 휴지통 · 위치 | 비우기 | 출처 | 확신도 |
|---|---|---|---|---|---|
| Obsidian | 미리 고른 방식 그대로(저절로 대체 없음) | 예 · 볼트 안 숨은 `.trash` | 자동 없음 · 플러그인 | https://huggingface.co/spaces/anpigon/obsidian-qa-bot/blob/e272fa2f2c33a981360ec7e29f42e93f75fd504f/docs/obsidian-help/Files%20and%20folders/Manage%20notes.md · https://forum.obsidian.md/t/obsidian-permanent-deletes-files-system-trash/46409 | 가능성 높음 |
| VS Code | 모달 「영구 삭제할까요」 | 없음 | — | https://github.com/microsoft/vscode/issues/14638 | 확실(모달) |
| Windows 탐색기(네트워크) | 영구 삭제 + 확인 | 없음 | — | 공식 없음 | 가능성 높음 |
| Win32 `IFileOperation` | 휴지통 대신 파괴 가능 · 경고 플래그 있음 | 없음 | — | https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-ifileoperation-setoperationflags | 확실 |
| Electron `shell.trashItem` | 오류면 거절(reject) · 네트워크 동작 문서 없음 | 없음 | — | https://www.electronjs.org/docs/latest/api/shell | 확실(문서) · 네트워크 불확실 |
| Rust `trash` crate | `IFileOperation` · 네트워크 별도 처리 없음 | 없음 | — | https://raw.githubusercontent.com/Byron/trash-rs/master/src/windows.rs | 가능성 높음(플래그) · 실패 동작 불확실 |
| Unreal | 소스 컨트롤로 되살림 | 없음 | — | https://issues.unrealengine.com/issue/UE-201031 | 불확실 |

## 우리에게

- 선택지: A 시스템 휴지통을 못 쓰면 라이브러리 안 숨은 `.trash` 로(Obsidian 의 두 번째 방식을 대체 경로로 씀 — 저절로 넘어가는 선례는 없음) · B 늘 거절 · C 영구 삭제 + 센 확인(VS Code 꼴 — 에이전트 호출엔 맞지 않음). 사용자 결정.
- 어느 쪽이든 구현은 「휴지통 대신 영구 삭제」 를 잡아야 한다 — 실 SMB 공유 · 큰 파일 실측이 구현 전 확인 항목.

## 한계

- light — 적대 리뷰 없음. Obsidian 은 공식 도움말 사본(제3자 저장소)과 포럼으로 확인했다. JetBrains 는 보지 않았다.
