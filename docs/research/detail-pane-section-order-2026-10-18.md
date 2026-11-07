# Section-order practice in a detail pane — content · location · references (2026-10-18)

| Item | Value |
|---|---|
| **Status** | Survey done · **decided 2026-10-19** — in both the right pane and the detail card: title → location (a short single line under the title) → content (preview · description · reason …) → references (at the very bottom) (user 「변수가 작은 짧은 줄이 위에 오는 게 좋다」 — PRD R15) |
| **Method** | `/research` light — 1 collector (6 web searches · mostly snippets) + main grounding (original text of Finder · Obsidian confirmed) |
| **Question** | In the detail pane that appears when an item is selected: ① is the path attached under the title or put in a metadata group ② is there a rule of putting lists of uneven length (references · backlinks) at the very bottom ③ do design guidelines have an ordering rule |

## Conclusion

- **There is no established rule** (likely — no rule on detail-pane order was found in Apple HIG · Fluent · Material · NN/g. This means it was not found, not a proof that none exists).
- **Two weak patterns:**
  1. **The path goes inside the metadata group, near the top** — Finder's 「정보 가져오기」 (Get Info) puts 「위치(Where)」 inside the 「일반」 (General) group right after the header (name · icon · size · modified date), and the preview is far below (after 「다음으로 열기」) (certain — original text confirmed). The Google Drive detail pane also puts 「위치」 as a field of the details tab (uncertain — third-party blog).
  2. **The reference list goes at the very bottom** — Obsidian can place backlinks (linked mentions · unlinked mentions) at the bottom of the note. However the default is a right sidebar tab (certain — official help).
- Unreal views references not as a detail-pane section but in a separate window (Reference Viewer) (likely) — not used as grounds for the order.
- NN/g progressive disclosure (show what matters first and the rest on request) is a general principle, so it only weakly supports 「내용 먼저」 and says nothing about the place of the path or references (snippet — original text unchecked).

## Peers

| Peer | Block order | Path position | Source | Confidence |
|---|---|---|---|---|
| macOS Finder Get Info | header → 일반 (General) → 추가 정보 (More Info) → 이름 및 확장자 (Name & Extension) → 설명 (Comments) → 다음으로 열기 (Open with) → 미리 보기 (Preview) → 공유 및 권한 (Sharing & Permissions) | 「위치」 (Where) inside 「일반」 — the full path | https://eclecticlight.co/2026/06/09/reading-the-finders-get-info-dialog/ | Certain (main confirmed the original text) |
| Obsidian backlinks | note body → linked mentions → unlinked mentions (when bottom display is selected) · default = right sidebar | Not applicable | https://obsidian.md/help/plugins/backlinks | Certain (main confirmed the original text) |
| Google Drive detail pane | Details · Activity tabs — a 「위치」 field in the details tab (folder link) · order relative to the preview unconfirmed | A field of the details tab | https://alicekeeler.com/2016/08/30/google-docs-find-document-details/ | Uncertain |
| Unreal Content Browser | References = a separate Reference Viewer window | Unconfirmed | https://dev.epicgames.com/documentation/en-us/unreal-engine/reference-viewer-in-unreal-engine | Uncertain |
| Windows Explorer details pane · Unity · Adobe Bridge · Figma · Notion · VS Code/JetBrains · OneDrive · Dropbox | Not found | — | — | Unknown |

## For us

The current briefing layout = content (preview · description · reason) → location → references. Following the two weak patterns gives **title → location → content → references** (the path is raised to metadata near the header and references stay at the very bottom). It is two precedents, not a rule, so it is a user decision.

## Limits · gaps

- light — no adversarial review · mostly search snippets. The original texts of the Windows details pane · Figma · Fluent · Material were not looked at.
