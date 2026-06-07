# Step 9 — 슬롯 팝업 분리

## 할 일

### 1. `src-tauri/tauri.conf.json` 수정
- `windows` 배열에 팝업 창 설정 추가:
  ```json
  {
    "label": "slot-popup",
    "url": "index.html#/popup",
    "visible": false,
    "width": 800,
    "height": 600,
    "decorations": true,
    "title": "Slot Popup"
  }
  ```

### 2. `src/pages/PopupPage.tsx` 생성
- route `/popup` 에 마운트
- URL 쿼리 파라미터 `?slotId=1` 읽어서 해당 슬롯의 `<TerminalSlot />` 렌더링
- 더미: slotId 무관하게 같은 더미 터미널 출력

### 3. `src/App.tsx` 에 라우터 추가
- `react-router-dom` 설치: `npm install react-router-dom`
- `/` → 기존 AppLayout
- `/popup` → PopupPage

### 4. `src/components/slot/SlotContextMenu.tsx` 수정
- "팝업으로 분리" 메뉴 항목 추가
- 클릭 시 `window.open('index.html#/popup?slotId=1', '_blank')` (Tauri WebView 새 창)

## 완료 기준
- `npm run build` 에러 없음
- 완료 후 `orch 4 "⟁dc29 step9 완료"` 로 보고
