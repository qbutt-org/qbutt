# qbutt

qbutt is a public Windows-first native Qt fork of qBittorrent. Keep the upstream source layout: `src/base` owns application/session services, `src/gui` owns Qt presentation, and libtorrent owns peer I/O, piece selection and verification. Bun/TypeScript is preferred for new build and integration tooling. Read `CODING_GUIDELINES.md` before C++ changes.

qbutt is architecturally independent from the private Svoiseti service. Its network settings accept an ordinary Mihomo subscription and expose basic node selection and enable/disable controls. Do not require service accounts, private APIs, fixed servers, provisioning or service-specific formats. Existing private projects are architectural references only.

## Sources of truth

- `docs/qbutt-architecture.md` defines the target architecture; `docs/implementation.md` defines the staged acceptance criteria. Planned APIs and capabilities in these documents are not evidence that they exist.
- `upstream-lock.json` records exact source and dependency revisions once verified. Never substitute a mutable branch or `latest` in release builds.
- `CMakeLists.txt`, `cmake/`, and `.github/workflows/` define the actual build. Document only commands that were run, or explicitly label them unverified.
- Before deciding, inspect the relevant implementation, data flow, runtime behavior and project scripts. Reuse established components and workflows; keep one owner for each contract and state. If a contract remains ambiguous after investigation, ask instead of adding speculative fallbacks, compatibility branches, wrappers or runtime checks.

## Architecture and safety

- One libtorrent session, its standard picker per torrent, infohash-scoped peers, and one writer per data set. Network adapters do not know pieces, game paths or deletion policy.
- Separate Edge, Transport, and Path with generation. An extra protocol on the same server is not an independent edge. Unknown capabilities remain unknown until probed.
- qbutt-net is a separate process with private bounded control IPC and authenticated loopback payload listeners. It does not install a system TUN. Do not import global routing/DNS/TUN policy from a subscription.
- Do not claim Tunnels only for a TCP prototype. It requires evidence for all peer, tracker, DNS, DHT, webseed and policy-transition traffic. A SOCKS relay is not proof of public inbound.
- Read-only repair analysis must not write. Managed in-place repair requires explicit consent, correct mappings, exclusive ownership and drained I/O; reuse libtorrent truncate/recheck. Verify hashes and exact sizes, preserve unknown files, reject unsafe paths and hardlink aliases.
- Safe update uses independent staging and recoverable journal steps. Never use ordinary hardlinks for writable staging or silently fall back to in-place. Completion policies run only after data verification and commit; removing a torrent preserves its payload.
- qbutt has a separate profile. Do not edit live qBittorrent, Koala, router or production service settings, or share an active download directory with another writer.

## Product interface

- Design for people downloading files, not for agents or network operators. Keep primary actions obvious and remove redundant controls, implementation jargon and explanatory paragraphs that merely describe automatic behavior. Fix unclear composition, labels or behavior before adding help; retain help needed to understand a constraint, error, consequence or next action.
- Preserve the user's explicit taste, approved references and existing product behavior. Apply visual cleanup to the changed area without starting an unrelated redesign. Show only real data, states and working actions; do not present planned or unavailable capabilities as ready.
- Keep every settings category visible. Place secondary parameters lower in the same page under a visible `Advanced settings` heading, separated by spacing; do not hide categories or these sections behind disclosure buttons.
- Use one coherent native Qt visual language: restrained flat surfaces, readable typography, consistent action icons and the `#009df7` accent. Reuse existing widgets, assets and theme ownership rather than adding a second UI framework. Build hierarchy through content order, typography, alignment and spacing; fix layout from the parent container before adjusting children.
- Do not add EYEBROW labels, decorative numbered sections, nested framed groups or repeated status indicators. Containers and dividers need a semantic purpose; empty space alone does not justify decoration. A step indicator must represent necessary process progress.
- Offer built-in System, Light and Dark appearance, defaulting to System. Apply appearance changes live and follow OS changes in System mode. Qt rendering styles are implementation details, not a user choice. Preserve explicit user appearance choices.
- On Windows, minimize goes to the taskbar, close goes to the tray, and explicit Exit quits without confirmation. Startup at Windows logon is silent. Do not reintroduce configurable variants of these actions.
- Ordinary torrent addition owns checking and reusing existing destination files. Keep specialized repair actions only for genuinely different work, preserving the storage safety and consent rules above.

## Working and verification

