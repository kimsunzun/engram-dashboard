# 여러 파일에 걸친 이름 바꾸기의 도중 꺼짐 대비 관행 (2026-10-31)

- **상태:** `/research` light — 수집자 1 · 메인 grounding 스팟체크(Unreal 리다이렉터 공식 문서 · Obsidian 포럼 원문 직접 확인) · 적대 리뷰 없음(light). 결정 대기 — S25 파일 작업 저널(`library-ops\` · flow ④ · 부팅 때 마저 끝내기)을 둘지.
- **배경:** 지금 설계 = 계획 → 저널 기록 → 파일 이름 바꾸기 → 참조하는 프리셋 파일 고치기 → 에이전트 프로필 고치기 → 저널 삭제 · 도중에 꺼지면 다음 부팅에 저널로 남은 단계를 마저 한다. 사용자 의문(2026-10-31): 「이거 다른 데서도 안 할 것 같은데」.
- **확신도 범례:** 확실 = 1차 문서 · 원문 직접 확인 · 가능성 높음 = 요약 · 사용자 보고 · 불확실 = 커뮤니티 요약 · 모름 = 못 찾음.

## 결론

1. **저널(의도 기록)로 여러 파일 이름 바꾸기를 보호하는 도구는 못 찾았다.** 사용자 짐작이 맞다(「없다」 가 아니라 「못 찾았다」).
2. **관행은 세 갈래다.**
   - **보호 없음** — 도중에 끊기면 참조가 반쯤 고쳐진 채 남고 사용자가 손으로 고친다(Obsidian · Lightroom 폴더 옮기기).
   - **설계로 피함** — 참조를 다시 쓰지 않는다(Unreal 리다이렉터 · Unity GUID).
   - **메모리 안에서 한 번에 적용 + 되돌리기** — VS Code 텍스트 편집 묶음 · JetBrains 되돌리기 · Local History. 디스크 상태의 꺼짐 복구는 아니다.
3. Obsidian 은 링크 600개짜리 노트 이름 바꾸기에 20초 넘게 걸리고, 그 사이 다시 바꾸거나 앱을 닫으면 옛 이름 · 새 이름 · 두 번째 새 이름을 가리키는 링크가 섞여 남는다는 보고가 있다(확실 — 포럼 원문).

## 피어별

| 피어 | 방식 | 도중에 꺼지면 | 확신도 |
|---|---|---|---|
| **Obsidian** | 이름 바꾼 뒤 링크를 차례로 고침 · 트랜잭션 · 저널 언급 없음 | 링크가 섞여 남음 · 사용자 몫 | 확실(포럼 보고 원문) · 구현 문서는 못 찾음 |
| **VS Code** | 텍스트 편집만 든 `WorkspaceEdit` 은 전부 아니면 전무 · 파일 만들기 · 지우기 · 이름 바꾸기가 섞이면 순서대로 적용하다 실패한 자리에서 멈추고 앞의 것은 되돌리지 않음 | 프로세스 꺼짐 대비는 못 찾음 | 가능성 높음(API 문서 재게시본 요약) |
| **JetBrains** | 되돌리기 · Local History · 충돌 나면 전부 되돌리고 미리보기 | 모름 | 불확실 |
| **Unreal** | 옛 경로에 리다이렉터를 남겨 참조를 바로 다시 쓰지 않음 · 나중에 「Fix Up Redirectors」 가 참조 패키지를 다시 저장하고 다 되면 리다이렉터 삭제 | 리다이렉터 덕에 중간 상태에서도 참조가 안 깨짐 · 다시 쓰기는 미뤄도 되고 다시 해도 되는 작업 | 확실(공식 문서 원문) |
| **Unity** | 참조가 경로가 아니라 `.meta` 의 GUID — 이름 바꾸기 때 참조 파일을 다시 쓸 필요 없음 | 해당 없음 | 불확실(일반 지식 · 이번에 검증 안 함) |
| **Lightroom Classic** | 카탈로그는 SQLite · 디스크 작업과 카탈로그 갱신을 묶어 보호한다는 근거 없음 | 도구 안에서 폴더를 옮기다 하위 폴더 일부가 양쪽에서 사라졌다는 보고 · 탐색기로 옮긴 뒤 「Find Missing Folder」 를 권함 | 가능성 높음(사용자 보고) |

출처:
- Obsidian — https://forum.obsidian.md/t/obsidian-note-renaming-bug-causing-broken-links-and-delayed-updates/59355 (메인 직접 확인 2026-10-31: 「renaming of moc takes over 20 seconds to complete」 · 「the structure is broken, leaving links point to moc, moc-new-name, and moc-second-new-name」) · https://forum.obsidian.md/t/internal-links-not-updating-after-renaming-files-on-ipad-no-sync-no-plugins/101595
- VS Code — https://vscode-api.js.org/classes/vscode.WorkspaceEdit.html
- JetBrains — https://intellij-support.jetbrains.com/hc/en-us/community/posts/4408685170834/comments/4408703669650
- Unreal — https://dev.epicgames.com/documentation/en-us/unreal-engine/asset-redirectors-in-unreal-engine (메인 직접 확인 2026-10-31: 「When you move or rename an asset … it leaves a Redirector in the asset's old location」 · 「This will resave all packages that point to the redirector, and will delete the redirector if it was able to resave all things referencing it」)
- Lightroom — https://community.adobe.com/questions-675/move-folder-from-1-ssd-to-another-within-lrc-2-of-3-subfolders-disappeared-984126

## 우리에게

| | (가) 저널 유지 | (나) 저널 없이 — 순서만 정함 (Obsidian 꼴) | (다) 리다이렉터 (Unreal 꼴) |
|---|---|---|---|
| 선례 | 못 찾음 | Obsidian · Lightroom | Unreal |
| 꺼진 뒤 | 다음 부팅에 마저 끝냄 | 옛 경로를 가리키는 줄이 「찾을 수 없음」 으로 남음 → **다시 연결 한 번이 그 옛 경로를 가리키는 모든 프리셋 · 에이전트를 고친다**(R19 (a)) | 옛 경로의 안내 파일이 참조를 이어 줌 |
| 비용 | 저널 모듈 · 부팅 때 재실행 · 두 번 해도 같은 단계 설계 | 0 — 이미 있는 다시 연결로 복구 | 사용자 폴더에 안내 파일이 생김(탐색기 · git 에 보임) · 나중에 치우는 작업 |

## 공백 · 확인 못 한 것

- Obsidian · VS Code · JetBrains 의 구현 소스는 보지 않았다.
- Unity GUID 항목은 이번에 검증하지 않았다.
- git · 패키지 매니저 · Dropbox 같은 「여러 단계 파일 작업」 도구는 다루지 않았다.
