# qa 바인딩 부록 — engram-dashboard

`qa.md` 가 가리킬 때만 연다. 임시 폴더(`$QA_TMP`) · 분리 실행 · 판정 규칙은 `qa.md` 를 따른다.

## 전체 회귀 — CI 세 잡 로컬 실행

qa 의 전체 회귀다. `ci.yml` 의 세 잡을 로컬에서 돈다. 잡끼리는 순차로 돈다.

### backend 잡

분리 실행으로 하나씩 돈다:

```bash
cargo build
cargo test --workspace -- --test-threads=4
cargo test -p engram-dashboard-transport --all-features --test ws_dial -- --list
cargo test -p engram-dashboard-transport --all-features --test ws_reject -- --list
cargo test -p engram-dashboard-transport --all-features --test ws_write_deadline -- --list
cargo check -p engram-dashboard --features snap-overlay
cargo test -p engram-dashboard-platform --features snap-overlay -- window::snap --test-threads=4
```

- 그 밖에 `qa.md` 「모듈별 명령」의 crate 줄을 전부 돈다(셸 타깃 다섯 · net 두 줄 · transport 세 줄 포함). daemon 줄은 뺀다 — backend 잡에 따로 없고 워크스페이스 줄이 이미 돈다.
- `--list` 세 줄은 줄마다 `: test` 로 끝나는 줄이 1개 이상이어야 PASS 다.
- `window::snap` 줄은 `test window::snap::` … `ok` 줄이 1개 이상이어야 PASS 다.
- 로컬 워크스페이스 회귀는 실 claude 시험을 빼지 않는다 — claude 가 없거나 미로그인이면 그 시험은 수단 없음 PARTIAL 이다(`qa.md` 「claude · codex 레인」과 같다).
- 빠진 스텝이 없는지 backend 잡 스텝 이름과 대조한다:
  ```bash
  PYTHONIOENCODING=utf-8 py -c "import yaml; w=yaml.safe_load(open('.github/workflows/ci.yml',encoding='utf-8')); [print(s['name']) for s in w['jobs']['backend']['steps'] if 'run' in s]"
  ```

### 생성물 동기 (ts-rs)

backend 잡 스텝 `Gate: ts-rs bindings are in sync with the wire contract` 의 로컬판이다. backend 잡을 돌기 전에 뜬다:

```bash
B="crates/engram-dashboard-protocol/bindings crates/engram-dashboard-agent/bindings src-tauri/bindings"
git ls-files -co -- $B | while read -r f; do echo "$(git hash-object "$f") $f"; done > "$QA_TMP/bindings.before"
```

- `--exclude-standard` 를 붙이지 않는다 — 무시된 생성물도 대조에 들어야 한다(CI 가 `git add -N -f` 를 쓰는 이유와 같다).
- backend 잡을 돈 뒤 같은 명령을 `bindings.after` 로 다시 뜨고 `diff "$QA_TMP/bindings.before" "$QA_TMP/bindings.after"` 가 비면 PASS 다.
- diff 가 있으면 FAIL 이다 — 재생성된 파일은 결과물이라 되돌리지 않고 목록을 보고한다.
- `git add -N` 으로 재지 않는다 — 사용자 인덱스를 바꾼다.

### gates 잡

라이선스 게이트가 `node_modules` 를 읽는다(CI 는 이 잡에서 `npm ci` 를 돈다 · 아래 `skip` 이 그 스텝을 뺀다). 먼저 `node_modules` 를 맞춘다:

