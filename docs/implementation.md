# qbutt: задание для реализации агентами

Читать вместе с `qbutt-architecture.md`. Этот файл является рабочим backlog и контрактом приёмки, а не утверждением, что соответствующие API уже существуют.

Уточнение требований: qbutt архитектурно независим от частного сервиса. Настройки принимают обычную Mihomo-подписку и позволяют выбрать узел; аккаунты, API сервиса и заранее заданные серверы не требуются. Обозначения S1/S2/S3 ниже — примеры независимых выходов для лаборатории. Публичный qbutt-net начинается с закреплённого публичного upstream без истории частных форков; обоснование приведено в `adr/0001-first-slice.md`.

## 1. Цель

Приоритеты уточнены пользователем: сетевая часть и repair существующих данных имеют высокий приоритет. Repair здесь означает проверку и докачку файлов раздачи, а не восстановление установки qbutt. Автообновление — в самом конце, простое скачивание с GitHub Releases по примеру `element-max`.

Текущая цель — завершить весь обязательный контракт этого плана, а не остановиться после первого рабочего этапа или публикации alpha. Промежуточный релиз подтверждает только перечисленные в его evidence сценарии. Единственный реестр состояния приёмки находится в разделе 9; незакрытые локальные работы продолжаются независимо от внешних условий отдельных проверок.

Сделать нативный qBittorrent fork с двумя реальными преимуществами: безопасным использованием существующих данных для repair/update и одновременными управляемыми сетевыми выходами через Native и выбранные пользователем узлы.

Разрешено менять qBittorrent, libtorrent, пользовательские Mihomo/Xray forks и серверный gateway. Не защищать upstream API ценой плохой архитектуры. Не переписывать работающие механизмы без необходимости. Успех измеряется корректным скачанным файлом, сохранностью данных и результатами controlled benchmark.

## 2. Общие правила работы

- Не использовать реальные коммерческие game data как обязательные fixtures. Генерировать маленькие воспроизводимые наборы.
- Не запускать код из случайной подписки/профиля и не выводить credentials в лог.
- Не менять настройки Koala, роутера и работающих production VPS без отдельной задачи и согласованного плана отката.
- Не импортировать активные данные из другого клиента с одновременной записью двумя процессами.
- Все зависимости закреплять revision. Не скачивать `latest` в release build.
- Каждый модуль имеет feature flag для экспериментов, тесты, метрики и описанный fallback.
- Исследовательский результат «не ускоряет этот сценарий» допустим. Подгонка метрик, выбор одного удачного пика и скрытое снижение безопасности недопустимы.
- Таймаут инструмента/пустой stdout не доказывает успешность операции. Получать итоговый статус процесса, артефакт и результаты тестов.

## 3. Первый рабочий этап

### T00. Инвентаризация и baseline

**Сделать:** определить доступные локальные repositories и пользовательские forks; собрать неизменённый контрольный qBittorrent; зафиксировать dependency lock; сформировать capability report реального qbutt-net кандидата.

**Не придумывать:** commit SHA, поддержку remote listen, метрики потерь, устойчивый UDP mapping, build commands пользовательских форков.

**Артефакты:** `upstream-lock.json`, `docs/baseline.md`, `docs/capabilities.md`, native Windows build, успешные smoke tests.

**Приёмка:** add torrent, selective download, hash validation, stop/resume, clean shutdown/restart. Отдельный профиль, исходные пользовательские настройки не меняются.

### T01. Fixtures и negative tests

Сгенерировать v1/v2/hybrid раздачи с известными payload hashes. Ввести изменённые bytes, длинный хвост, укороченный файл, перенос пути, boundary pieces между файлами и изменённый источник после индексации.

Подготовить два controlled peers, к которым клиент сможет добраться только через разные сетевые выходы. Зафиксировать ожидаемые topology и byte counters.

**Приёмка:** проверки выявляют отсутствие smart repair и невозможность route-aware mixed path в исходной сборке. Тесты не зависят от живого публичного роя.

