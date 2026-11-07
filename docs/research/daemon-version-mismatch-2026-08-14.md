# Synthesis output — version mismatch between a resident daemon ↔ an independently upgrading client

## Our situation (constraints)
A desktop app + a local resident daemon. The **daemon's reason to exist is to survive the app being closed** — it holds live child processes (agent sessions). So "restart the daemon on upgrade" is not free; it kills what the daemon was meant to protect.
Currently: protocol version = one integer, **compared for equality**. On a mismatch the client rejects that daemon and **does not start a new daemon on top of it either** → until the user kills it by hand, the app cannot attach and agents are stuck. When it was raised from 1→2 in the past, "reconnecting forever" actually happened, and the "popup guide on mismatch" in the original design was never implemented.
We have just added 3 new commands to the protocol (nobody sends them yet), and an old daemon cannot even decode that envelope. The error an old daemon returns has no correlation id, so the sender waits forever.
The frontend has **no modal, banner or toast at all**. By project principle, a new UI feature must be accompanied by an LLM call path.

## F1. Automatic kill+restart is used "only when there is nothing to lose"
- Bazel adopts automatic daemon restart — because its state is a build cache that can be recomputed. (Confidence: certain)
- tmux uses integer equality comparison + **connection refusal**, with no automatic restart. If you kill the server, the sessions simply die. Manual workarounds only. (certain)
- Docker is the only one that **avoids it structurally** — it moves the party holding the containers out of the version-sensitive control daemon (containerd-shim) and sets live-restore, so containers live even if the daemon dies. (certain)
- **Implication:** our daemon is on the "has something to lose" side. So the automatic kill+restart family is unsuitable.

## F2. A single equality comparison of a version number survives only when a cutover is intended
- The mainstream in practice = **min/max range negotiation at the format level + capability negotiation on top + timeouts underneath**. (certain)
- Kafka has **an individual [min,max] version per message type**, not for the whole protocol, and the client queries the support table right after connecting and calls only within that range → **adding a new message type does not affect anything else.** A direct refutation of our problem. (certain)
- MCP·LSP layer **version (format compatibility) and capability (whether a feature is possible) as separate layers**. (certain)
- Watchman's docs state explicitly that **comparing version numbers is not recommended** and name-based capability probes should be used. (certain)
- The criterion MCP states for bumping the version = **only when backward compatibility breaks**. (certain)
- Gradle·VS Code Remote let per-version instances coexist so the mismatch itself never arises (at the cost of accumulating idle resources). (certain)

## F3. "Waiting forever because it does not know the new message kind" is blocked in three layers
- (a) **self-describing frames** — JSON-RPC can open the envelope even without knowing the method name and return a "method not found" error with the same id. (certain)
- (b) **pre-gating** — Kafka·Docker first check the peer's supported range before sending and call only within it. (certain)
- (c) **the timeout is the final backstop** — gRPC's official docs warn that without a deadline the client may effectively wait forever, and recommend always setting an explicit deadline. (certain)
- **Implication:** the ground on which we judged "the version must be raised" (waiting forever) is **not a version problem but a problem of no timeout + no correlation id on the error.** The version was only a tool that masked that symptom.

## F4. For the notification surface, a two-way severity split is canonical
- **A modal only for "what you are trying to do now is blocked."** Apple HIG: use alerts sparingly, only for critical information needed immediately. Microsoft: a ContentDialog only when it is severe enough to block the whole app interaction. Win32: whether it is critical · whether it needs immediate action is the modal criterion. (certain)
- **A failure that occurred automatically and has nothing to do with a particular user action = a banner (InfoBar)** — the MS guidance branches on this explicitly, which is effectively identical to our scenario. (certain)
- **The time of notification is not the moment of detection but the time of use.** kubectl **deliberately designed** the client/server skew warning to appear only when you type `kubectl version`, not on every command (avoiding noise). The k8s API server's deprecation warning is also attached as a header on the actual request rather than as an advance notice. Win32 states explicitly that if it can resolve itself within a few seconds, the notification should be delayed. Tailscale shipped a real noise bug where a mismatch warning appeared even when only the build metadata differed. (certain)
- **Notification fatigue has a measured cost** — browser security warning click-through up to 70.2% (a field study with 25 million+ exposures), about 90% of SSL warnings ignored. Win32 states that the cost of over-notifying is blocking all notifications from that app. (certain)
- **A message is useful only if it offers an action** — title = what went wrong / body = how to fix it / button = a clear action. (certain)
- **Giving the same signal to humans and automation is standard practice** — k8s Conditions put a machine-readable Reason + a human-readable Message in one object. In LSP, severity is itself a protocol type. (certain)

