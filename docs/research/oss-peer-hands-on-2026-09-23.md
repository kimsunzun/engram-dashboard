# herdr·orca 직접 써 보기 — 실행법과 orca 오케스트레이션의 실체

| 항목 | 값 |
|---|---|
| **상태** | 1 패스 기록 — herdr 는 **소스 빌드까지 실측**(TUI 미기동) · orca 는 **소스·이슈 독해**(앱은 사용자가 설치해 쓰는 중이고, 이 문서는 앱을 직접 몰지 않았다) |
| **날짜** | 2026-09-23 |
| **방법** | herdr 참조 클론 안에서 `cargo build --release` 실행 · 두 클론(`I:\Engram_Workspace\opensource\{herdr,orca}`) 정독 · GitHub 이슈·릴리즈 조회(gh) |
| **확신도 범례** | **실측** = 우리가 직접 돌려 봤다 · **소스 확인** = 클론·README 를 읽었다 · **이슈 확인** = GitHub 이슈·릴리즈·검색 결과(gh, 2026-09-23) · **추정** = 정황뿐 |
| **왜** | 「받아 둔 클론」 둘을 읽기만 하다가 실제로 써 보기로 했다. herdr 는 데몬 모델의 가장 가까운 피어라 **돌아가는 물건**이 필요했고, orca 는 사용자가 설치했는데 **오케스트레이션이 안 되는 것 같다**는 체감의 정체를 가려야 했다 |

> ★**먼저 읽을 것 — 둘 다 옛 문서의 전제를 바꾼다**★
>
> 1. ★**herdr 클론은 더 이상 master 가 아니다**★ — `v0.9.1` 태그(`065ef9d`)에 detached 로 서 있고 **그 자리에서 빌드했다**(읽기 참조와 실행 빌드를 한 곳에 둔다 — 사용자 결정). 이 날짜 전 문서의 herdr `파일:줄` 포인터는 옛 master `624dfd4` 기준이라 **지금 체크아웃에서는 줄이 어긋난다** — `git show 624dfd4:<경로>` 로 연다(로컬 `master` ref 는 그대로 있다).
> 2. ★**orca 의 「Orchestration」은 앱이 알아서 모는 기능이 아니다**★ — 실행 중인 앱 런타임이 Run·Task·Dispatch 상태와 라우팅을 쥐지만 **워커를 스케줄하거나 배치하지 않는다.** 모는 것은 **코디네이터 에이전트**이고, CLI 명령군 + 에이전트 스킬로 그 런타임을 부른다. UI 에서 켜면 무언가가 알아서 배치되는 물건이 아니다(§2-3).

---

## 0. 결론

- **herdr** — Windows 는 GA 다(소스 확인). 소스 빌드는 **README 개발 절에는 없고 `AGENTS.md` 에만 있는 전제 하나(Zig 0.16.0)**만 채우면 6분에 컴파일러 경고 0(`build.rs` 의 `cargo:warning` 1줄 제외)으로 선다(실측). 확인은 `herdr.exe --version` 까지이고 ★**TUI 는 아직 안 띄웠다**★.
- **orca** — 「안 되는 것 같다」의 1순위 후보는 **기대 불일치**(추정)이고, 그다음이 **CLI·스킬 미설치/미연결**과 **Windows 결함**(둘 다 이슈 확인 — 열린 것이 여럿)이다. ★**사용자의 실제 실패 양상·버전은 모른다**★ — 순위는 그것 없이 세웠다(§2-6).

---

## 1. herdr (`herdrdev/herdr` · Apache-2.0)

### 1-1. 받는 길 셋

| 길 | 무엇 | 비고 |
|---|---|---|
| 공식 설치 스크립트 | PowerShell 한 줄(아래 코드 블록) — PATH 등록 · 버전별 폴더 + `current` junction 으로 업데이트 | ★**보안 정책 확인이 먼저다**★(아래) |
| 릴리즈 zip | 최신 안정판 `v0.9.1`(2026-09-16) · 자산에 `herdr-windows-x86_64.zip`(이슈 확인) | 앱 로컬 ConPTY 런타임을 함께 싣는다(§1-3) |
| **소스 빌드(이번에 고른 길)** | 참조 클론 안에서 빌드(§1-2) | ConPTY 런타임이 안 실린다 |

