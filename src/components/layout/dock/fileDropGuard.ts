// TRD S21-slot-docking §4-1 · §8-5 M10 · PRD AC22: 우리 창에 파일 · URL 을 끌어 놓으면 웹뷰가 그것으로 탐색해 앱 화면이 사라진다 — 그 놓기를
//   막는다. 늘 켜 둔다(끌기 세션과 무관).
// ★파일 · URL 을 실은 끌기만 막는다★ — 드롭 처리기를 끈 창에서 웹뷰가 놓기로 탐색하는 것은 그 둘뿐이다(Chromium
//   `DragData::ContainsURL`). 다른 끌기까지 막으면 입력창(챗 작성란 등)에 글자를 끌어 놓는 기본 동작이 거절된다.
// ★끊지 않는다(`stopPropagation` 없음)★ — 에이전트 트리의 react-dnd HTML5 백엔드가 뿌리 `window` 의 버블 리스너로 자기 끌기를
//   받는다.

/** 이 문서에 막기를 단다. 반환 = 떼기. */
export function installFileDropGuard(): () => void {
  // ★`dragenter` 도 막는다★ — Chromium 은 커서 밑 요소가 바뀐 걸음에 `dragenter` 만 낸다(`EventHandler::UpdateDragAndDrop`).
  //   그것을 안 막으면 그 걸음 동안 문서가 끌기를 안 받는 것으로 쳐져 파일 · URL 에 copy 가 서고, 그 틈(~16–60 ms)에 떼면
  //   `drop` 사건 없이(`PerformDrop`) 탐색으로 간다 — 아래 `drop` 막기가 안 돈다.
  const onEnterOver = (e: DragEvent): void => {
    const dt = navigatingTransfer(e)
    if (dt === null) return
    e.preventDefault()
    dt.dropEffect = 'none'
  }
  const onDrop = (e: DragEvent): void => {
    if (navigatingTransfer(e) !== null) e.preventDefault()
  }
  document.addEventListener('dragenter', onEnterOver)
  document.addEventListener('dragover', onEnterOver)
  document.addEventListener('drop', onDrop)
  return () => {
    document.removeEventListener('dragenter', onEnterOver)
    document.removeEventListener('dragover', onEnterOver)
    document.removeEventListener('drop', onDrop)
  }
}

function navigatingTransfer(e: DragEvent): DataTransfer | null {
  const dt = e.dataTransfer
  if (dt === null) return null
  const types = Array.from(dt.types)
  return types.includes('Files') || types.includes('text/uri-list') ? dt : null
}