### T02. Контракты

Зафиксировать `Edge`, `Transport`, `PathContext`, `generation`, `NetworkPolicy`, `RepairPlan`, `RepairState`, `CompletionEvent`.

Разделить control IPC и payload. Определить точные единицы счётчиков: bytes, MiB/s, verified goodput, wire traffic. Определить поведение при неизвестной capability и несовместимой версии child process.

**Приёмка:** fake qbutt-net и fake storage позволяют проверить contracts без реального VPN/дисковой загрузки. Не создавать общий RPC-фреймворк на десятки методов заранее.

## 4. Параллельные направления

### Поток A: Smart Repair

**A1. Read-only analysis**

Целевая `.torrent` + выбранный каталог. Получить подтверждённые valid pieces и список проблемных файлов. Ни truncate, ни rename до подтверждения. Отличать кандидаты по имени/размеру от проверенных данных.

**A2. Managed in-place repair**

Quiesce I/O, ownership, явное разрешение записи, применение существующего truncate к правильным mappings, запуск стандартного recheck и докачки. Unknown files не удаляются.

**A3. Mapping и индекс**

Поиск в выбранных корнях, metadata-only индекс, поддержка переименований. v1 хэшировать в целевом логическом layout, v2 проверять file roots. Не обещать universal delta после вставки bytes.

**A4. Safe update**

Сначала полный staging target для простоты. Затем только при необходимости оптимизация write-affected files с замыканием по v1-boundary pieces. Journal, корректная оценка места, backups и идемпотентный recovery. Обычные hardlinks не используются для изменяемого staging.

**A5. Приёмка**

Убить процесс на каждом шаге commit/rollback. Проверить исходные hashes, целевые hashes, exact file sizes, отсутствие повреждения других hardlink aliases и unknown files. In-place сообщает отсутствие полного rollback.

### Поток B: qbutt-net и transport adapters

**B1. Отдельный процесс**

Собрать qbutt-net на базе проверенной публичной ревизии Mihomo. Приватный control channel, version handshake, bounded logs, безопасная остановка.

**B2. Закреплённый local proxy**

Loopback SOCKS listeners для S1/S2/S3 с конкретным adapter. Поддерживаемые TCP и UDP capabilities проверить реальными probes. Сначала без remote listener.

**B3. Кооперация с Koala**

Проверить физический bind, DNS/bootstrap, отсутствие double tunnel и traffic recursion. Koala не редактировать автоматически. Если нужен explicit exclusion, оформить его отдельной диагностируемой настройкой.

**B4. Transport lifecycle**

Pinned transport, health/failure events, generation, переключение резервного transport без неожиданных edge changes. Никакой группы «все протоколы всех VPS» как скрытого единственного маршрутизатора.

**B5. Приёмка**

Подтверждены egress и нужная address family; падение child не ломает storage; нет неограниченных буферов; секреты не попадают в export; повторный запуск не оставляет бесконтрольные старые listeners.

### Поток C: libtorrent paths

**C1. Один proxy path**

Разобрать реальные call sites `instantiate_connection`, session proxy settings, listen/UDP contexts, tracker managers. Сохранить Native baseline без поведенческих изменений.

**C2. Mixed TCP**

Передавать выбранные per-connection proxy settings и PathContext. Два controlled peers через разные egress скачивают один torrent с одним picker. Не запускать отдельную session на путь.

**C3. Полный контур**

UDP association/uTP, DHT identity, HTTP/UDP trackers, DNS, webseeds. Peer pool scoped infohash, оригинальные remote endpoints, self-connection detection. Согласованные tracker identity/key и source IP.

**C4. Политики**

Direct, Pinned, Mixed, Tunnels only. Fail closed включает все torrent/discovery виды трафика и закрытие старых соединений при переключении. Private torrents ограничиваются pinned path и допустимыми источниками.

**C5. RouteSelector**

