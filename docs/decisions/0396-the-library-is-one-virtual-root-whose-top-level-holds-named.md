# ADR-0396: The library is one virtual root whose top level holds named mounts; references and moves cross mounts

- 상태: 확정 (2026-11-21, 근거: user 2026-11-21 「루트는 가상루트로 최상단 루트고(트리에서는 관리되고 실물은 없는?), 그 밑에는 특정 이름이 있고 근데 그 이름은 나중에 실제 펼칠때 대체되는거고」 · 「그럼 그 폴더간의 레퍼 참조 이동도 되겠네」 · 「이대로 문제는 없는거잖아. 가상이니깐. 만약에 연결 끊기면 수동으로 맞추거나 해당 폴더 되돌린다음에 어떻게 어떻게 하면 되는거고.」 · 「ㅇㅇ 쭉쭉 진행해.」)
- 관련: Amends ADR-0385 (decisions 1 and 2) · Amends ADR-0312 (decisions 2 and 6) · ADR-0315 (path from the top of the library — holds, the top is now the virtual root) · ADR-0320 (trash inside the root) · ADR-0330 (relink) · ADR-0387 (absent skips, unreadable blocks) · parent TRD `docs/process/S25-preset-library/trd.md` §5-2 item 21

## 맥락
ADR-0385 made library roots several and independent: an added root gets a name prefix, an unprefixed path means the first root, and no reference crosses roots. Sizing that design against the code (2026-11-21 survey) found it large — roughly 3–5k lines, about half tests — because the library service, the launch decision and every path spelled to the wire or matched against an agent attachment assume one root: per-root locks and generations, a per-root launch judgement, a root-aware path type at about 50 spelling sites, and the built-in turned into a real root. The user then proposed a different concept: keep one tree, put the root names under a virtual top, and swap a name for its real folder only when touching disk — so references, moves and resolution work as in one folder and only the identity of each top folder differs.

## 결정
1. The library is one virtual root with no physical folder; it exists only in the tree. Its top level holds only named mounts, and each mount maps to a real folder (for example a git clone anywhere on disk). Nothing can be created directly at the virtual root.
2. A reference is a plain path from the virtual root, `<mount>/a/b` — the mount name is its first segment; there is no separate prefix syntax. The mount name is replaced by the mount's real folder only when the path is turned into a disk path.
3. References and moves across mounts are allowed. A reference broken by a mount that is missing, removed or renamed is fixed by hand — relink, or restore the folder.
4. Steps 9a and 10a stay as built; the step's place (after 10a, before 9b) and the step-TRD scope of ADR-0385 decisions 3 and 4 stand, now about mounts: mount name rules, mount management commands, how the current single root and the paths already written become a mount, the effects of removing or renaming a mount, a move across mounts on different drives, trash per mount, overlapping mounts, the first start, and the built-in `engram:/`.

## 거부한 대안
- **Several independent roots with a name prefix (ADR-0385 decisions 1 and 2)** — sized at roughly 3–5k lines (per-root locking and generation, per-root launch judgement, a root-aware path type at about 50 spelling sites), and references between roots are forbidden. Basis: code survey 2026-11-21. User replaced it with this design.
- **Keep one root, one git clone per subfolder** — works today at no cost, but every clone must sit under the same folder. Basis: code (`LibPath` is relative to the one root). Offered to the user, who answered that several paths are needed (「결국 여러 패스는 필요해보이는데」).
- **Mounts beside a base root whose content stays at the top (unprefixed = base)** — keeps paths already written, but a mount name could collide with a top folder of the base and the base needs a rule of its own. (Basis for rejection: weak — 사용자 직접 기각) The user chose a virtual top holding only mounts.
- **Forbid references across mounts** (one check: the first segment must be the preset's own mount) — would keep ADR-0385's 「no interference」 condition. (Basis for rejection: weak — 사용자 직접 기각) The user chose to allow references and moves across mounts.

## 근거
- User decisions 2026-11-21 (quoted in the status line).
- Precedent: Unreal mount points — a mount name and a disk location (`ContentPath`) are registered separately and references use the mount name (`/PluginName/...`), with references across mounts allowed (research `docs/research/multi-library-reference-paths-survey-2026-10-15.md`).
- Code survey 2026-11-21: the disk layer already takes the root as a parameter (`LibPath::to_path`, `from_disk`, `LibraryStore` methods), so mapping the first segment to a folder sits at the disk boundary.

## 영향 / 불변식
- The mount name appears inside shared preset files (`company/skills/review`), so every PC must register a mount under the same name; the folder's place and its disk name are free.
- Renaming a mount is renaming a top folder: references in every mount and agent attachments are rewritten, which changes files in other git repositories.
- Removing a mount removes only the registration; files on disk are not touched. Its items become absent and follow ADR-0387 (absent skips, unreadable blocks).
- ADR-0312 decision 2 (one drive, no move across volumes) no longer holds: mounts may sit on different drives. ADR-0312 decision 6 (prefix only for added roots) is replaced by decision 2 here.
- ADR-0312 decision 3 (outside content is brought into the root) meets decision 1 here, since a mount is any folder; the step TRD states which folders may be mounts and asks the user whether that reopens decision 3 (as ADR-0385 already required).