## Derived options
- **㉠ Do not bump the version + capability negotiation + timeout + a banner at the time of use.** F2·F3·F4 all point this way.
- **㉡ Bump the version + guide with a modal + introduce a daemon replacement policy.** F1 is against it (we are on the "has something to lose" side), and F4 is against the modal.
- **㉢ (long term) Docker-style structural separation** — move session ownership out of the version-sensitive control path. The deepest solution but a large redesign.

## What could not be confirmed / abstentions
- **No mature precedent was found of a GUI app notifying a background daemon version mismatch with a modal or banner.** Most of the peers surveyed were CLI tools. That is, F4's notification guidance came from general UI guidelines, not from "a GUI precedent that solved the same problem."
- No primary counter-argument that "capability negotiation is excessive" was found (this means it was not found, not that none exists).
- No primary document was found that states an explicit ranking that "a timeout is a higher-level safeguard than version negotiation" (indirect evidence only).
- Unconfirmed how VS Code Remote handles the old server when the client is upgraded in the middle of an active session.

---

## Adversarial review result (cross-family · mandatory gate at the medium tier)

**Verdict: BLOCK — the recommendation above could not stand as it was.** Below is what the review cut down and the conclusion that changed as a result. **The body above is the pre-review synthesis, and the final conclusion is ADR-0158.**

- **「무한 대기는 버전 문제가 아니다」 was a logical leap.** A deadline only cuts the wait, and the correlation marker is attached only when the peer **could open** the envelope. A peer that cannot open the envelope at all cannot give an answer that can be paired. The accurate expression = **「버전 불일치가 원래 있던 결함 둘을 드러냈다」.**
- **「이미 설치된 옛 상대에게 능력을 어떻게 묻나」 was pointed to as the biggest weakness** — if you ask about something it does not know, it cannot understand even that. **In our structure, we confirmed by measurement that this is blocked**: `AgentEvent::Hello` first advertises the daemon's entire capabilities in the daemon→client direction, and there are zero declarations in this protocol crate that reject an unknown field. That is, there is no case where the new app sends something unknown to an old daemon.
- **「모달 vs 띠는 정본 2분법」 is an overstatement.** Vendor guidance has more surfaces, and it also says **to raise it to a modal if the state is severe enough to block the whole app interaction.** However, going the capability-check route means the 「못 붙는」 state itself never arises, so in our case the band is right.
- **「사용 시점 고지가 유일하게 지지된다」 is also an overstatement.** The criterion is the **scope of impact** — persistent, app-wide degradation gets a **band that notifies in advance**, an optional feature gets notification at the time of use. An old daemon is a persistent condition, so **notifying at startup is right** (consistent with the user's judgment).
- **GUI precedents do exist** (the synthesis's 「못 찾았다」 was a limit of the survey) — commercial management tools that use a **band + action button** for console·server version mismatch, a tool that uses a popup titled 「백그라운드 서비스 버전 불일치」, and a case that **removed the intrusive popup and kept only the band**.
- **A fourth path nobody had proposed** — **generation overlap + letting the old generation wither** (start the new daemon in a separate place and send only new connections to it, and leave the old daemon as it is until its own sessions end). A formal technique of the proxy-server family. **The only path that solves 「데몬을 죽이면 안 된다」 structurally** — recorded as a long-term direction.
- Several exaggerations in individual peer descriptions were also pointed out (one tool's lossless restart is opt-in and not the default, and major upgrades are outside its guarantee · one tool can split sockets so that servers coexist, so 「수동 우회뿐」 is too strong · the quoted warning-ignore rate figures are values from a specific browser and specific experimental conditions and are not generalized values). **Read the confidence labels in the body above as lowered by these findings.**

## Points of contention / limits

- No primary counter-argument that 「능력 협상이 과하다」 was **found** (this means it was not found, not that none exists).
- **No** primary document states an explicit ranking that 「마감 시한이 버전 협상보다 상위 안전장치」 (indirect evidence only).
- Most of the peers surveyed are CLI tools, so the GUI notification guidance came from general interface guidelines and not from 「같은 문제를 푼 GUI 선례」 (the review found the precedents themselves, but the final form of that UX is unverified).
