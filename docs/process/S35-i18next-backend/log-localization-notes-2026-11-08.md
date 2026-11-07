# Log / error message localization in OSS: collection 1

Method note: WebFetch/WebSearch only (WebFetch answers come from a small summarizing model, so quotes are "as returned"; originals not re-read by me). ~16 calls. Sample is small; no distribution claim beyond it.

## Key conclusions

1. **Unreal Engine (likely, not certain).** `UE_LOG` is not a localization path. Docs show `UE_LOG(LogTemp, Warning, TEXT("..."))` with `TEXT()` only. The Logging page says nothing about localization. Localization goes through `FText` (`LOCTEXT`/`NSLOCTEXT`), documented as "user-facing text that needs to be localized". `FMessageLog` (editor Message Log, feedback to artists/designers; also echoes to Output Log) takes `FText`, and the community wiki says "you must use LOCTEXT() in a production project". So the user's memory is half right: UE localizes user-visible diagnostics (Message Log, notifications) but not `UE_LOG` output. I did not find an Epic page stating the "UE_LOG is not localized" rule explicitly. The inference rests on the absence of any mention plus the `TEXT()`-only examples. Notifications and error dialogs were not fetched (unknown, but they take `FText` by the same convention; original not checked).
2. **Typical OSS split (likely, within sample).** Logs and developer-facing errors stay in one language (English). User-facing errors are localized. The mechanism is almost always: a stable code or key travels with the error, and the text is looked up in a catalog at the display boundary. The catalog sits in the UI/client (Matrix, GitLab API) or in the backend's translation catalog keyed by id (Mattermost `AppError` id, Home Assistant `translation_key`).
3. **Counter-examples exist but are retreating.** OpenStack localized log messages (`_LI/_LW/_LE/_LC`) and then removed it in 2016-17. Mattermost translated server logs and DB errors and voted to stop.
4. **Rationale (stated):** operators search the web for English strings; technical text is hard to translate well; support tickets arrive in English; log aggregation and grep need one language. Sources below.

## Per-source evidence

### Unreal Engine
- Text localization page: https://dev.epicgames.com/documentation/en-us/unreal-engine/text-localization-in-unreal-engine. FText is "user-facing text that needs to be localized". `FText -> FString` is lossy; `FText::AsCultureInvariant`/`INVTEXT` exist for non-localizable text. No mention of `UE_LOG`.
- Logging page: https://dev.epicgames.com/documentation/en-us/unreal-engine/logging-in-unreal-engine. Examples use `TEXT()`; no localization discussion.
- FString page: https://dev.epicgames.com/documentation/en-us/unreal-engine/fstring-in-unreal-engine. FString to FText does not get "auto localization"; no explicit log guidance.
- FMessageLog: https://dev.epicgames.com/documentation/en-us/unreal-engine/API/Runtime/Core/FMessageLog and wiki https://unrealcommunity.wiki/message-log-4wzqj97j (snippet-only for the wiki claim). It displays in the editor Message Log and Output Log; the example is `FMessageLog("MyModule").Error(LOCTEXT("InvalidValue","..."))`.

### Ecosystem guidance
- .NET `ILogger` docs (https://learn.microsoft.com/en-us/dotnet/core/extensions/logging): message templates plus structured placeholders plus `EventId` (numeric IDs). No localization guidance appears at all. The design leans on templates and event IDs for machine filtering, not on translated text. This is an absence, not an explicit "don't localize".
- Rust: no official guidance found. Community pattern (only a crate example, `limiteron`, https://docs.rs/crate/limiteron/0.3.0-rc.7/source/src/i18n/mod.rs, snippet-only): the `Display` impl stays canonical English for logs/tracing, and a separate `to_localized_string()` uses a Fluent catalog. `thiserror`/`anyhow`/`tracing` themselves have no i18n. Treat as uncertain/low-weight.
- Python/OpenStack gettext practice: see OpenStack below. GNU gettext manual and Apple/Android guidance: not found (gap).

### Concrete OSS apps
1. **Mattermost** (Go server + React UI). The server had `AppError` with translated ids, and also translated logs/DB errors. Forum thread (https://forum.mattermost.com/t/were-considering-a-change-to-mattermost-translations-and-we-want-to-hear-from-you/12445, fetched summary): community voted to stop translating server-side strings invisible to end users (logs, DB errors; also API errors/emails were in the original scope). Rationale quoted: admins search in English; hard to translate technical text; support in English. Snippet (original not checked): errors not visible to users use `model.NoTranslation` as the id. Handbook: https://handbook.mattermost.com/contributors/ways-to-contribute/localization (snippet-only).
2. **Home Assistant** (Python core). Exceptions extending `HomeAssistantError` take `translation_domain`, `translation_key`, `translation_placeholders`; the text lives under `exceptions` in `strings.json` (https://contributors.home-assistant.io/core/integrations/translations/, https://developers.home-assistant.io/docs/core/integration-quality-scale/rules/exception-translations, blog https://developers.home-assistant.io/blog/2024/03/15/exception-translations). Blog snippet says the log message is removed from the exception args so the text comes from the translation cache. Explicit "logs stay English" statement: not found (inferred).
3. **GitLab** (Rails + Vue). Externalization guide (https://docs.gitlab.com/ee/development/i18n/externalization.html): "Messages in the API (`lib/api/` or `app/graphql`) do not need to be externalized". It says nothing explicit about logs.
4. **Matrix / Synapse** (https://spec.matrix.org/v1.8/client-server-api/ and `synapse/api/errors.py`). Errors are `{errcode: "M_FORBIDDEN", error: "<human readable>"}`; clients branch on `errcode`. That clients (Element) localize by errcode is the search tool's inference, not a source quote. Original not checked, so uncertain.
5. **OpenStack** (Nova, Cinder, oslo.*): counter-example turned precedent. Older docs required `_LI/_LW/_LE/_LC` for INFO..CRITICAL logs, none for DEBUG, and `_()` for exception messages (https://docs.openstack.org/nova/ocata/i18n.html). Then "Remove log translations" (https://bugs.launchpad.net/bugs/1674577, commit e.g. https://git.openstack.org/openstack/oslo.concurrency/commit/f4b6622407d7f64ed8c340c1b82f15b18867ad02; ML threads Nov 2016 / Mar 2017 per search summary). Exception messages kept `_()`.
6. **Trilium** (Node server + UI): has a "server translation" doc (https://docs.triliumnotes.org/developer-guide/concepts/i18n/server-translation). I did not open it; unknown scope.
- Not covered (no evidence gathered): VS Code, Grafana (search returned nothing relevant), Nextcloud, Jellyfin, Immich, Signal, Rocket.Chat, Tauri apps. Only 5 apps sampled, short of the 8-12 target.

## Gaps / suggested follow-ups
- Epic notification/dialog APIs (`FNotificationInfo`, `FMessageDialog`) and any explicit Epic statement on `UE_LOG`.
- VS Code (`vscode-nls`/`l10n`; log output channels are English, uncertain), Grafana, Nextcloud (`$l->t()` on server exceptions), Jellyfin, Immich, Rocket.Chat: check with `gh api` on repos.
- Apple/Android/Microsoft globalization guidance on diagnostics vs UI strings.
- Direct source for Rust error-localization practice (e.g. `miette`, Fluent/`fluent-templates` usage in real apps).

Most expensive work: the .NET logging page fetch, which dumped the whole page and added nothing.