- `npm ls --depth=0` 이 빠짐(`missing`) · 어긋남(`invalid`)을 찍을 때만 분리 실행으로 `npm ci` 를 돈다.
- 이 워크트리의 vite 가 떠 있으면 `npm ci` 를 돌지 않는다 — vite 가 쥔 `esbuild.exe` 때문에 `EPERM` 으로 죽고 `node_modules` 가 반쯤 지워진다. 확인 = `qa.md` 「띄우기 전 점검」 넷째 줄(포트와 무관). 떠 있으면 끄지 않고 사용자에게 알린다.
- 변경이 `package.json` · `package-lock.json` 을 건드리면(`git diff --name-only <기준점>`) 따로 만든 사본 `$QA_TMP/npm` 에서도 분리 실행으로 `npm ci` 를 돈다 — 설치된 패키지가 멀쩡하면 위 조건이 `npm ci` 를 건너뛰어 불일치가 안 드러난다. 사용자 트리에서는 이 이유로 `npm ci` 를 돌지 않는다.
- 그 사본은 `qa.md` 「변이 확인 — 격리 사본」 세 줄의 `$QA_TMP/wt` 를 `$QA_TMP/npm` 으로 바꿔 만들고, `npm ci` 판정 직후 `git worktree remove --force "$QA_TMP/npm"` 으로 지운다 — `$QA_TMP/wt` 는 변이 확인 몫이라 쓰지 않는다.
- `npm ci` 가 `EUSAGE`(`package.json` · `package-lock.json` 불일치)로 죽으면 사용자 트리든 사본이든 FAIL 이다 — CI 도 같은 데서 죽는 저장소 결함이다.

그 뒤 `ci.yml` 의 bash 스텝을 뽑아 현재 셸의 서브셸에서 그대로 돈다(하네스의 `rg` 는 셸 함수라 새 `bash` 에는 없다):

```bash
PYTHONIOENCODING=utf-8 py -c "
import sys, yaml
w = yaml.safe_load(open('.github/workflows/ci.yml', encoding='utf-8'))
skip = ('Resolve and assert', 'Install ', 'Verify ripgrep', 'npm ci')
n = 0
for s in w['jobs']['gates']['steps']:
    name = s.get('name', '')
    if 'run' not in s or s.get('shell') != 'bash' or name.startswith(skip):
        continue
    n += 1
    open('%s/gate-%02d.sh' % (sys.argv[1], n), 'w', encoding='utf-8').write('# ' + name + '\n' + s['run'])
" "$QA_TMP"
for f in "$QA_TMP"/gate-*.sh; do ( set -eo pipefail; RUNNER_TEMP="$QA_TMP"; source "$f" ) > "$f.log" 2>&1; echo "rc=$? $(head -1 "$f")"; done
```

- 줄마다 `rc=0` 이어야 PASS 다. `rc≠0` 이면 `<스텝>.log` 를 읽어 게이트 위반인지 도구 부재인지 가른다.
- 도구 부재 = PARTIAL — `rg` 의 PCRE2(`rg --pcre2-version`) · `cargo-about` 0.9.2 · 환경 탓 `npm ci` 실패(네트워크 · npm 없음 · 위 vite 가 떠 있어 못 돔).
- 이 블록은 checkout 의 `ci.yml` 셸을 실행한다 — 믿는 로컬 checkout 에서만 돈다.

### frontend 잡

- `npm ci` 는 「gates 잡」의 세 줄을 그대로 따른다 — 낡은 `node_modules` 는 tsc · vitest 거짓 FAIL 을 낸다.
- 분리 실행으로 `npm test` · `npx tsc --noEmit`.

## GUI 조리법

### cdp.mjs

- 하위 명령은 셋뿐이다 — `info`(창 목록) · `eval "<js>"`(결과 JSON · 식이 돌려준 Promise 는 기다린다) · `shot <png>`.
- `eval` 은 맨 위 `await` 를 받지 않는다(SyntaxError) — `(async () => { … })()` 로 감싼다.
- 환경변수 = `CDP_PORT`(기본 9223) · `CDP_MATCH`(창 url · title 정규식 — 팝아웃 창을 집는다 · 기본 = main 창).
- 영역 캡처는 없다 — 위치 · 크기 · 색 · 보임은 `eval` + `getBoundingClientRect()` · `getComputedStyle()` 숫자로 잰다.
- `shot` 경로는 `$QA_TMP/<이름>.png` 로 준다 — 파일명만 주면 저장소의 `_wip/shots/` 에 떨어진다.
- 클릭 · 휠 · 키 · 끌기 같은 신뢰 입력은 CDP `Input.*` 로 넣는다 — `cdp.mjs` 에 없으니 드라이버를 `$QA_TMP` 에 짠다.
- 페이지에 넣을 JS 는 Write 도구로 파일에 쓴다 — bash heredoc 은 역슬래시를 먹는다.