공식 설치 스크립트(소스 확인 — `docs/next/website/src/content/docs/windows-beta.mdx:12-14`):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://herdr.dev/install.ps1 | iex"
```

엔드포인트 보안이 이 fileless PowerShell 명령을 막는 PC 를 위해 **herdr 가 스스로 대안을 적어 둔다** — `install.cmd` 를 `curl.exe` 로 받아 돌린다(`windows-beta.mdx:16-20`):

```cmd
curl.exe -fsSLo install.cmd https://herdr.dev/install.cmd && install.cmd && del install.cmd
```

★**둘 다 회사 PC 에서 원격 스크립트를 받아 실행하는 방식이라 보안 정책 확인이 먼저다**★(앞의 것은 실행 정책 우회까지 건다) — 가부는 담당 부서가 판단하고 이 문서는 내리지 않는다. 대안 경로는 **막힘을 비켜 가는 길이지 허가가 아니다.**

- Windows GA 의 근거(소스 확인): `docs/next/CHANGELOG.md:147`("Windows support is now generally available through stable releases…") · `docs/next/website/src/content/docs/install.mdx:6`("generally available, with documented platform-specific limitations and ongoing fixes"). ★줄 번호는 `v0.9.1` 체크아웃 기준이다★ — 옛 master 에서는 CHANGELOG 가 `:32` 였다.

### 1-2. 소스 빌드 — 실측 2026-09-23

1. **클론을 태그로 옮겼다.** 받아 둔 클론은 shallow · master `624dfd4`(2026-08-20, 원격보다 256 커밋 뒤)였다. `git fetch --depth 50 origin tag v0.9.1` → `git switch --detach v0.9.1` 로 ★**`065ef9d`("release: v0.9.1", 2026-09-16)에 detached**★. 여전히 shallow 이고 로컬 `master` ref 는 안 건드렸다.
2. **Rust 1.96.1.** `rust-toolchain.toml` 이 채널과 컴포넌트(`clippy`·`rustfmt`)를 고정하고 rustup 이 자동 설치한다(`~/.rustup/toolchains/` 약 1.4 GB). ★**함정 — 전경에서 친 `rustup show active-toolchain` 이 조용히 그 설치를 방아쇠로 당겨 도구 제한 120초에 걸렸다**★ → **클론 안에서** 인자 없는 `rustup toolchain install` 을 **먼저, 명시적으로(또는 백그라운드로)** 돌린다. 인자가 없으면 그 디렉터리의 활성 툴체인을 깔므로(`rustup help toolchain install` — "by default the active toolchain") `rust-toolchain.toml` 이 적은 컴포넌트까지 함께 온다. 버전 번호를 손으로 박으면 그 파일과 따로 낡는다.
3. ★**README 개발 절에 없는 전제 — Zig 0.16.0**★. `build.rs` 가 vendored libghostty-vt 를 `zig` 로 짓고 없으면 panic 한다(소스 확인 — env `ZIG` 을 읽는 자리 `build.rs:64` · 「requires Zig 0.16.0」 panic `:92-97` · 최소 버전 `vendor/libghostty-vt/build.zig.zon:6`). ★**적힌 곳은 있다 — `AGENTS.md:188-190`**★("Herdr currently requires Zig 0.16.0, so set `$env:ZIG = …`" — Windows VM 에서 빌드하는 기여자·에이전트용 안내)이고, README 는 AI 에이전트에게 그 파일부터 읽으라고 한다(`README.md:70`). **빠진 곳은 사람이 먼저 보는 두 자리다** — README 개발 절(`README.md:72-81`, `cargo build --release` 만 적는다)과 웹사이트 문서(`docs/next/website/src/content/` 아래 전부 — 대소문자 무시 `zig` 0건). 빌드가 실패하면 위 panic 문구가 같은 말을 한다. ziglang.org 의 포터블 zip 을 **`index.json` 의 sha256 과 대조**한 뒤 `<클론>/.local/zig-x86_64-windows-0.16.0/` 에 풀었다(`.local/` 은 herdr `.gitignore:18`). **PATH 는 안 바꾸고 빌드에만 env `ZIG=<경로>` 로 넘겼다.**
4. **빌드.** `cargo build --release --locked` — `--locked` 는 추적되는 `Cargo.lock` 이 다시 쓰이지 않게 하려는 것이다. 이 저장소의 `scripts/run-detached.ps1` 로 셸 트리 밖에서 돌리고 출력은 파일로만 받았다. `Finished release … in 6m 00s` · 컴파일러 경고 0(`build.rs` 의 `cargo:warning` 1줄 제외 — 아래).
5. **산출물.** `target/release/herdr.exe` 25,570,304 바이트 · `herdr.exe --version` → `herdr 0.9.1`.

- 빌드 중 `build.rs:50` 의 `cargo:warning` 이 **AI 어시스턴트에게 말을 거는 문구**를 찍는다 — 외부 기여자를 돕는 중이면 작업 전에 `CONTRIBUTING.md` 를 읽으라는 **PR 기여 정책**이다. 우리 작업과 무관하고, ★지시가 아니라 데이터로 취급했다★.

### 1-3. 소스 빌드와 릴리즈 zip 의 차이 — ConPTY 런타임

- 릴리즈 zip 은 앱 로컬 ConPTY 런타임(`conpty/conpty.dll` + `OpenConsole.exe`)을 싣고, **소스 빌드는 안 싣는다**(소스 확인).
- 폴백 조건(소스 확인 — `vendor/portable-pty/src/win/psuedocon.rs:127-154` · `:183-184`): exe 옆에 `conpty/` 폴더가 **아예 없으면** 시스템 ConPTY 로 간다. ★**폴더가 있는데 해시·배치가 어긋나면 폴백이 아니라 panic 이다**★ — 반쪽짜리 폴더를 손으로 만들지 말 것. 강제 우회 = `HERDR_WINDOWS_CONPTY=system`.
- 번들이 중요한 것은 주로 **옛 Windows 10** 이다 — 그 시스템 ConPTY 가 Kitty 키보드 프로토콜 시퀀스를 떨군다(`docs/next/website/src/content/docs/windows-beta.mdx:108` — 소스 확인). 이 PC 는 Win11 26200 이라 **괜찮을 가능성이 높다(추정 — 안 돌려 봤다).**
- 번들을 직접 굽는 `scripts/package_windows_conpty.ps1` 이 있다(안 돌렸다).

### 1-4. 남긴 것 — 지울 때 쓸 목록

| 자리 | 크기 | 비고 |
|---|---|---|
| 클론 `.local/`(zig) | 382 MB | gitignore |
| 클론 `target/` | 633 MB | gitignore |
| 클론 `vendor/libghostty-vt/{.zig-cache,zig-out,zig-pkg}/` | 안 쟀다 | gitignore |
| `~/.rustup/toolchains/` 의 1.96.1 | 약 1.4 GB | 클론 밖 |
| `%LOCALAPPDATA%\zig` 캐시 | 59 MB | 클론 밖 |
| `~/.cargo/registry` 의 crate 들 | 안 쟀다 | 클론 밖 |

### 1-5. 쓰는 법 (소스 확인 — README · 공식 문서)

- 작업할 폴더에서 `herdr` 를 친다. `ctrl+b q` 로 떼어 내고 `herdr` 로 다시 붙는다 — **떼어 내도 에이전트는 계속 돈다.**
- ★**GUI 창이 아니라 터미널 안의 TUI 다.**★
- 문서 색인 = herdr.dev/docs(quick start · supported agents · keyboard · configuration · session state · remote · socket api · plugins).
- **한국어 IME** — `experimental.switch_ascii_input_source_in_prefix` 의 Windows 지원은 **한국어 IME 한정**이다(`docs/next/website/src/content/docs/configuration.mdx:562` — 옛 master 에서는 `:517`).

---

## 2. orca (`stablyai/orca` · MIT)

### 2-1. 클론과 현행의 거리

- 로컬 클론은 ★**2026-08-20 의 shallow 50 커밋 스냅숏(`df7460af`) — 현행보다 약 한 달 뒤**★(소스 확인).
- 현행(★**2026-09-23 gh 조회 시점의 스냅숏 — 거의 매일 바뀐다**★): 최신 릴리즈 `v1.4.207`(2026-09-22) · 거의 매일 릴리즈 · 75.7k★ · 마지막 push 2026-09-23(이슈 확인). 버전을 인용할 땐 이 값을 베끼지 말고 `gh release list -R stablyai/orca` 로 다시 본다.
- ★**그래서 아래 「소스 확인」은 한 달 전 코드의 말이다**★ — 현행과 갈린 것이 확인된 자리는 그 자리에 적었다.

### 2-2. 기능 지도 (소스 확인 — README + `src/`)

- **병렬 워크트리** — 격리된 워크트리 하나에 에이전트 하나 · 자식/최상위 계보 · 같은 프롬프트를 여러 에이전트에. 워크트리별 셋업 훅 = `orca.yaml`.
- **에이전트** — CLI 에이전트 약 29종을 **평범한 PTY** 로(Claude Code · Codex · Gemini · OpenCode · Cursor · Copilot …). 계정 전환 + Claude/Codex 사용량 추적.
- **터미널** — WebGL xterm · 분할 · 재시작을 넘는 스크롤백(Windows 는 LOCALAPPDATA 아래 터미널 데몬 — `config/electron-builder.config.cjs:355-363`).
- **리뷰 루프** — diff 줄 코멘트를 에이전트에게 되돌려 보낸다 · GitHub·Linear PR/이슈를 앱 안에서. `src/main/` 에 gitlab·bitbucket·azure-devops·gitea·jira 디렉터리도 있다(성숙도는 **추정** — 안 읽었다).
- **편집·브라우저** — VS Code 풍 에디터 · 내장 Chromium + 「Design Mode」 · Computer Use(win32 provider).
- **Orca CLI** — `orca worktree create` · `terminal create/send/read/wait` · 브라우저 snapshot/click/fill. **에이전트도 이것을 쓴다.**
- **실험 토글**(Settings > Experimental) — Pet · Agents View · Agent Dashboard(칸반) · **Chat UI** · Terminal attention · Agent sleep · New card style(워크트리 카드) · Cloud VM(목록 = `src/renderer/src/components/settings/experimental-search.ts:9-234`). Chat UI 는 클론에도 있다 — 이름 `native-chat-experimental-search-entry.ts:7` · 등록 `experimental-search.ts:142` · 렌더 `ExperimentalPane.tsx:143-144` · 본체 `src/main/native-chat/`.
- **원격·모바일** — SSH 원격 워크트리 · Linux 헤드리스 `orca serve` · iOS/Android 컴패니언.
- **Windows** — NSIS 설치본 · 안정판은 SignPath 서명(`electron-builder.config.cjs:319-331`) · WSL 지원.

### 2-3. ★오케스트레이션의 실체 — 앱 런타임은 기록·라우팅하고, 모는 것은 코디네이터 에이전트다★ (소스 확인 — `skill-guides/orchestration.md`)

「Orca Orchestration」 = **실행 중인 Orca 런타임이 쥔 조율 상태 + 그것을 부르는 `orca orchestration …` CLI 명령군 + 에이전트 스킬 `orchestration`**. CLI 명령은 **실행 중인 런타임으로 가는 RPC** 다(`:52` "`orca orchestration` commands are RPC calls to the running Orca runtime") — 런타임이 Run·Task·Dispatch 와 메시지를 기록하고 라우팅하며, 복구 명령 `reset` 이 지우는 것도 「local orchestration database state」다(`:287`). 현 master 가이드는 첫 문단에서 이것을 "Orca's structured coordination layer. It records who owns work…" 로 부른다(gh 조회 2026-09-23).

| 어휘 | 뜻 |
|---|---|
| Run | 코디네이터의 이름공간 + 수신함 |
| Task | 작업 항목(의존 그래프는 선택) |
| Dispatch | 한 번의 시도 |

- 명령: `worker-start` · `send` / `check --wait` / `ask` / `reply` · `worker_done` · 결정 게이트 · `worker-release`.
- ★**가이드가 스스로 못 박는다**★ — "a Run … never schedules or places workers"(`:104`) · "Agents still choose placement and concurrency; Orca does not schedule workers or infer conflicts"(`:181`). **배치와 동시성은 코디네이터 에이전트가 CLI 를 불러 정하고, 런타임은 기록하고 라우팅한다**("Orca routes them to that Dispatch's Run" — `:104`).
- 옛 스케줄러 명령(`coordinator-start/stop` · `run` · `run-stop`)은 **은퇴했다** — 효과 없이 복구 안내만 돌려준다(`:285`).

### 2-4. 켜는 법 (소스 확인)

1. ★**Orca 앱(런타임)이 떠 있어야 한다**★ — 가이드의 전제 첫 줄이 "`orca status --json` should show a running runtime" 이다(`skill-guides/orchestration.md:49` · 현 master 가이드도 첫 단계에서 런타임부터 확인한다 — gh 조회 2026-09-23). 명령이 그 런타임으로 가는 RPC 라(`:52`) 런타임이 없으면 갈 곳이 없다(추정 — 꺼 놓고 돌려 보지는 않았다).
2. **Orca CLI 를 PATH 에 올린다** — Settings 에서, 또는 아래 대화상자가 함께 한다. Windows 는 **사용자 PATH 를 레지스트리에** 쓴다. ★**설치 프로그램은 이것을 안 한다**★(`src/main/cli/cli-installer.ts:208`).
3. **스킬 설치** — 앱의 「Enable orchestration」 대화상자가 **2·3 을 한 버튼으로 한다**: 「Install CLI & skill」(`src/renderer/src/components/floating-terminal/FloatingTerminalOrchestrationDialog.tsx:156`)이 터미널을 열기 전에 CLI 가 PATH 에 없으면 먼저 등록하고(`:165-170` 의 `onBeforeOpenTerminal` → `src/renderer/src/lib/agent-skill-cli-prerequisite.ts:21` → `:43` `window.api.cli.install()`), 그다음 스킬 설치 명령을 도는 터미널을 연다. 명령 문자열은 `src/shared/agent-feature-install-commands.ts` 가 **조립만** 한다. 손으로 치면 `npx skills add https://github.com/stablyai/orca --skill orchestration --global`(Node 필요).
   - ★**이 명령도 보안 정책 확인이 먼저다**★ — 서드파티 npm 패키지(`skills`)를 받아 실행하고, 그것이 **전역 에이전트 스킬 폴더에 쓴다**(`--agent` 를 안 주고 감지가 0 이면 알려진 에이전트 약 75종 전부에 깐다고 조립기 주석이 적는다 — `agent-feature-install-commands.ts:50-52`). 가부는 담당 부서가 판단하고 이 문서는 내리지 않는다.