Начать с availability/failure fallback и ограниченного exploration. Затем добавить измерение полезной передачи и стабильности. Не заменять piece picker и не считать малый ping доказательством высокой скорости.

**C6. Приёмка**

Тесты: same last-mile cap; дополнительная достижимость; bad path; changing IP; failed UDP; choked peer; no demand; no duplicate slots; qbutt-net restart. Packet capture подтверждает политику, UI text её не заменяет.

### Поток D: UI, lifecycle, release

**D1. UI**

Repair preview, Paths, Diagnostics и Policies в существующем Qt. Тяжёлый I/O не в UI thread. Показывать «неизвестно» вместо выдуманных loss/RTT/public IP.

**D2. Completion policies**

Событие после готовности выбранных файлов и commit. `remove_torrent` сохраняет payload; `delete_data` отдельное явное действие. Preview на импорте и журнал причины.

**D3. Import/recovery**

Отдельный профиль. Backup и проверка конфликтующих активных torrent jobs. Сетевой импорт выбирает nodes/providers, не переносит весь Koala config.

**D4. Updater**

Выполнить последним: простой поток скачивания обновления из GitHub Releases по примеру `element-max`. Приложение и qbutt-net обновляются одним комплектом; ошибка проверки отличается от отсутствия новой версии. Не вводить отдельный сложный updater-фреймворк.

**D5. Приёмка**

Нельзя получить массовое удаление при первом импорте, premature completion во время repair или потерю настроек при неудачной установке.

## 5. Inbound gateway после устойчивого path context

### E1. Диагностический прототип

Проверить существующий FRP или минимальный trusted gateway: правильный публичный port, реальный source address, проверка с независимого узла. Не выдавать один открытый TCP socket за готовый UDP/uTP/DHT inbound.

### E2. qbutt-edge

Реализовать authenticated listener leases. qbutt-net инициирует control connection через существующий tunnel. Каждый входящий peer TCP получает отдельный work connection через adapter, а не общую очередь всех payload в control stream. UDP envelope сохраняет source endpoint внутри существующего datagram transport.

Публичные listeners ограничены политикой сервера; administrative endpoint не выставляется без защиты. Отдельные qbutt credentials и квоты не затрагивают остальных клиентов сервиса.

### E3. Приёмка

Сторонний controlled peer первым подключается к опубликованному VPS endpoint через домашний CGNAT. qbutt видит реальный peer IP/port, корректный path и нормальные pieces. Announce сообщает правильный endpoint. Lease expiry/reconnect не оставляет ложной достижимости.

## 6. Интеграция и доказательство пользы

Интегратор запускает одинаковый corpus на Native baseline, лучшем отдельном VPN, простом load balancing и RouteSelector. Фиксирует verified bytes/sec, completion time, reconnects, redundant bytes, CPU/RAM/I/O, UI stalls.

Лаборатория обязана содержать как сценарий выигрыша multipath, так и сценарий, где дополнительный VPN бесполезен. Система не считается адаптивной, если всегда включает все выходы и показывает больше соединений.

Публичные тесты выполняются повторными длительными окнами с описанием условий. Разница между удачным пиком и устойчивым выигрышем должна быть видна в отчёте.

## 7. Финальный формат отчёта каждого PR

```text
Что изменено:
Какие существующие механизмы переиспользованы:
Затронутые contracts:
Как собирать:
Какие тесты выполнены и где их результат:
Negative/fault scenarios:
Измерения до/после:
Что не проверено на реальном пользовательском окружении:
Способ отключить изменение / откатить миграцию:
Следующий конкретный технический шаг:
```

## 8. Очерёдность первого запуска

Сначала T00 → T01 → T02, затем независимо A1/A2, B1/B2 и D1. C2 начинается, когда B2 уже доказывает один корректный path; после него C3/C4, A4 и E1. Автоматическая адаптация C5 появляется после измеримого и корректного ручного Mixed режима. D4 выполняется последним.