- Optimize for clean, simple, strong final code across the codebase. Minimality means no unnecessary code, state, abstractions or misplaced ownership, not the fewest changed lines. Remove obsolete code rather than preserving it behind fallbacks.
- Write product code, scripts, configuration and agent tooling for the person who will read, understand and maintain them. An agent's ability to search dense code or a huge file does not make that representation convenient for a developer.
- Keep a consistent style and meaningful visual rhythm: order related code by reading and execution flow, separate independent logical blocks with blank lines, and expand complex expressions and control flow. Do not save lines at the expense of readability or combine multiple actions into long one-liners. Formatting is a baseline check, not a substitute for thoughtful structure.
- Split files by coherent responsibilities so people can navigate without constant search. Do not mix unrelated scenarios in one file or fragment cohesive logic into wrappers without benefit. Keep manually edited configuration indented and multiline.
- Preserve unrelated dirty work. Use separate worktrees for concurrent subsystem implementations. Keep temporary backups and generated research/build artifacts outside tracked source.
- Clean up obsolete builds, duplicated test runtimes and generated payloads as soon as they are no longer needed. Retain compact diagnostic evidence and dependencies/builds needed for unfinished work; verify cleanup paths and release owned processes and test volumes before deletion.
- Do not write unit tests. Use generated legal fixtures and integration, end-to-end and fault scenarios appropriate to the change. A timeout or empty output is not a successful check; inspect exit status and artifacts.
- Native regressions, incorrect payload, data loss and leaks override throughput claims. Report measured verified bytes separately from relay/wire traffic and identify untested real-network scenarios.
- The Windows build entrypoint is `./scripts/build-windows.ps1`; it produces a portable bundle and records the source revision/dirty state. Use the isolated fixture setup and `bun run smoke:native`, `smoke:repair`, `smoke:proxy`, and `smoke:network` commands in `tests/README.md`. Set `QBUTT_LAB_PATHS=1` to exercise the bundled transport child. Never use a live profile as a test fixture.
- Functional visual tests use an isolated Light profile with custom themes disabled; use `tests/appearance.ts` (default `QBUTT_LAB_APPEARANCE=functional`). Theme and release-default checks use `QBUTT_LAB_APPEARANCE=product` to exercise System, Light and Dark, including live switching. Assert control state through Qt properties as well as screenshots. Never ship test appearance overrides or overwrite a user's saved layout/theme.
- For GUI changes, inspect the running native interface and perform the affected actions with real pointer input. Check representative window sizes, the minimum size with scrollbars, long labels, growing lists, affected themes and relevant loading/empty/error states. Check changed motion in operation, including entry and exit. For significant UI changes, use a fresh read-only reviewer in a clean context when subagents are available: provide only the brief, applicable instructions, final sources/diff and verified renders; ask for provable requirement violations. Verify findings yourself and repeat affected checks after fixes; the reviewer cannot override the user's taste.

## Review and ablation

- After each meaningful edit, review the diff for unused methods, imports and files; unnecessary state, fallbacks and wrappers; incorrect layer ownership; one-line helpers without semantic value; abandoned hypotheses; and simplifications that preserve readability. If removing a temporary workaround requires an out-of-scope change, explain the concrete dependency and request that change.
- После получения рабочего результата проведи абляционный проход: поочерёдно убирай добавленные абстракции, состояния, fallback-ветки, обёртки и специальный код, затем повторяй применимые проверки. Всё, без чего требуемое поведение и долгосрочная поддерживаемость сохраняются, удали. Не упрощай ценой потери ясных архитектурных границ, диагностируемости или доказанной будущей расширяемости.
- After a working UI render, remove auxiliary text, decorative markers, containers, dividers, repeated states and effects one at a time. After each removal, inspect the render and perform the affected action at the relevant window sizes and themes. Keep it removed when understanding, action discoverability, feedback and confirmed product identity survive; otherwise restore the necessary minimum or fix composition. Remove handlers, styles and dependencies that lose their consumer.
- When editing instructions, remove duplicates, situational exceptions, obsolete examples and paraphrases one at a time. Give each rule one owner and check representative decisions against the remaining guidance. Do not move a duplicate into another file or weaken an explicit preference to shorten the text.

Keep ablation within the changed area. Repeat each pass until a full review finds nothing dispensable; after the final removal, repeat the main scenario and affected checks. Ablation requires actual removal and verification, not only a mental assessment. If you did not inspect the running interface yourself, report source review and leave visual ablation unverified.

## Git and public delivery

- `origin` is the standalone public `qbutt-org/qbutt` repository; `upstream` is `qbittorrent/qBittorrent`. Keep only `main` in the published repository. Do not mirror upstream branches or tags, or join GitHub's fork network. Preserve source history, licenses and notices.
- Fetch upstream into its own remote, review the selected release or commit against our changes, integrate it locally, and update `upstream-lock.json` after applicable checks. Publish the resulting qbutt state to `origin/main`; upstream engagement is a separate action. Apply the same maintenance model to component repositories.
- Never commit/push credentials, subscription URLs, private profiles, dumps or signing keys. Never print secrets in logs, diffs, answers or GitHub content. Audit selected changes from private dependencies before transferring them; never publish their history wholesale.
- Keep component repositories under `qbutt-org` with short `qbutt-*` names. Create a component repository only when it owns actual source changes.
- Verify published commit and repository visibility through GitHub, not the remote name. Review the complete outgoing diff and use explicit staging paths.
- Build and validate releases locally, then upload the finished artifacts. GitHub workflows may retain only `workflow_dispatch`; do not add automatic triggers or dispatch a run without an explicit user request. Preserve CI credits.

## Maintaining instructions

Обновляй проектные инструкции, когда меняется их контракт или пользователь явно просит сохранить универсальное долговременное правило. Обновляй существующее правило у его владельца и удаляй устаревшее в том же изменении. Ситуативные договорённости, временные приоритеты и очередность задач держи в плане реализации.