4. **에이전트에게 조율·감독을 시키거나 `/orchestration` 을 친다.**

- localStorage `orca.orchestration.enabled`(`src/renderer/src/lib/orchestration-setup-state.ts:2`)는 **셋업을 마쳤다는 표시일 뿐**이다.
- ★클론 가이드의 "must be enabled in Settings > Experimental"(`skill-guides/orchestration.md:51`)은 낡았다★ — 그런 항목이 없고, 현 master 가이드에서는 그 줄이 빠졌다.

### 2-5. 「안 되는 것 같다」의 후보 — 순위

| 순위 | 후보 | 근거 |
|---|---|---|
| 1 | **기대 불일치** — 「The AI Orchestrator」로 내세우지만 UI 에서 알아서 조율하는 것은 없다 | 추정(전제 = §2-3) |
| 2 | **CLI·스킬 미설치 또는 미연결** — 대화상자(§2-4 의 3)가 CLI 와 스킬을 함께 깔아 주므로, 남는 모양은 ① 대화상자를 건너뛰고 손으로 반만 깐 경우 ② 스킬은 깔렸는데 **쓰는 에이전트에 연결되지 않은** 경우 ③ 근거 칸의 이슈들이다 | 이슈 확인 — #14303(메인테이너: "you don't have the CLI installed") · #12422("Agents: not linked") · #18263(Windows · 공유 provider-home 링크 · open) · #21095/#17935(스킬 설명 1024자 초과) |
| 3 | **Windows 결함** | 이슈 확인 — #14505(Claude 워커에 task 프롬프트가 **제출 안 된 채** 얹힘 · 1.4.205 · Win11 에서 12/12 · 2026-09-21 · open) · #22345(1.4.207 에서 `--worktree new-child/new-top-level` 이 `selector_not_found` — ★**가이드 예시가 글자 그대로 실패한다**★) · #14502(패키지된 Windows 빌드의 번들 zod 손상 — 1.4.180 에서 보고 · 현행 재현 모름) · #15825 · #20695 |
| 4 | **에이전트가 스킬을 무시** | 이슈 확인 — #16075(Claude Code 가 `/orchestration` 을 자기 네이티브 서브에이전트로 갈아 끼움 · Win 1.4.186 · open) |
| 5 | **Claude/Codex 밖 백엔드 불안정** | 이슈 확인 — #18399/#16902(Grok) · #11721/#22250(Antigravity) · #17326/#18725(Windows 의 Qwen) · #15125(`dispatch --inject` 는 claude/codex/gemini/cursor 만) |
| 6 | UI/UX | 이슈 확인 — #4374 · #17701 · #21955 |