Не откладывать работающий qbutt до завершения всей дорожной карты: первый самостоятельный артефакт должен доказывать repair с exact sizes и один явно выбранный сетевой путь. Каждый следующий артефакт расширяет проверенный сквозной сценарий.

## 9. Реестр полной приёмки

### 9.1. Объём и подтверждённая база

Обязательный объём: T00–T02, A–D, результат inbound E и интеграционные проверки раздела 6. Отдельный новый сервер `qbutt-edge` — вариант реализации E: существующий gateway допустим, если сохраняет все endpoint, authentication, UDP и lifecycle требования. Отсутствие собственного gateway не закрывает и не отменяет проверку inbound.

Streaming, live same-file multi-swarm, удалённые BitTorrent workers, publisher-assisted delta, глобальный piece cache/CAS и одновременные transport variants одного edge остаются явно отложенными расширениями или гипотезами архитектуры. Предложения из исследовательского диалога сами по себе не превращают их в обязательные функции. Обязательный A3 включает локальный поиск и сопоставление файлов; его нельзя отложить вместе с live multi-swarm. Конкретные названия классов, `qbutt.db`, named pipes и WinSparkle не являются самостоятельными результатами приёмки: сохраняются владельцы контрактов и требуемое поведение. Решение о приватных inherited pipes первого child зафиксировано в [ADR](adr/0001-first-slice.md).

