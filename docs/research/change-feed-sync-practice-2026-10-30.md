# Change-feed sync practice — how multiple clients each catch up (2026-10-30)

- **Status:** `/research` light — collector 1 · main grounding spot-check (read the Microsoft Graph `driveItem: delta` document source text directly) · no adversarial review (light). Awaiting decision — whether to keep the S25 move record (`library-moves.json` · M10) as is.
- **Background:** Each UI holds its own collection (G33 — multiple UIs can attach) · library files are plain text, so there is no fixed id (G33). Current design = the daemon numbers 「옛 경로 → 새 경로」 and keeps the most recent 1,000 (the record id changes if the root changes) · the UI pulls 「커서 N 뒤로」 · if truncated, the line is 「찾을 수 없음 · 다시 연결」.
- **Confidence legend:** certain = primary document checked directly · likely = primary document summary · SDK docs · forum · unknown = not found.

## Conclusion

1. **All five peers surveyed share the same frame** — the server does not know the clients; the client holds a 「어디까지 읽었나」 token itself and pulls 「그 뒤로 바뀐 것」; if the token is too old, the server reports an error and makes the client rescan from the start. **The overall frame is the same as our design.**
2. **Difference one — peers are item-id based.** A rename or move comes out not as an event but as 「id X 의 최신 경로는 Y」 (OneDrive · Drive). OneDrive states 「항상 id 로 추적하라」 and 「폴더 이름을 바꿔도 하위 항목은 안 나온다」 (certain). **We have no id, so we can only record 「옛 경로 → 새 경로」 pairs** — the same frame moved into a path world.
3. **Difference two — recovery after truncation.** Peers rescan everything and compare against local state to recover on their own (they have ids, so they can match up). **Without ids, even if we rescan we cannot tell where the old path went, so 「찾을 수 없음 · 다시 연결」 remains the only way** — and R19 has already decided on 「훑기는 후보를 찾아 제안하지 않는다」.
4. **What corresponds to our 「기록 id」 (when the root changes, every cursor is invalid)** = OneDrive 「서버 상태가 바뀌어 새 토큰이 필요하면 410」 · Dropbox 「reset」 (using the same cursor against a different root is also a cause of reset). There is no separate named field, but the role is the same.
5. No peer publishes a retention period or a count cap — 1,000 is our own choice.

## Per peer

| Peer | Token | Token too old | Move representation | Whole-invalidation signal |
|---|---|---|---|---|
| **Microsoft Graph / OneDrive delta** | opaque token (inside the deltaLink URL) · held by the client · `token=latest` (certain) | `HTTP 410 Gone` + a new link that rescans from the start · compare with local state after the full scan — two error codes (replace with the server's / upload the local differences) (certain) | id based · latest state per item (changing twice shows once) · renaming a folder does not return descendants · 「항상 id 로 추적」 (certain) | 「서버 상태가 바뀌어 새 토큰 필요」 is also 410 (certain) |
| **Google Drive changes** | `getStartPageToken` → `changes.list` → `newStartPageToken` · held by the client (certain) | unknown | `fileId` + new metadata — id based (likely) | unknown |
| **Dropbox list_folder** | opaque cursor · `list_folder/continue` (likely) | `409 reset` → call `list_folder` again for a new cursor (likely) | unknown | reset — switching the path root is also a cause (likely) |
| **CouchDB `_changes`** | `since=<seq>` · seq is opaque since 2.0 · held by the client (certain) | unknown | no rename — the document id is the key, so delete + new document (certain) | unknown |
| **Apple CloudKit** | per-zone opaque change token · held by the client (certain) | `changeTokenExpired` → discard the token and start over (likely) | id based · moves unknown | unknown |

Sources:
- Microsoft Graph — https://learn.microsoft.com/en-us/graph/api/driveitem-delta (main checked directly 2026-10-30: 「the service returns an `HTTP 410 Gone` error … and a `Location` header containing a new nextLink that starts a fresh delta enumeration from scratch」 · 「The delta feed shows the latest state for each item, not each change」 · 「renaming a folder doesn't result in any descendants of the folder being returned from delta. When using delta you should always track items by id」 · the 410 reason includes 「if server state has changed and a new token is required」)
- Google Drive — https://developers.google.com/workspace/drive/api/reference/rest/v3/changes/list · https://developers.google.com/workspace/drive/api/guides/manage-changes
- Dropbox — https://dropbox.github.io/dropbox-sdk-java/api-docs/v4.0.0/com/dropbox/core/v2/files/ListFolderContinueError.html · https://www.dropboxforum.com/t5/Dropbox-API-Support-Feedback/409-reset-error-for-list-folder-continue-api-from-business/m-p/286174
- CouchDB — https://docs.couchdb.org/en/stable/api/database/changes.html
- CloudKit — https://developer.apple.com/documentation/cloudkit/ckfetchrecordzonechangesoperation

## For us

- **The move record is the standard change-feed frame moved into an id-less path world** — the UI holds the cursor and the daemon does not know the UI · if truncated, it gives a signal (our `truncated` = the 410 / reset slot) · if the root changes, the record id changes (= 「서버 상태가 바뀌어 새 토큰 필요」).
- **Unlike peers, automatic recovery is not possible** — because there is no id. So after truncation it is 「찾을 수 없음 · 다시 연결」 (the same path as R19).
- With multiple roots (G25), there is one record and one record id per root.

## Gaps · what could not be confirmed

- Drive token expiry behavior and retention period, Dropbox move representation and retention period, and the effect of CouchDB compaction on the feed — not found.
- Syncthing · rsync · git were not examined.
- No peer was found that publicly uses a 「작은 상한 + 연속 번호」 record — all use an id-based latest-state feed or a per-document seq.
