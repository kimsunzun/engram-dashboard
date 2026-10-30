# 검사 결과 표시 — 실패 표시와 「고친 뒤 다시 검사 전」 상태 (관행 조사)

- **상태:** light 조사 · 2026-10-18 · 수집자 1(문서형) + 메인 grounding(Unreal 원문 대조) · 적대 리뷰 없음
- **질문:** 설정 에셋을 CLI 로 검사(약 2초 · 모델 호출 없음 · 설정마다 OK 기록)할 때, 실패를 ✗ 로 보이는가, 사용자가 고친 뒤 다시 검사하기 전에는 무엇을 보이는가.
- **맥락:** 감시 없음 — 앱 안 편집은 즉시 알고, 바깥 편집은 새로 고침 때 안다. 검사는 지금 실행 직전에만 돈다.

## 결론

- **실패는 명시 표시가 관행이다**(Unreal 「Compile Failed」 · JetBrains 실행 구성 빨간 X · IDE 오류 배지). — 가능성 높음(Unreal 만 원문 대조)
- **고친 뒤 상태는 두 갈래다.** 싸고 프로세스 안에서 도는 검사(VS Code 언어 서버 · JetBrains 편집기 분석)는 바뀌면 바로 다시 검사해 낡은 창이 없다. 비싸고 명시로 도는 검사(Unreal 블루프린트 · Xcode/VS 빌드)는 「다시 해야 함」 상태를 따로 두고 「지금 검사」 단추를 준다. — 가능성 높음
- **옛 표시를 그대로 두는 것은 의도된 설계로 찾지 못했다** — 그런 사례(Xcode 낡은 이슈 · JetBrains 빨간 파일명 · rust-analyzer 저장 시 진단)는 버그로 보고된다. — 가능성 높음
- **Unreal 의 「Compile Needed」 가 가장 가까운 꼴이다** — 성공 · 필요 · 경고 · 실패 넷이고 어느 상태든 누르면 다시 컴파일한다. — 확실(원문 대조)

## 피어별

| 피어 | 실패 표시 | 고친 뒤 | 확신도 | 출처 |
|---|---|---|---|---|
| Unreal 블루프린트 Compile 단추 | Compile Failed | Compile Needed「The Blueprint needs to be recompiled.」 · 누르면 컴파일 | 확실(원문 대조) · 아이콘 모양 · 실패→필요 전환은 미확인 | https://dev.epicgames.com/documentation/unreal-engine/toolbar-in-the-blueprints-visual-scripting-editor-for-unreal-engine |
| VS Code | Problems · 오류 배지 | 언어 서버가 입력 중/저장 때 다시 냄 · 낡은 진단은 버그 취급 | 가능성 높음(2차 자료) | https://github.com/microsoft/vscode-go/issues/2907 |
| JetBrains | 실행 구성 빨간 X | 구성 편집 때 다시 판정(추정) · 빨간 파일명이 안 지워지는 건 버그 보고 | 불확실 | intellij-support 포럼 글 |
| Xcode · Visual Studio | 이슈 목록 | 빌드 오류는 다음 빌드까지 남음(VS 「Build Only」 필터) · Xcode 낡은 이슈는 버그 보고 | 가능성 높음(포럼) | https://developer.apple.com/forums/thread/709069 |
| GitHub Actions | 「Workflow file is not valid」 | 원격 검사는 push 때 · 편집기 확장은 입력 중 | 가능성 높음(얇음) | — |
| Unity Asset Manager | imported with error | 최신 · 낡음 · 오류 세 갈래 표시 | 불확실(요약만) | https://docs.unity3d.com/Packages/com.unity.asset-manager-for-unity@1.7/manual/import-assets.html |
| Terraform 저장된 plan | — | 쓸 때 「Saved plan is stale」 로 거절(표시 없이 키 무효화) | 가능성 높음 | https://discuss.hashicorp.com/t/question-on-error-saved-plan-is-stale/52912 |

## 공백

- VS Code · JetBrains · Xcode · VS · GitHub 는 1차 문서를 못 읽었다(검색 요약 · 포럼).
- 외부 CLI 로 검사하는 설정 파일 목록에 「검사 필요」 표시를 단 피어는 찾지 못했다 — 가장 가까운 것이 Unreal.