### 프론트 명령 레지스트리 — `window.__engramCmd`

- 명령 목록은 앱에게 묻는다: `node scripts/cdp.mjs eval "window.__engramCmd.list().map(c=>c.id).sort()"`.
- 실행 = `window.__engramCmd.run('<id>', { … })`.
- 에이전트 띄우기 = `run('agent.spawnInto', { cwd: '<$QA_TMP 아래 폴더>', backend: 'claude' | 'codex' })` → 새 AgentId. 끄기 = `run('agent.kill', { agentId })`.
- 칸에 에이전트 올리기 = `run('layout.setSlotContent', { viewId, slotId, content: { type: 'agent', agent_id } })` — `viewId` · `slotId` 는 DOM 의 `[data-view-id]` · `[data-slot-id]`.
- 챗(JSON) 에이전트 출력은 칸에 올린 뒤 그 칸 DOM 에서 읽는다 — 칸 없는 view 로 구독하면 `buffering` 에 멈춘다.
- 터미널 칸 글자 = `run('slot.domMode.enable', { slotId })` 뒤 그 칸 `innerText`(평문만 · 부른 창에서만). 되돌리기 = `slot.domMode.disable`.
- 터미널 커서 · 글자 색은 이 경로로 못 잰다 — 못 쟀다고 보고한다.
- cwd 같은 경로 인자는 슬래시로 쓴다.

### 클라이언트 탈출구 — `window.__ENGRAM_AGENT__`

- 창마다 하나이고 릴리스 빌드에도 있다. 레지스트리에 없는 프로필 일을 여기서 한다.
- `createClaudeProfile(name, cwd, extraArgs, env, autoRestore, outputFormat?)` · `createCodexProfile(…같은 인자)` → 프로필. `spawnProfile(id, resume)` · `deleteProfile(id)` · `setProfileAutoRestore(id, bool)`.
- `outputFormat` 을 빼면 `'Terminal'` 이다 — 레지스트리 경로의 기본(`'StreamJson'`)과 다르다.
- `window.__engram`(store 스냅샷)은 DEV 빌드에만 있다.
- view 캐시(`useViewStore`)는 `__engram` 에 없다 — `performance.getEntriesByType('resource')` 에서 페이지가 받은 `/src/store/viewStore.ts…` URL 을 찾아 그 URL 그대로 `import()` 한다(DEV). 쿼리를 뗀 URL 은 빈 새 스토어를 연다.

### 셸 invoke — `window.__TAURI__.core.invoke`

- 셸 command 를 바로 부른다 — `list_tabs` · `get_view` · `set_split_ratio`(칸 폭 · 비율 하한 0.1) · `assign_agent` · `move_slot_to_window` · `settings_set` · `restore_status`.
- `list_tabs` 는 창 label 을 받는다 — `invoke('list_tabs', { window: 'main' })`. `{}` 면 던진다.
- 데몬 버스 명령(토큰 없이 · main 창) = `invoke('forward_daemon_command', { cmd: { Command: { envelope: { name: '<명령>', request_id: crypto.randomUUID(), owner: '', proto_ver: 1, args: { … } } } } })`.
- IPC 호출을 엿볼 때 `window.__TAURI_INTERNALS__.invoke` 를 바꿔 끼우지 않는다 — 대입이 조용히 무시된다. `window.fetch` 를 감싸 `http://ipc.localhost/<command>` 요청을 본다.
- 어느 호출부가 쐈나 = 감싼 자리에서 `new Error().stack`. 줄번호는 vite 변환본 기준이다 — `fetch('/src/<경로>.tsx')` 로 변환본을 받아 맞춘다(DEV).
- 챗 스타일 값은 인라인 값으로 잰다 — `document.documentElement.style.getPropertyValue('--chat-font-size')`. `getComputedStyle` 은 테마 fallback 때문에 적용 경로가 죽어도 통과한다.

### 짧은 상태 · 실 에이전트