- **규모** — `orchestration` 라벨 열린 이슈 157건, 그중 `os:Windows` 도 붙은 것 38건(gh 검색 2026-09-23 — 이슈 확인).

### 2-6. 모르는 것

- #14502 가 1.4.207 에서도 재현되는지.
- 로그인·계정 관문이 있는지 — 본 것은 없다(추정).
- ★**사용자의 실제 실패 양상과 버전**★ — 실패 화면 하나로 위 순위가 뒤집힐 수 있다.

---

## 3. 우리와 닿는 자리 (관찰만 — 결정 아님)

- **orca 오케스트레이션의 모양이 우리 것과 같다** — 코디네이터 에이전트가 CLI 를 부르고 `ask`/`reply` 가 있는 우편으로 주고받는다. 우리는 에이전트가 `engram` CLI 와 `eg_send`(`reply_to`)로 같은 일을 한다. ★**상태를 쥐는 자리도 같다**★ — orca 런타임이 Run·Task·Dispatch 와 메시지를 기록하고 라우팅하듯, 우리도 **데몬이 우편을 쥔다**(메시징 커널을 데몬이 호스팅하고 — ADR-0110 — 배달 기록·파킹·회신 계약을 커널이 관리한다: `crates/engram-dashboard-messaging/src/service.rs`). 「앱이 스케줄하지 않고 에이전트가 CLI 로 몬다」는 **우리만의 선택이 아니다.** herdr 도 README 가 같은 모양을 적는다 — 에이전트가 CLI·socket api 로 페인을 띄우고 서로 프롬프트를 보내고 상대가 막힐 때까지 기다린다(`README.md:34` — 소스 확인).
- **herdr 는 여전히 데몬 모델의 가장 가까운 피어다**(CLAUDE.md 「참조 구현」). 이번에 달라진 것은 **돌아가는 바이너리가 생겼다**는 것뿐이다.

---

## 4. 안 한 것

- herdr TUI 기동 · 실제 에이전트 붙이기 · 떼기/다시 붙기 — 전부 미실행.
- herdr ConPTY 번들 굽기(`scripts/package_windows_conpty.ps1`) — 미실행.
- orca 앱을 이 문서가 직접 몰지 않았다 — 기능 지도는 한 달 전 클론 + README 독해다.
