# Practice for a problem summary above the edit pane — list all · count · banner (2026-10-18)

| Item | Value |
|---|---|
| **Status** | survey done · **decided 2026-10-19** — keep: under the header of the edit pane · agent preset slot, list every problem on its own line (a problem inside a contained preset is 「<프리셋> › …」) · wording = same as the row marker · clicking goes to that row · the row marker stays · hidden when there is no problem · no collapse threshold (user 「위에 쭉 나열해 유저가 한눈에 볼 수 있도록」 — PRD R51) |
| **Method** | `/research` light — 1 collector (searches + read 4 official pages) + main-session grounding (GOV.UK primary source checked) |
| **Question** | When there are several problems, at the top: ① are all listed one by one, or only a count shown? ② from how many does it collapse? ③ is the per-row marker kept alongside? ④ does clicking go to that spot? |

## Conclusions

1. **The strongest form = list every problem on its own line at the top** (certain) — the GOV.UK error summary puts the title 「There is a problem」 + one link line per error at the very top of the body, and it is always shown even when there is a single error. NN/g also says the summary gives 「모든 오류를 한눈에」. The Unreal compile results and the VS Code Problems panel are per-item lists too. **A count-only form** (the VS Code status bar · the remaining-count badge of a Fluent MessageBar group) is an entrance to the list and does not replace the list (likely).
2. **No collapse threshold exists anywhere** (certain — the GOV.UK primary source has no count limit). When there are several messages, Fluent puts severe ones on top · collapses by choice and shows a remaining-count badge (search summary only — primary source not checked).
3. **The per-row marker stays as it is** (certain) — NN/g 「요약을 유일한 오류 표시로 쓰지 말라」 · GOV.UK 「요약 문구를 입력 옆 문구와 같게」.
4. **Clicking goes to that spot** (certain — GOV.UK each error = a link to that field · Unreal compile results = moves to that node, likely).

## Peers

| Peer | Form | Collapse | Row marker alongside | Click moves | Source | Confidence |
|---|---|---|---|---|---|---|
| GOV.UK Error summary | at the very top · title + one link line per error · shown even for one | no threshold | yes · same wording | yes · that field (the first field if several) | https://design-system.service.gov.uk/components/error-summary/ | certain (main checked primary source) |
| NN/g form error guidelines | summary = the whole picture | none | yes · do not use the summary alone | not mentioned | https://www.nngroup.com/articles/errors-forms-design-guidelines/ | certain (collector read) |
| Fluent 2 MessageBar | stacking · severe on top · collapse by choice with a remaining-count badge | no fixed number | not mentioned | not mentioned | https://fluent2.microsoft.design/components/web/react/messagebar/usage | likely (search summary only) |
| Carbon notification | no guidance on grouping several errors · avoid stacking callouts | — | — | — | https://carbondesignsystem.com/components/notification/usage/ | certain (absence confirmed) |
| Atlassian section message | no guidance on several messages | — | — | — | https://atlassian.design/components/section-message/usage | certain (absence confirmed) |
| Unreal compile results | message list panel | none | unconfirmed | yes · to that node | https://dev.epicgames.com/documentation/unreal-engine/compiler-results-in-the-blueprints-visual-scripting-editor-for-unreal-engine | likely (search summary only) |
| VS Code | status bar = count → clicking opens the Problems panel (full list) | none | yes · squiggly underline | count → panel · Alt+F8 steps through in order | https://code.visualstudio.com/docs/editor/editingevolved | certain (collector read) |

## For us

Under the edit pane header, one line per problem (wording = same as that row's marker) · clicking goes to that row (collapsed panes · contained presets are expanded) · the row marker stays · invisible when there is no problem. The collapse count is not set because practice gives no basis (scroll if long). User decision.

## Limits · gaps

- light — no adversarial review. We did not look at JetBrains inspection widgets · Polaris. The pages read also went through a summarizing tool, so the quotations are not byte-for-byte checked.