- 1초 미만으로 사는 화면 상태는 폴링으로 못 잡는다 — 페이지에 `MutationObserver` 기록기를 먼저 걸고 동작을 일으킨다.
- 그 상태를 세워 두고 재려면 칸에 올리기 전에 `window.__ENGRAM_AGENT__.subscribeOutput` 을 감싸 이력 청크를 붙잡았다가 순서대로 푼다.
- 이 PC 의 WebView 는 `prefers-reduced-motion: reduce` 를 보고한다 — 움직임은 CDP `Emulation.setEmulatedMedia`(`prefers-reduced-motion` = `no-preference`)를 건 뒤 잰다.
- 구독이 나갔나는 셸 · 데몬 로그에 안 남는다 — 그 이벤트(예: `usage-limits-updated`)를 `window.__TAURI__.event.listen` 으로 듣고 첫 스냅숏 도착으로 본다.
- 새 패키지를 더한 뒤 첫 기동은 vite 가 의존성을 다시 최적화해 main 창이 `Invalid hook call` 로 빌 수 있다 — `location.reload()` 한 번.
- codex 승인 거절 시험의 쓰기 대상은 `%TEMP%` 밖에 둔다 — `$QA_TMP` 도 `%TEMP%` 안이라 승인 요청이 서지 않는다. 그 폴더는 뒷정리 목록에 더한다.

### 에이전트 입출력

- claude · codex 는 입력 전 출력 0건 · 뷰 `buffering` 이 정상이다 — 실측 단위는 스폰이 아니라 입력 → 응답이다.
- 터미널 에이전트 입력 = `window.__ENGRAM_AGENT__.writeStdin(id, new TextEncoder().encode('<글>'))`. codex TUI 는 `'\r'` 을 따로 한 번 더 쓴다 — 함께 쓰면 입력창에 남는다.
- 챗 칸 입력 = 그 칸 `textarea` 에 native setter(`Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set`)로 값을 넣고 `input` 이벤트 뒤 Enter `keydown`(둘 다 `bubbles: true`). 빈 칸 표식 = `[data-rich-empty]`.
- 에이전트가 실제로 친 명령은 칸 글자가 아니라 세션 기록에서 읽는다 — claude = `~/.claude/projects/<cwd 슬러그>/<세션>.jsonl` 의 tool_use · 결과, codex = `~/.codex/sessions/` rollout(읽기만).
- 한글이 든 프로세스 명령줄은 콘솔로 읽지 않는다 — `Get-CimInstance Win32_Process … | Out-File -Encoding utf8 "$QA_TMP/<이름>.txt"` 로 떨궈 파일을 읽는다(콘솔 코드페이지가 깨뜨린다).

## 창 조작 · 세션

- 창 · 포커스 · 화면 캡처 · 네이티브 입력을 재기 전에 세션이 잠기지 않았는지 본다 — `powershell -NoProfile -Command "Get-Process LogonUI -ErrorAction SilentlyContinue"` 가 무엇이든 찍으면 잠김이다(미검 판별). 잠겼으면 그 실측은 수단 없음 PARTIAL 로 일찍 멈춘다.
- 창 옮기기 · 최대화 · 최소화 · 닫기 · 전경 판정은 웹뷰 JS 로 안 된다(capability 가 막는다).
- 숨은 창 판정 = 그 창(`CDP_MATCH`)에서 `window.__TAURI__.window.getCurrentWindow().isVisible()`. `document.visibilityState` 는 숨긴 창에서도 `visible` 이다.
- PowerShell `Add-Type` P/Invoke 는 훅이 막는다 — C# 헬퍼(user32 `SetWindowPos` · `ShowWindow` · `GetForegroundWindow` · `EnumWindows`)를 `$QA_TMP` 에 짓는다:
  ```bash
  MSYS_NO_PATHCONV=1 /c/Windows/Microsoft.NET/Framework64/v4.0.30319/csc.exe /nologo /out:"$(cygpath -w "$QA_TMP/win.exe")" "$(cygpath -w "$QA_TMP/win.cs")"
  ```
- csc 에는 역슬래시 경로를 준다 — 슬래시 소스 경로는 `CS1504`(파일을 못 연다)로 죽는다.