Аудит ниже относится к исходникам `6e96bcce504772d89cfde038e9d18479b2543f47` от 12 сентября 2026 года. [Verification alpha.1](https://github.com/qbutt-org/qbutt/releases/download/v0.1.0-alpha.1/verification.json) связывает их с чистой сборкой, SHA-256 приложения `be45670a50a1667aaa1d079406618b6265447b18877e6e6abfbc1be2724fe67f` и qbutt-net `599a78654d71e245cb226dbe5f6080af863c4943` / `2453058bf906cace94a50fce57434857e4bf57db76a504ba4d0c1f6223eb4b0e`. Это исходная точка, а не утверждение о проверке будущих изменений.

| Evidence | Подтверждённый результат и граница |
| --- | --- |
| `native`, 8 checks | v1/v2/hybrid: selective download 54 149 bytes; полный payload 2 151 362 bytes с exact sizes; stop/restart/resume/recheck и сохранение payload после удаления задачи. Обычный recheck сохраняет лишний хвост 8 193 bytes. |
| `repair`, 23 checks | 21 вариант v1/v2/hybrid плюс проверки аутентификации и сохранности после shutdown. Corrupt/grow/shrink/missing-nonzero исправлены; v1 также проверяет уже заданный rename mapping, inserted bytes, source mutation и unknown files. Hardlink/reparse отвергаются. Missing-empty отвергает apply без создания файла; это проверенный отказ, а не поддержка такого repair. |
| `paths`, 8 checks | Реальный bundled child, выбранный loopback adapter, TCP peer и HTTP tracker, смерть child с остановкой передачи, retry с новым generation, stop/Native и охрана transitions. Verified payload 2 151 362 bytes. Relay stream bytes содержат протокольные данные и не являются wire traffic или measured goodput. |
| `completion-control` и `completion-repair`, по 1 check | Обычный auto-exit срабатывает; активный repair preview удерживает приложение после завершения другой раздачи. Полный rule engine и queued/nested-dialog race injection этим не проверены. |
| `final-ui`, 5 checks | Реальный offscreen Qt: контекстное меню repair, read-only таблица с относительными именами, согласие, apply/recheck, сохранность unknown file, Qt file dialog и выбор узла. Это не physical desktop interaction, не Windows native file dialog и не проверка длительных UI stalls. |

В [baseline](baseline.md) отдельно зафиксирован неизменённый upstream control. В [capabilities](capabilities.md) HTTPS-пробы ShadowQUIC/VLESS и таймаут Hysteria2 относятся к старому qbutt-net `a268eef...`; они не доказывают поведение bundled `599a786...` или всего торрент-трафика. Наличие тестового кода без successful evidence не закрывает проверку.

### 9.2. Обязательные условия по направлениям

Статусы: **закрыто** — выполнен весь указанный результат; **частично** — есть конкретная реализация/проверка, но условие справа остаётся; **открыто** — сквозной результат не реализован или не подтверждён. Статус меняется только вместе с проверенными revision, результатом процесса и evidence. Будущие API в этой таблице не объявляются существующими.

| Условие / статус | Реализация и имеющиеся доказательства | Что необходимо для закрытия |
| --- | --- | --- |
| **T00 — закрыто для функционального baseline** | [Lock](../upstream-lock.json), [build entrypoint](../scripts/build-windows.ps1), [baseline](baseline.md), alpha.1 native evidence; отдельный профиль в [profile_p.cpp](../src/base/profile_p.cpp). | Сохранять воспроизводимую сборку и native regression при обновлении компонентов. Сравнение производительности закрывается отдельно в разделе 9.3. |
| **T01 — частично** | [Генератор](../tests/fixtures/generate.ts), [torrent metadata](../tests/fixtures/torrents.py), [native](../tests/native-smoke.ts), [repair](../tests/repair/smoke.ts), [TCP lab](../tests/network-lab/smoke.ts). Есть несколько piece sizes, v1 boundary, Unicode, пустой файл и filesystem negatives. | Добавить два дополняющих peer за разными выходами и отрицательный контроль Mixed на baseline; выполнить полный storage/network fault corpus из архитектуры §9, включая long/case/traversal layouts, смену тома, нехватку места и commit/rollback crashes. |
| **T02 — частично** | [PathManager](../src/base/net/pathmanager.cpp) владеет bounded versioned control и одним path/generation; [RepairService](../src/base/bittorrent/repairservice.h) — состояниями in-place. [API repair](../src/webui/api/repaircontroller.cpp) проверяет operation id и consent. | Согласовать реальные Edge/Transport/PathContext/NetworkPolicy, RepairPlan/journal и CompletionEvent между владельцами. Интеграционно проверить malformed/oversized/incompatible child, stale generation, storage failures и точные единицы счётчиков; не вводить отдельный RPC-фреймворк или копию session state. |
| **A1 — частично** | [repairanalysis.cpp](../src/base/bittorrent/repairanalysis.cpp) читает целевые v1 pieces и v2 roots/piece layers под [guard](../src/base/bittorrent/repairfileguard.cpp); worker вынесен из UI. Alpha подтверждает read-only и verified bytes для управляемой остановленной раздачи. | Дать анализ целевой `.torrent` и выбранных каталогов до запуска записи, включая выбранные файлы и найденные mappings; честно разделить candidate/verified/download/temp bytes. Проверить отмену анализа и отсутствие любых изменений, включая создание отсутствующих файлов, на всех поддержанных layouts. |
| **A2 — частично** | [RepairService](../src/base/bittorrent/repairservice.cpp), [TorrentImpl](../src/base/bittorrent/torrentimpl.cpp), [SessionImpl](../src/base/bittorrent/sessionimpl.cpp): admission, drain, исключительное изменение размера через Windows handle, стандартный recheck. Исключение из `truncate_files` обосновано в ADR. | Поддержать отсутствующие zero-length targets и штатные selected/incomplete/download-path mappings без текущих ограничений первого этапа; проверить передачу ownership при ошибке/отмене/перезапуске и final hashes + exact sizes после докачки. Не обещать rollback для in-place. |
| **A3 — частично** | Текущие torrent rename mappings передаются в анализ и проходят v1 fixture; source mutation вызывает новый анализ. Metadata-only поиск по выбранным корням и между раздачами отсутствует. | Реализовать поиск кандидатов, явное сопоставление переименованных/перемещённых файлов, invalidation по identity/size/mtime и reuse в целевом v1 layout/v2 roots. Проверить перестановку файлов, другой piece size и отказ от ложного reuse после вставки bytes. |
| **A4/A5 — открыто** | [RepairService](../src/base/bittorrent/repairservice.cpp) имеет только in-place states; staging, journal, commit и rollback отсутствуют в проверенной базе. | Полный независимый staging target, точная оценка места, source read-only, один writer, проверка hashes/lengths перед commit, сохранение unknown files и журналируемый recoverable swap/replace. Убить приложение на каждом шаге commit/rollback; recovery идемпотентен, исходные данные и hardlink aliases сохраняются. Disk-full не переключает режим в in-place. |
| **B1 — частично** | Bundled qbutt-net, private inherited stdio, hello v1, frame 64 KiB, loopback credentials и ограниченный ответ child в [PathManager](../src/base/net/pathmanager.cpp). Alpha paths доказывает реальный child death/retry/shutdown. | Для итогового multipath child проверить protocol/version rejection, backpressure и суммарные лимиты, parent EOF/crash, освобождение всех TCP/UDP listeners и отсутствие неограниченных diagnostics. Старые отдельные child-пробы повторить на итоговом bundle. |
| **B2/B4 — частично** | [PathManager](../src/base/net/pathmanager.cpp) импортирует подписку/локальный YAML и явно выбирает один adapter; сохраняет generation, блокирует путь при отказе. [Proxy manager](../src/base/net/proxyconfigurationmanager.cpp) применяет один runtime proxy. | Несколько независимых edges одновременно, transport fallback только внутри выбранного edge, generation lifecycle для всех контекстов; реальные TCP/UDP/IPv4/IPv6 probes итоговых Hysteria2, ShadowQUIC, VLESS и остальных предлагаемых adapters. Не считать aliases одного сервера разными egress; неизвестные capabilities сохранять неизвестными. |
| **B3/B5 — частично** | Физический interface передаётся child; приложение не устанавливает TUN и не импортирует глобальные routing/DNS правила. Старые HTTPS probes при Koala on описаны отдельно. | Проверить реальный Native/remote egress, bootstrap/DNS, отсутствие recursion/double tunnel при Koala off/on/restart, смене default route, sleep/resume и отказе интерфейса. Итоговый diagnostics export не содержит secrets/личные адреса без выбора пользователя. Изменение Koala требует отдельной авторизации; локальные проверки не ждут её. |
| **C1 — частично** | [SessionImpl](../src/base/bittorrent/sessionimpl.cpp) сохраняет одну libtorrent session и включает session-wide SOCKS для первого TCP path. Alpha paths проверяет TCP peer и HTTP tracker через недоступные напрямую synthetic endpoints. | Сохранить Native baseline после engine fork; провести выбранный PathContext через все реальные сетевые call sites и снять session-global предположение в C2/C3. |
| **C2 — открыто** | В [lock](../upstream-lock.json) stock libtorrent 2.0.11; текущий runtime proxy один. | Одна session и один picker скачивают один проверенный torrent одновременно от двух controlled peers, каждый достижим только через свой edge; затем Native плюс несколько remote edges. Оригинальные peer endpoints, общий connection budget и обычный dedup сохраняются. |
| **C3 — открыто** | В pinned slice [SessionImpl](../src/base/bittorrent/sessionimpl.cpp) отключает DHT/LSD/uTP/inbound/port mapping. TCP lab не проверяет DNS, UDP tracker или webseed. | Route-aware TCP/UDP/uTP, HTTP(S)/UDP trackers, DHT, DNS, webseeds и metadata discovery; generation инвалидирует UDP associations и старые объявления. Infohash-scoped peers, self-connection detection, BEP42 identity и согласованные tracker peer id/key/source/port проверяются реальными запросами и packets. |
| **C4 — частично** | Смерть pinned child удерживает blocked mode; переключение Native/Pinned разрешено только при пустой session, включая скрытые metadata jobs. | Полные Direct/Pinned/Mixed/Tunnels only с активными jobs; запрещённые старые TCP/UDP соединения закрываются при переходе. Capture доказывает отсутствие Native torrent/discovery при любом отказе Tunnels only. Отдельный bootstrap/control policy; private torrent Pinned, без DHT/PEX и вне разрешённого source scope. |
| **C5/C6 — открыто** | Адаптивного RouteSelector и controlled сравнений в проверенной базе нет. | Availability/failure fallback, ограниченный exploration, история полезной передачи и устойчивости без замены picker. Проверить bad path, changing IP, failed UDP, choke/no-demand, один peer за aliases, no duplicate slots и restart. Здоровое соединение не мигрирует ради ping; ограничения суммарные, reward не считает relay bytes. |
| **D1 — частично** | [RepairDialog](../src/gui/repairdialog.cpp), [PathsWidget](../src/gui/pathswidget.cpp), alpha final-ui. Существующие Qt модели и API сохранены. | Qt-сценарии для полного repair/search/staging, нескольких paths, policies и diagnostics; быстрые фильтры по путям/источникам вместе с existing category/state/tracker. Проверить долгий I/O/probe/cancel, большой список и UI response. Новые API versioned и доступны только через разрешённый authenticated API. |
| **D1 diagnostics — частично** | Path status честно пишет unknown; child strings отфильтрованы, произвольный stderr не публикуется. Native peers/statistics остаются upstream. Полной панели причин и export нет. | Связать реальные discovery/connect/choke/demand/disk/hash/rate counters и причины выбора/запрета маршрута. Разделить peer payload, unique verified bytes, relay и wire traffic. Ограничить retention и проверить export на credentials, passkeys, subscriptions, private paths/IP, включая malicious names/errors. |
| **D2/D5 — частично** | [SessionImpl](../src/base/bittorrent/sessionimpl.cpp) и [Application](../src/app/application.cpp) подавляют старые completion actions во время repair; alpha completion и native removal подтверждены. | Детерминированные правила на wanted-files-verified-and-committed, existing category/tag/ratio/time, terminal order, preview, reason journal и idempotency. `remove_torrent` сохраняет payload; `delete_data` отдельно разрешён. Проверить completion во время moves/repair/commit, queued/nested-dialog races, restart/replay и первый импорт без массового удаления. |
| **D3 — частично** | [Profile](../src/base/profile_p.cpp), [application startup](../src/app/application.cpp) и portable build отделяют профиль/каталог загрузок. Сетевой импорт выбирает node definitions. Мастера безопасного переноса профиля/metadata нет. | Импорт копирует настройки/metadata с backup перед миграцией, preview политик и проверкой активных/пересекающихся jobs; исходный профиль не меняется. До записи в imported payload подтверждён один writer. Неудачная миграция сохраняет настройки и восстанавливается; provider import не переносит глобальную политику. |
| **D4 — открыто, выполнять последним** | [MainWindow](../src/gui/mainwindow.cpp) показывает ссылку на GitHub Releases и отсутствие updater; alpha bundle unsigned. | Простой check/download готового согласованного bundle из GitHub Releases, сравнение версий и явное отличие network/check failure от no update. Проверить целостность/подпись, mismatch, interrupted/corrupt download, portable profile preservation и failed migration recovery. Без отдельного backend и сложного updater framework; выпуск подписи требует доверенного ключа. |
| **E1–E3 — открыто** | Checked slice outgoing-only; SOCKS relay не является public inbound. Trusted accepted-connection adapter, leases и внешний capture отсутствуют. | После устойчивого PathContext реализовать/подключить gateway с authentication, quotas, lease expiry/reconnect, отдельными TCP work streams и UDP metadata. Независимый peer первым подключается через публичный endpoint к домашнему клиенту за CGNAT; app видит оригинальные IP/port/path, получает verified pieces и правильно announces TCP/UDP endpoint. Поддельные metadata отвергаются; истёкший lease не показывается достижимым. |

### 9.3. Сквозные release gates

Эти условия обязательны сверх отдельных успешных функций. Alpha.1 их полностью не закрывает.

| Gate / статус | Что требуется подтвердить |
| --- | --- |
| **Корректность и восстановление — частично** | На итоговой сборке весь corpus без необъяснённых hash/payload mismatches, exact selected-file lengths, один writer, unknown-file preservation; unsafe mappings/hardlinks не вредят aliases. Native/resume, in-place restart, staged crash recovery и policy/delete проходят вместе. |
| **Сеть и полезность — открыто** | Одинаковый corpus для unchanged upstream, qbutt Native, лучшего одного edge, простого распределения и RouteSelector. Есть выигрыш от дополнительной достижимости, отсутствие выигрыша remote, общий last-mile cap, плохой path и choked/no-demand peer. Повторяемые окна и topology; измеряются unique verified bytes/sec, completion time, overhead, churn, CPU/RAM/I/O и UI stalls. Проектный native median gate — не более 5% регрессии в стабильном стенде; при шуме расширяется выборка. Публичные длительные окна отдельно показывают внешнюю применимость. |
| **Политика и exposure — открыто** | Packet evidence для всех torrent/discovery классов, всех policy transitions и отказов; private scope и identities корректны. TCP/UDP inbound и Koala coexistence подтверждаются отдельно. Нет Tunnels-only заявления на основании TCP smoke. |
| **Поставка — частично** | [Build](../scripts/build-windows.ps1) и alpha подтверждают pinned bundle, source/dirty manifest, лицензии и локальную верификацию. Для финальной сборки повторить полную приёмку, совместимость app/net и подпись/обновление; проверить опубликованный commit, visibility, assets и только `main` через GitHub. Workflows остаются только `workflow_dispatch`, автоматические или незапрошенные запуски не нужны. Публичные материалы содержат измерения и ограничения, без секретов и недоказанных обещаний. |
| **Итоговый код — продолжается с каждым изменением** | Просмотр полного diff после изменения, абляционный проход и применимые build/integration/fault checks. Удалены ненужные состояния, wrappers, fallback и следы неверных гипотез; границы Qt/application/libtorrent/transport сохранены. Unit-тесты и отдельные markdown-отчёты не добавляются. |

### 9.4. Локальная работа и внешние условия

Локально можно параллельно завершать A3/A4/A5, B lifecycle, C2/C3/C4, D2/D3 и fixtures/Qt integration. C5 следует за корректным ручным Mixed и измерениями; E — за устойчивым PathContext; D4 интегрируется последним. Наличие внешнего условия одной проверки не является причиной останавливать остальные направления или объявлять всю цель выполненной.

| Внешнее условие | Зависимая проверка | Что делается до его появления |
| --- | --- | --- |
| Разрешённый публичный endpoint и независимый controlled peer | Реальный E3 через CGNAT, announce и TCP/UDP inbound; повторяемые real-network окна. Изменения production VPS не предполагаются разрешёнными. | Локальный gateway/adapter, auth/quotas/leases, поддельные metadata, controlled TCP/UDP flows и fault recovery. |
| Явное разрешение менять режимы/перезапускать действующий Koala и нарушать текущий сетевой сеанс | Полная B3 off/on/restart/default-route/sleep матрица на пользовательском хосте. | Изолированные profiles, physical bind, read-only observations текущего режима и локальные failure scenarios без изменения Koala. |
| Доверенный release signing key либо выбранный процесс его выпуска | Подпись распространяемого bundle и проверка доверия updater. SHA-256 без доверенного источника не заменяет подпись. | Весь check/download/version/mismatch/corruption/rollback поток и криптографическая проверка на изолированном тестовом ключе. Private keys не попадают в source, logs или release assets. |

Полная цель закрывается только после выполнения всех обязательных строк и release gates с итоговым evidence. Пока внешняя проверка не проведена, её статус остаётся открытым с точной причиной; локальный успех не подменяет этот результат.
