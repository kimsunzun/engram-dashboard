# Action buttons in notification banners — open-source / industry convention

- Status: done (2026-11-05). Tier: light (one collector, main spot-check of the WinUI InfoBar claim). Decision: pending with the user (version-mismatch 「데몬 끄기」 placement — ADR-0343 follow-up).
- Question: is a dismissible top-of-window banner notice-only, or does it conventionally carry an action button (Restart / Reconnect / Stop)? Our ADR-0180 decision 1 says the band only informs.

## Findings (collector output, verbatim below)

## Conclusion (first)
- Yes. An action button on a top-of-window dismissible banner is conventional (certain, for the 4 systems I confirmed: WinUI InfoBar, libadwaita AdwBanner, KMessageWidget, Primer Banner). Banners are not notice-only.
- Count: usually ONE primary action (WinUI: one ActionButton slot, more only via custom content; libadwaita: exactly one button). Primer allows up to 2 (primary + secondary). KMessageWidget allows N (addAction, one button each). Material: not confirmed (fetch failed).
- Microsoft's own example is "Update available. Restart the application to apply the latest update." with the InfoBar pattern, and "Reconnect to continue working." + action button (Network Settings). Action-to-resolve-state banners are an intended use.
- Destructive actions: I found NO explicit design-system guidance on stopping a service that ends running work from inside a banner (unknown). What I did find: WinUI says to use ContentDialog "for scenarios where the application is confirming a user action, showing information the user must read" and when severity blocks the app. A reasonable inference (my interpretation, not a quote): banner button = entry point; if it ends running work, follow with a confirm dialog or state consequences in the banner text.

## Table
| System/app | Action button allowed/used | Limit | Quote | Confidence | URL |
|---|---|---|---|---|---|
| Fluent/WinUI InfoBar | Yes, ActionButton slot (Button or HyperlinkButton) | 1 slot; extra buttons via custom Content (appear below message) | "the option to include your own call to action or hyperlink button"; "Use an InfoBar control when a user should be informed of, acknowledge, or take action on a changed application state." | certain (full page read) | https://learn.microsoft.com/en-us/windows/apps/design/controls/infobar |
| WinUI InfoBar, when NOT to use | Dialog for must-read / confirming | n/a | "For scenarios where the application is confirming a user action, showing information the user must read, use a ContentDialog." Also: "Do not use an InfoBar ... for time-sensitive alerts, or for non-essential messages." | certain | same |
| WinUI InfoBar, close | Close X default; IsClosable=false allowed while unresolved | n/a | "If the notification is required to be in view until the status is resolved ... set the IsClosable property to false." Sample: Title "Update Available" with restart wording | certain | same |
| libadwaita AdwBanner | Yes, optional single button | 1 | "Banners can optionally have a button with text on it, set through AdwBanner:button-label" (page read via summarizer; verbatim as returned) | likely (summarizer-relayed; original not re-read) | https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/class.Banner.html |
| KDE KMessageWidget | Yes, actions become buttons | N (addAction per action) | "Add action to the message widget. For each action a button is added to the message widget in the order the actions were added." Class: "A widget to provide feedback or propose opportunistic interactions." | likely (header comment via summarizer) | https://invent.kde.org/frameworks/kwidgetsaddons/-/raw/master/src/kmessagewidget.h |
| GitHub Primer Banner (React) | Yes: Banner.PrimaryAction + Banner.SecondaryAction; dismiss shown when onDismiss given | 2 | (paraphrase by summarizer; no verbatim) "used to highlight important information" | likely (summarized; original verbatim not confirmed) | https://primer.style/components/banner/react/latest |
| Material Design banner | Unknown: both fetches failed/empty. (My memory says M2 banners have 1-2 actions; uncertain, not sourced.) | unknown | none | uncertain | https://m2.material.io/components/banners (no content obtained) |
| Apple HIG | Unknown: fetch returned title only | unknown | none | unknown | https://developer.apple.com/design/human-interface-guidelines/alerts |
| JetBrains notifications | Search snippet only: notifications carry actions, plugin-update flow has a "Restart IDE" button; guidance page 404 | unknown | none verbatim | uncertain, original not confirmed | https://jetbrains.github.io/ui/controls/notifications (404 on fetch) |
| VS Code / Firefox "Restart to Update" | Snippets only: VS Code shows "Restart to update..." as menu item; Firefox About page shows "Restart to Update" button. Not banner evidence | n/a | none | uncertain, original not confirmed | search results only |
| Daemon/service version-mismatch banner with action | Not found in any source | unknown | none | unknown | none |
| Atlassian, Carbon, Docker Desktop, GitHub Desktop, Slack/Discord, Chrome infobars | Not searched/fetched (budget) | unknown | none | unknown | none |

## Gaps
- Material, Apple HIG, Atlassian, Carbon, JetBrains guideline text not obtained.
- No real-app evidence for a banner with a "restart service" button; VS Code/JetBrains/Firefox claims are snippets only.
- No source addresses destructive action in banner vs dialog directly; the confirm-dialog advice above is inference.
- Fetch summarizer outputs for libadwaita, KDE, Primer were relayed through a small model; treat quotes as likely, not exact.

Most expensive task this run: batch of 8 parallel WebFetch calls (4 of which returned 404/empty).
