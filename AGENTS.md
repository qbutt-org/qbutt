# qbutt

qbutt is a public Windows-first native Qt fork of qBittorrent. Keep the upstream source layout: `src/base` owns application/session services, `src/gui` owns Qt presentation, and libtorrent owns peer I/O, piece selection and verification. Bun/TypeScript is preferred for new build and integration tooling. Read `CODING_GUIDELINES.md` before C++ changes.

qbutt is architecturally independent from the private Svoiseti service. Its network settings accept an ordinary Mihomo subscription and expose basic node selection and enable/disable controls. Do not require service accounts, private APIs, fixed servers, provisioning or service-specific formats. Existing private projects are architectural references only.

## Sources of truth

- `docs/qbutt-architecture.md` defines the target architecture; `docs/implementation.md` defines the staged acceptance criteria. Planned APIs and capabilities in these documents are not evidence that they exist.
- `upstream-lock.json` records exact source and dependency revisions once verified. Never substitute a mutable branch or `latest` in release builds.
- `CMakeLists.txt`, `cmake/`, and `.github/workflows/` define the actual build. Document only commands that were run, or explicitly label them unverified.
- Study the relevant implementation before deciding. Keep one owner for every contract and avoid copying session state into another backend.

## Architecture and safety

- One libtorrent session, its standard picker per torrent, infohash-scoped peers, and one writer per data set. Network adapters do not know pieces, game paths or deletion policy.
- Separate Edge, Transport, and Path with generation. An extra protocol on the same server is not an independent edge. Unknown capabilities remain unknown until probed.
- qbutt-net is a separate process with private bounded control IPC and authenticated loopback payload listeners. It does not install a system TUN. Do not import global routing/DNS/TUN policy from a subscription.
- Do not claim Tunnels only for a TCP prototype. It requires evidence for all peer, tracker, DNS, DHT, webseed and policy-transition traffic. A SOCKS relay is not proof of public inbound.
- Read-only repair analysis must not write. Managed in-place repair requires explicit consent, correct mappings, exclusive ownership and drained I/O; reuse libtorrent truncate/recheck. Verify hashes and exact sizes, preserve unknown files, reject unsafe paths and hardlink aliases.
- Safe update uses independent staging and recoverable journal steps. Never use ordinary hardlinks for writable staging or silently fall back to in-place. Completion policies run only after data verification and commit; removing a torrent preserves its payload.
- qbutt has a separate profile. Do not edit live qBittorrent, Koala, router or production service settings, or share an active download directory with another writer.

## Product interface

- Design for people downloading files, not for agents or network operators. Keep primary actions obvious and remove redundant controls, implementation jargon and explanatory paragraphs that merely describe automatic behavior.
- Keep every settings category visible. Place secondary parameters lower in the same page under a visible `Advanced settings` heading, separated by spacing; do not hide categories or these sections behind disclosure buttons.
- Use one coherent native Qt visual language: restrained flat surfaces, readable typography, consistent action icons and the `#009df7` accent. Avoid nested framed groups and decorative containers. Reuse existing widgets and theme ownership rather than adding a second UI framework.
- Offer built-in System, Light and Dark appearance, defaulting to System. Apply appearance changes live and follow OS changes in System mode. Qt rendering styles are implementation details, not a user choice. Verify both schemes and preserve explicit user appearance choices.
- On Windows, minimize goes to the taskbar, close goes to the tray, and explicit Exit quits without confirmation. Startup at Windows logon is silent. Do not reintroduce configurable variants of these actions.
- Ordinary torrent addition owns checking and reusing existing destination files. Keep specialized repair actions only for genuinely different work, preserving the storage safety and consent rules above.

## Working and verification

- Optimize for simple, strong final code across the codebase, not the smallest patch. Remove obsolete state, imports, wrappers and mistaken ownership instead of preserving them behind fallbacks.
- Preserve unrelated dirty work. Use separate worktrees for concurrent subsystem implementations. Keep temporary backups and generated research/build artifacts outside tracked source.
- Clean up obsolete builds, duplicated test runtimes and generated payloads as soon as they are no longer needed. Retain compact diagnostic evidence and dependencies/builds needed for unfinished work; verify cleanup paths and release owned processes and test volumes before deletion.
- Do not write unit tests. Use generated legal fixtures and integration, end-to-end and fault scenarios appropriate to the change. A timeout or empty output is not a successful check; inspect exit status and artifacts.
- Review the diff after each change. After a working result, perform an ablation pass: remove each unnecessary abstraction, state, wrapper, fallback and special case, then repeat applicable checks. Preserve clear architectural boundaries and useful diagnostics.
- "Работает" недостаточно. После того как довел до рабочего состояния, убедись, что решение встроено в код красиво и без временных подпорок. Если по пути пришлось оставить костыль или фоллбэк, потом обязательно добейся его удаления, даже если для этого надо явно попросить пользователя сделать связанное изменение.
- Native regressions, incorrect payload, data loss and leaks override throughput claims. Report measured verified bytes separately from relay/wire traffic and identify untested real-network scenarios.
- The Windows build entrypoint is `./scripts/build-windows.ps1`; it produces a portable bundle and records the source revision/dirty state. Use the isolated fixture setup and `bun run smoke:native`, `smoke:repair`, `smoke:proxy`, and `smoke:network` commands in `tests/README.md`. Set `QBUTT_LAB_PATHS=1` to exercise the bundled transport child. Never use a live profile as a test fixture.
- Functional visual tests use an isolated Light profile with custom themes disabled; use `tests/appearance.ts` (default `QBUTT_LAB_APPEARANCE=functional`). Theme and release-default checks use `QBUTT_LAB_APPEARANCE=product` to exercise System, Light and Dark, including live switching. System must follow the host appearance. Assert control state through Qt properties as well as screenshots. Never ship test appearance overrides or overwrite a user's saved layout/theme.

## Git and public delivery

- `origin` is the standalone public `qbutt-org/qbutt` repository; `upstream` is `qbittorrent/qBittorrent`. Keep only `main` in the published repository. Do not mirror upstream branches or tags, or join GitHub's fork network. Preserve source history, licenses and notices.
- Fetch upstream into its own remote, review the selected release or commit against our changes, integrate it locally, and update `upstream-lock.json` after applicable checks. Publish the resulting qbutt state to `origin/main`; upstream engagement is a separate action. Apply the same maintenance model to component repositories.
- Never commit/push credentials, subscription URLs, private profiles, dumps or signing keys. Never print secrets in logs, diffs, answers or GitHub content. Audit selected changes from private dependencies before transferring them; never publish their history wholesale.
- Keep component repositories under `qbutt-org` with short `qbutt-*` names. Create a component repository only when it owns actual source changes.
- Verify published commit and repository visibility through GitHub, not the remote name. Review the complete outgoing diff and use explicit staging paths.
- Build and validate releases locally, then upload the finished artifacts. GitHub workflows may retain only `workflow_dispatch`; do not add automatic triggers or dispatch a run without an explicit user request. Preserve CI credits.

## Maintaining instructions

Если пользователь в ходе работы даёт новые устойчивые правила по стилю кода, структуре или процессу, то их надо кратко и по делу сразу добавлять в этот файл `AGENTS.md`, если это реально полезно будущим агентам.

Правила в `AGENTS.md` добавлять только если пользователь явно просит сохранить что-то универсальное и долговременное. Ситуативные договорённости, временные приоритеты и очередность задач держать в плане реализации.
