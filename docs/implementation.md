# qbutt: задание для реализации агентами

Читать вместе с `qbutt-architecture.md`. Этот файл является рабочим backlog и контрактом приёмки, а не утверждением, что соответствующие API уже существуют.

Уточнение требований: qbutt архитектурно независим от частного сервиса. Настройки принимают обычную Mihomo-подписку и позволяют выбрать узел; аккаунты, API сервиса и заранее заданные серверы не требуются. Обозначения S1/S2/S3 ниже — примеры независимых выходов для лаборатории. Публичный qbutt-net начинается с закреплённого публичного upstream без истории частных форков; обоснование приведено в `adr/0001-first-slice.md`.

## 1. Цель

Приоритеты уточнены 14 сентября 2026: сначала доказать concurrent multipath через реальные независимые VPN-выходы и завершить общий discovery. Затем интеграция интерфейса/диагностики и inbound; простой updater из GitHub Releases по примеру `element-max` остаётся последним. Очередность и бюджет времени — в разделе 8. Уже реализованный repair сохраняет обязательные проверки безопасности: это проверка и докачка файлов раздачи, а не восстановление установки qbutt.

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

## 8. Текущая очередность и ограничение затрат

Этот раздел задаёт порядок продолжения существующего goal; обязательный объём не сокращён. На момент пересмотра счётчик goal: 10 ч 25 мин 53 с. 20 сентября пользователь возобновил работу с остатком квоты 31% и разрешил продолжать до 5%: проверять остаток между этапами, сохранить запас на остановку процессов и cleanup, не использовать reset credits. После полного завершения цели либо достижения разрешённого предела квоты пользователь поручил сохранить результат, остановить стенды и выключить компьютер. Не повторять начальную разработку и доказанные проверки без изменения кода или новой причины.

| Порядок | Результат этапа | Оценка остатка |
| --- | --- | --- |
| 1 | Native + три реальных независимых VPN-выхода одновременно скачивают один публичный тестовый torrent. Сначала контролируемые пиры с отдельными pieces, затем публичный swarm. Один picker/storage, per-path payload и verified goodput, hashes/sizes. Разные выходные IP подтверждены независимыми пирами/серверными наблюдениями; локального capture туннелей недостаточно. | 2–4 ч |
| 2 | DHT/PEX/HTTP(S)/UDP trackers дают разные кандидаты через разрешённые paths в общий pool одного торрента; последующая одновременная загрузка доказана без ручного добавления всех пиров. Довести managed DHT, public/private/metadata policy и переходы. Повторить сравнение на проблемном публичном swarm. | 2–3 ч |
| 3 | Объединить диагностику и Qt acceptance; применить layout и встроенное тёмное оформление из 8.1. Одна актуальная сборка приложения и компонентов. | 1–2 ч |
| 4 | Завершить optional inbound lifecycle/TCP/UDP и независимую WAN/CGNAT-проверку. Шлюз не обязателен для обычной Mihomo-подписки. | 1–3 ч |
| 5 | Простой updater, финальное ревью/абляция, необходимая регрессия на окончательных бинарниках, публикация исходников и локального релиза. | 1–2 ч |

Ориентир полного остатка: **8–14 часов**, без гарантии срока и без ожидания доступов/внешней сети. До первого реального multipath-доказательства: **2–4 часа**. Пересчитать после этапа 1: проблемы transport/discovery могут увеличить остаток. Большой файл не обязателен: ограничить объём fixtures, измерять устойчивое окно, учитывать startup отдельно, удалять payload/runtime после сохранения компактного evidence.

Экономия квоты: один интегратор, максимум два сабагента на независимые ограниченные задачи; не дублировать аудит/сборку. У этапа один конкретный критерий выхода. Каждые 60–90 минут содержательной работы сверять результат и оценку остатка. Два одинаковых неуспешных прогона требуют разбора причины, а не нового полного прогона. Сохранять актуальные зависимости/build roots; полную матрицу повторять на итоговой композиции либо при затрагивающих её изменениях. Облачный CI не запускать.

Текущий срез после alpha.2 и IPC v5: результаты и точные ограничения перечислены в 9.1. In-place и staging теперь сохраняют selected/ignored physical mappings, AutoTMM, отдельные download/save paths и incomplete suffix; закрыты актуальные 106 restart faults, completion faults и реальные imported completion barriers. Controlled Native regression/capacity benchmark прошёл; независимый VPN ingress подтвердил TCP/uTP/DHT. Следующие локальные проверки — активные UDP/discovery policy transitions и окончательная storage/lifecycle композиция, затем ресурсные и route-selection сравнительные измерения. Koala off/on/restart по просьбе пользователя отложен, это не мешает остальной работе. Актуальные build/dependencies сохраняются; завершённые fixtures очищаются сразу после проверки.

### 8.1. Layout и встроенная тема

Источник layout — прочитанные без записи сохранённые настройки qBittorrent 5.2.3 и скриншоты пользователя. `ui-default-layout.json` содержит порядок, видимость и ширины всех 38 столбцов торрентов и 6 столбцов файлов; пользовательский профиль и credentials в него не входят.

- Новый профиль: sidebar скрыт, нижняя панель открыта на «Файлы», отношение высот 467:493. Видимые столбцы торрентов: очередь, имя, загружено, размер, скорость загрузки, прогресс, состояние, оставшееся время, осталось. Файлы: имя, общий размер, прогресс, приоритет, осталось, доступность.
- Начальный layout применять через именованные столбцы в существующих Qt-компонентах. Не зашивать QHeaderView blob и не перезаписывать пользовательские изменения. Ширины — логические пиксели Qt; адаптировать к DPI/размеру окна, проверить новый и уже настроенный профили.
- Первый запуск и release: встроенная тёмная палитра без выбранного `.qbtheme`, спокойные тёмные фоны и холодные акценты, близкие ICEBERG. Не копировать тему целиком; сохранить custom themes и ручной Light/System. Проверить текст, выделение, disabled controls, галочки, focus, таблицы и progress bars.
- Функциональные визуальные прогоны: Light + Fusion без custom theme в изолированном профиле. Проверки темы/релизного первого запуска: штатные настройки продукта без принуждения к Light. Общий переключатель стендов — `QBUTT_LAB_APPEARANCE=functional|product`, по умолчанию functional. Состояние controls проверять через Qt-свойства вместе со скриншотом. Тестовый профиль не попадает в release.


## 9. Реестр полной приёмки

### 9.1. Объём и подтверждённая база

Обязательный объём: T00–T02, A–D, результат inbound E и интеграционные проверки раздела 6. Отдельный новый сервер `qbutt-edge` — вариант реализации E: существующий gateway допустим, если сохраняет все endpoint, authentication, UDP и lifecycle требования. Отсутствие собственного gateway не закрывает и не отменяет проверку inbound.

Streaming, live same-file multi-swarm, удалённые BitTorrent workers, publisher-assisted delta, глобальный piece cache/CAS и одновременные transport variants одного edge остаются явно отложенными расширениями или гипотезами архитектуры. Предложения из исследовательского диалога сами по себе не превращают их в обязательные функции. Обязательный A3 включает локальный поиск и сопоставление файлов; его нельзя отложить вместе с live multi-swarm. Конкретные названия классов, `qbutt.db`, named pipes и WinSparkle не являются самостоятельными результатами приёмки: сохраняются владельцы контрактов и требуемое поведение. Решение о приватных inherited pipes первого child зафиксировано в [ADR](adr/0001-first-slice.md).

Публичная alpha.1 относится к исходникам `6e96bcce504772d89cfde038e9d18479b2543f47` от 12 сентября 2026 года. [Verification alpha.1](https://github.com/qbutt-org/qbutt/releases/download/v0.1.0-alpha.1/verification.json) связывает их с чистой сборкой, SHA-256 приложения `be45670a50a1667aaa1d079406618b6265447b18877e6e6abfbc1be2724fe67f` и qbutt-net `599a78654d71e245cb226dbe5f6080af863c4943` / `2453058bf906cace94a50fce57434857e4bf57db76a504ba4d0c1f6223eb4b0e`. Это исходная точка, а не утверждение о проверке будущих изменений.

| Evidence | Подтверждённый результат и граница |
| --- | --- |
| `native`, 8 checks | v1/v2/hybrid: selective download 54 149 bytes; полный payload 2 151 362 bytes с exact sizes; stop/restart/resume/recheck и сохранение payload после удаления задачи. Обычный recheck сохраняет лишний хвост 8 193 bytes. |
| `repair`, 23 checks | 21 вариант v1/v2/hybrid плюс проверки аутентификации и сохранности после shutdown. Corrupt/grow/shrink/missing-nonzero исправлены; v1 также проверяет уже заданный rename mapping, inserted bytes, source mutation и unknown files. Hardlink/reparse отвергаются. Missing-empty отвергает apply без создания файла; это проверенный отказ, а не поддержка такого repair. |
| `paths`, 8 checks | Реальный bundled child, выбранный loopback adapter, TCP peer и HTTP tracker, смерть child с остановкой передачи, retry с новым generation, stop/Native и охрана transitions. Verified payload 2 151 362 bytes. Relay stream bytes содержат протокольные данные и не являются wire traffic или measured goodput. |
| `completion-control` и `completion-repair`, по 1 check | Обычный auto-exit срабатывает; активный repair preview удерживает приложение после завершения другой раздачи. Полный rule engine и queued/nested-dialog race injection этим не проверены. |
| `final-ui`, 5 checks | Реальный offscreen Qt: контекстное меню repair, read-only таблица с относительными именами, согласие, apply/recheck, сохранность unknown file, Qt file dialog и выбор узла. Это не physical desktop interaction, не Windows native file dialog и не проверка длительных UI stalls. |

В [baseline](baseline.md) отдельно зафиксирован неизменённый upstream control. В [capabilities](capabilities.md) HTTPS-пробы ShadowQUIC/VLESS и таймаут Hysteria2 относятся к старому qbutt-net `a268eef...`; они не доказывают поведение bundled `599a786...` или всего торрент-трафика. Наличие тестового кода без successful evidence не закрывает проверку.

Последующая локальная объединённая сборка: source `858cd336f957a28bd4f779ec1c1293a5a9935f3b`, SHA-256 приложения `0c3649f952cac668f17e392221741212a393a05d4cb9f59aa2f6e8445f5160c1`, libtorrent `30fcb515bfa5fea20067643064d90b97cbd9c388`, прежний child `599a786...`. Manifest отмечает development/dirty: при сборке изменён lock libtorrent. Это не release. На этой сборке exit 0 и evidence `passed` получены для `native-46lvl2`, `repair-kBRSlx`, `staging-rwZpvO` (SQLite) и `profile-import-N571Pu`: native v1/v2/hybrid, 21 in-place вариант, полный и выборочный staging, четыре сочетания Bencode/SQLite при импорте, аварийное восстановление импорта и неизменность исходных profiles/payloads.

Staging fault corpus отдельно проверен на композиции исходников `c27db97797163092cfce5e7432c93f1c04e2b5a4` + schema fix `67675ac8c03d17c50afdea4312434100d91b1407`; SHA-256 fault binary `59dcc1e79e44cf31626b288874dc88dc5db9c88117117d5255458307cf411503`. Все 106 Legacy crash cases (`staging-faults-jwlwwO`) и 12 SQLite cases завершились exit 0 с проверкой после shutdown; отдельно прошли 21 malformed journal на каждом backend, selective recovery и реальные ошибки native resume storage с повтором после restart. Эти fault-результаты нельзя приписывать будущему финальному bundle без повторной проверки.

Два TCP пути в одной session/picker подтверждены на development source `e94339ea85b3fa888f7fa7ae302b3a9827c465da` с изменениями, сохранёнными затем в `788d63e3649eb2b50325b116b5e04cf6b1436c98`, binary SHA-256 `157ed3f812c1f0dead11e7fa6690d24b9995f31a3ff3f0146edb45616b2d0e37`, evidence `mixed-hnNgwD`: одновременная передача и повтор после неверного пути, каждый с exact payload 2 151 362 bytes.

Последующая проверка: `mixed-native-pN4HKX` от 13 сентября завершён passed — четыре локальных пути, непересекающиеся pieces, рост всех четырёх соединений одного infohash за 606 мс, exact sizes/SHA-256 для 2 151 362 bytes. `network-benchmark-iA2eOq`: 16 успешных окон, один tunnel 84.29 КиБ/с, Native + два tunnel 208.96 КиБ/с после setup; полное время малого файла 37.19/38.49 с. Это не реальный WAN/VPN и не полная UDP/discovery-приёмка.

20 сентября: `wan-u2giHC` — passed на app source `776dd3ad71c33a306294bb9575498a4629c35d4f`, binary SHA-256 `7849d1a68e773d909271330b5f1563d8f4bdb0ba508808bba287f475d63dae55`, libtorrent `d50a672789f3d190d9cea78c29d258065c390958`, qbutt-net `addc9d590d4ef34e751f684bb83dc4e058a7a130`. Один infohash, Native + три реальных VLESS/gRPC edge, четыре разных source IPv4 на независимом peer; по 4 194 304 bytes отдельными pieces с каждого пути, рост всех четырёх за одно окно 1502 мс. 16 777 216 bytes проверены по exact size/SHA-256. Это controlled TCP с ручным добавлением пиров, без утверждения о discovery/UDP/ускорении. Существующий Koala TUN не менялся; payload и временные настройки удалены.

Публичный swarm `public-swarm-10zjdB`: одно Native окно дало 5 767 168 новых verified bytes, затем one-tunnel warmup завершился timeout без пиров. Сравнение не завершено и не считается passed. Повторять после исправления managed UDP/discovery; payload обоих окон удалён.

Текущая WAN-проверка `wan-HVLPaw` — passed, app SHA-256 `67458231c404214dd09f24cbdf2be6cd857cf3d3b9a43641c0f249cf7816b5af`, libtorrent `6816464064abf008a9ccea59dee4160eba48761c`, child `64f2d908b6a90c13cc75259d0d85e5e3d0fa9806`. Native и три реальных VPN одновременно передали и подтвердили pieces; после закрытия соединений каждый путь сохраняет ровно 4 194 304 verified bytes. Общий файл 16 777 216 bytes, SHA-256 `1d3327cc891ea8872c28cf9bed7e35a4dbcd45bd2e1c96b5bd8d1bc692ccdab5`; независимый peer подтвердил четыре разных публичных IPv4. Recheck завершён и не начисляет повторно сетевые verified bytes. Это controlled TCP, не сравнение скорости публичного swarm.

Whole-app discovery `discovery-gDoBNE` — passed на app SHA-256 `d6e066684f7535f1d3bdc2ba5cca54d8ae8c800fa498fb9d7ddbfd32784d80d6`: два route-local DHT, HTTP и UDP tracker дали четыре разных peer в один pool. Без `addPeers` одновременно работали четыре TCP-соединения через два пути; disjoint pieces собрали 2 151 362 bytes с exact sizes/SHA-256. Сумма per-path verified counters совпала с payload. DHT использует разные node IDs, `ro=1`, без ложного `announce_peer` при неизвестном публичном endpoint.

Режим TCP+uTP `discovery-wkuHSd` — passed на app SHA-256 `651a9b88741626438324e0bf239b2d7c2f8cc83ab51b92bf6450986c4c6ce71b`, libtorrent `9aff1615fe70a055169f12700dfd4c84bb8b594e`, qbutt-net `1f1b4586cc1549c2c33f8983823931554df56a9e`. После четырёх uTP timeouts те же разрешённые поколения путей установили TCP: четыре одновременных peer, 1 081 344 + 1 070 018 verified bytes, exact sizes/hashes. Торрент активен до включения DHT; добавление поздних bootstrap routers теперь будит существующую очередь discovery. Компонентная проверка доказывает отсутствие TCP retry при forced uTP и отзыве поколения до исполнения queued retry. PEX проверен отдельно в `discovery-5ZdmXd`: peer B найден только через PEX от A, другой path/generation, exact payload; оба partial seed сохранили свои pieces и не скачивали друг у друга.

WAN gateway `gateway-wan-UogdS4` — passed на app SHA-256 `3451fa261e7c1b4ea4106e5c80c2ee241b1aa8a015edd9a5d02bae02580b00a8`: удалённый peer первым инициировал TCP к публичному lease, домашняя libtorrent сохранила original IP/port и path/generation, получила и проверила 524 288 bytes по SHA-256; carrier/relay counters и terminal rollover проверены. Gateway и peer на одном удалённом observer: это проверка remote gateway → home carrier, не доказательство независимой third-party reachability, UDP или CGNAT-топологии. Ephemeral процессы и payload очищены.

Оформление `qt-acceptance-NJpTdY` — три процесса завершились exit 0: новый продуктовый профиль (встроенный Dark/Fusion, без custom theme), перезапуск после пользовательских изменений layout/Light и новый функциональный Light/Fusion. Qt-свойства подтверждают все 38 столбцов торрентов и 6 файлов, sidebar и вкладку Files; PNG проверены. Driver SHA-256 `3be30638967e87ac75c1f64032fcea389964d438b2cf1df111811c840be872f6`. Тесты используют обычное сохранение настроек при shutdown, а не прямую запись внутренних settings. Полная Qt-приёмка этой композиции приведена следующим пунктом.

Полная Qt-приёмка `qt-acceptance-Fkwso1` — passed, driver SHA-256 `66024011a18490d5502a704c6e343513ffc3d81e1c4b92f8da7821ff55437948`. Реальные Qt widgets проверены в Light/Fusion: переключение paths, отмена поиска по 30 000 файлов без записи, preview/mapping, managed staged repair и explicit commit, completion stop, 300 diagnostics samples и redacted export, 2001 transfer row с составными фильтрами, восстановление Native. Максимальный event-loop gap 7 мс, максимальное взаимодействие с большой таблицей 4 мс, экспорт 59 мс; bound 250 мс. Authenticated v4 child отклонил все три пробы неверных credentials и завершился по EOF. Новый stopped repair job дожидается окончания native initialization; согласие на добавление явно включает создание отсутствующих пустых target files. Preview до согласия не пишет, существующие файлы заблокированы от записи до разрешения staging. Screenshots проверены; payload/profile/copied runtime удалены общим finally.

Публичный Ubuntu swarm `public-swarm-DDX0Dg` — passed на app SHA-256 `223f440397f878f970d304ac0e05b277305d6e39c4d5c36d6458379f9d2158d7`, тех же libtorrent `9aff1615` и child `1f1b4586`. Три окна Mixed по 32,26 с: 23 330 816, 24 379 392 и 27 262 976 independently hash-checked bytes; в каждом окне Native и все три реальных VPN имеют положительный verified contribution. Медиана 755 661 bytes/s при общем лимите 1 MiB/s; DHT 105–290 nodes. Это не сравнительный benchmark и не проверка полного ISO. Предыдущий Mixed warmup выявил ошибку app parsing: `resolveNative` возвращает три поля, обычный `resolve` — одно; исправление сохраняет строгую проверку path/generation. `path-dns-lyQ54q` отдельно проверил physical source, ответ, сохранение двух SOCKS paths/child и отказ устаревшему Native generation.

Контролируемый benchmark `network-benchmark-cTs9xu` — passed, 12 окон (3 rounds × 4 режима), неизменённый upstream SHA-256 `9393e0c523b35a437fb9b356b4c7c7402dbbd9d97b9c1ae519fd01f1219c471e`, qbutt `651a9b88741626438324e0bf239b2d7c2f8cc83ab51b92bf6450986c4c6ce71b`. Медианы verified bytes/s: upstream Native 86 565, qbutt Native 86 561, один tunnel 86 552, Mixed 213 086; Native regression 0,0045%, Mixed 2,462×. Каждый прогон собрал exact 2 151 362 bytes с SHA-256, redundant payload 0. Медианы полного completion 37,28/37,26/37,13/38,43 с включают startup. Стенд задаёт отдельные ограничения 96 KiB/s на пути и complementary reachability; он не доказывает выигрыш при общем last-mile cap или на любом публичном swarm.

На libtorrent `9aff1615` пересобраны и пройдены `route-policy-09el4E` и пять `selector-current-f57cffa4` сценариев. Component integration подтвердил смену HTTP/UDP tracker и DHT routes/identity, отмену старого tracker, 1 MiB webseed через authenticated SOCKS, 512 KiB automatic uTP; selector — network retry, immutable verified-block origin, private/unknown metadata pinning, choke/exploration bounds, corrupt/unchecked payload без credit, поздние v2 hashes и исключение padding из credit. Это не whole-app доказательство всей policy matrix или Internet UDP.

Native DHT late-listener regression исправлен в libtorrent `fb7bc0ef`: новый Native socket получает уже принадлежащие session bootstrap routers до bootstrap. Локальный `qbutt_dht_integration` с physical IPv4 воспроизвёл отказ на `9aff1615`, прошёл после исправления; managed DHT corpus также passed. `public-swarm-e1pPJc` на app SHA-256 `69e6945b9521a83b9128ab6bf9a2347e8a814476620490c0f179cf0873a69332` дал три Native окна без rejected attempts: 111 673 344 / 115 343 360 / 117 178 368 независимо hash-checked bytes, медиана 3 575 094 bytes/s при лимите 4 MiB/s. Полный ISO не проверялся. Полное сравнение `public-swarm-UkXQv3` остановилось на upstream warmup с DHT 0; это failed run, не сравнительный результат. Cold-start повтор с сохранённым binding (`public-swarm-4KuzP6`) также завершился upstream warmup timeout: причина этого публичного отказа окончательно не установлена, сравнительной оценки нет. Не подтвердившее гипотезу изменение запуска стенда удалено.

Общий application cap `network-benchmark-hI8y6A` — passed, шесть окон на app `223f4403`: при общем лимите 96 KiB/s Native 92 192 и Mixed 96 263 verified bytes/s, разница 4,42%; каждый payload exact/hash correct, redundant 0. Это общий лимит приложения, не физический last-mile/netem. `network-benchmark-JsQ8CU` — passed: после отключения плохого relay тот же peer автоматически перешёл на здоровый route за 10,46 с без addPeers/смены policy; Native connection сохранила local port/path/generation, итоговый hash верен, 10 947 повторных bytes учтены отдельно.

Completion на app `223f4403`: `policies-cftBN8` — passed (terminal/restart idempotency, import preview acknowledgement, repair defer, staging commit receipt, move before policy, explicit delete с сохранением unknown files); `policies-thresholds-eyzR2L` — passed для ratio 1 и seeding 7 s. Это обычный runtime без включённых fault hooks.

На app `69e6945b` whole-app `discovery-policy-XSm2oh` — passed: private и unknown→private собрали по 2 151 362 exact/hash-verified bytes только через pinned path. При включённых global DHT/PEX private не делает DHT queries и не рекламирует `ut_pex`; unknown выполнил один pinned `get_peers` до импорта metadata и ни одного после private metadata. Metadata внесена штатным duplicate `.torrent` import в существующий magnet: private seed по протоколу не предоставляет `ut_metadata`.

На том же app `webseed-xo1Ain` — passed: private HTTP webseed передал 2 151 362 verified bytes через первый path; после его остановки private остался активным с 0 bytes/HTTP/connections в течение 8 с. Публичный torrent затем получил столько же exact/hash-verified bytes через второй path, пока private оставался заблокирован. Напрямую достижимый Native HTTP canary не получил запросов. Mid-transfer reconnect отдельно не подтверждён.

Публичное сравнение `public-swarm-8CHdku` на app `69e6945b` и прежнем unchanged upstream прошло первый круг: upstream Native 3 688 619, qbutt Native 3 680 721, один VPN 3 632 088, Mixed 3 664 509 independently verified bytes/s при общем лимите 4 MiB/s. Все четыре пути Mixed внесли полезные bytes. Второй круг завершился Native warmup timeout (0 DHT nodes), поэтому общий suite **failed**, три сравнительных круга не подтверждены. Первые четыре окна использовали порты 46123–46126, отказ — 46127; более ранние отказы на 45123 не дают достаточного основания объявить проблему только портовой или только клиентской.

Входящий uTP подключён в libtorrent `d600f78d7`: только готовый advertised UDP route может принять новый peer; исходящие ассоциации остаются закрыты для входящих. Socket owner задаёт path/generation и transport, отозванные/family-mismatched/self endpoints отклоняются. Plain и TLS не делят публичный SYN port: TLS-uTP остаётся outgoing-only. Component `qbutt_dht_integration` прошёл, включая запрет unadvertised SYN и атомарный отказ при конфликте plain/TLS port. Whole-app `gateway-utp-EHCH6U` и TCP regression `gateway-bZYo9w` на app SHA-256 `2e811db141f335e6cb3181ec1e0e29cf82c352a5d611bd8a942265895672bd34` прошли: каждый получил ровно 2 151 362 проверенных bytes с совпадающим engine credit, сохранил original peer endpoint, продлил lease и пережил explicit reopen/terminal rollover. UDP прошёл через выбранный SOCKS adapter и datagram carrier без TCP ingress. Это контролируемый IPv4 стенд; WAN UDP, IPv6 и входящий DHT отдельно не доказаны.

HTTPS webseed `webseed-https-zDfAdj` на app `69e6945b` — passed: официальный Ubuntu SHA256SUMS (893 bytes с заранее закреплённым SHA-256) получен исключительно через выбранный subscription path с обычной проверкой TLS certificate. Engine credit: выбранный path 893, Native/остальные 0; relay 6500 bytes. Это небольшой реальный HTTPS transfer, без negative invalid-certificate или независимого packet capture.

Независимый WAN gateway preflight `gateway-wan-UBZTYY`, `gateway-wan-G1G7aX` и `gateway-wan-cepSss` — failed до запуска gateway: отдельные выбранные VPN adapters S1/S2 приняли 59 bytes HTTP-запроса, но scoped observer capture не увидел SYN и HTTP listener не принял соединение. Проверены high ports 61468/59569 и ранее успешный 40971; гипотеза только диапазона портов не подтвердилась. Fixture требует доказанный внешний source IP, отличный от домашнего SSH origin и observer; SOCKS CONNECT сам по себе не считается успешным VPN ingress. Причина недоставки через выбранные adapters пока не установлена.

Последующая IPv6 приёмка на app `2e811db1`: `gateway-ipv6-IhytWt` (TCP) и `gateway-ipv6-utp-1NQuMa` (uTP) — passed. Каждый получил 2 151 362 exact/hash-verified bytes через advertised IPv6 lease, сохранил исходный endpoint и прошёл renewal/reopen/terminal rollover. Это owned `/128` loopback alias, не WAN IPv6.

Входящий DHT на той же сборке: `gateway-utp-dht-RtLiF7` (IPv4) и `gateway-ipv6-utp-dht-FdGhKk` (IPv6) — passed. BEP5 `ping`/`get_peers` вернули совпадающие transaction/node ID, token и точный advertised source endpoint при активном path/generation; затем тот же lease принял полный uTP payload и прошёл lifecycle. Bootstrap ограничен owned loopback listener; outgoing bootstrap и WAN DHT этим не подтверждены.

Объединённая development-сборка app SHA-256 `a90ed0a7054135912fa71763d99a59014b3c2bcd02269254319c10d6a95f690d` прошла `native-PrN885`, `repair-hssut0`, Qt `qt-acceptance-ss9sUh` и appearance `qt-acceptance-pAPNch`: v1/v2/hybrid, 24 repair candidates, сохранность protected/unknown targets, staging/completion/diagnostics, 30 000-file search cancellation и responsive UI. Встроенная dark, сохранённая Light и functional Light проверены отдельно; новый layout и пользовательские изменения сохраняются. Native incoming uTP `native-inbound-utp-TauZ10` — passed с protocol=UTP до старта: ровно один инициирующий seed, 2 151 362 bytes, точные sizes/hashes, нет managed paths/child/TCP. Unchanged upstream control `native-inbound-utp-x8dFT6` также passed. Первый прогон с изменением protocol после старта завершился timeout без установленной причины; последующие успехи не объявляются исправлением этого отказа.

Простой updater реализован в `ReleaseUpdater` и `ReleaseUpdateDialog`: ручные check/download полного согласованного ZIP из qbutt GitHub Releases, SemVer с prerelease, HTTPS, ограниченные redirects/размеры и SHA-256 из API вместе с `SHA256SUMS.txt`. `qbutt-update-1zi4n1yd` — 11 process-level HTTPS/Qt сценариев passed: current/version ordering, success/cancel, повреждение archive/checksum, interruption, wrong asset, insecure redirect, HTTP error и untrusted certificate. Предыдущий destination и profile не изменены при ошибке/отмене, partial не остаётся. Qt properties, text geometry и screenshots проверены; live check GitHub API прошёл. Скачивание использует обычную сеть независимо от torrent paths, что явно указано в диалоге. Установка/миграция не выполняются; SHA-256 не объявляется подписью издателя. Эти результаты предшествуют отдельной release-сборке с `deprecated-functions=OFF`; готовый portable требует своей проверки.

Публичный [v0.1.0-alpha.2](https://github.com/qbutt-org/qbutt/releases/tag/v0.1.0-alpha.2) собран локально из чистого `210f13479cce747fa3d54fc1c23319e96ed846cb` с `deprecated-functions=OFF`; app SHA-256 `bb076791f605301d4cc30945a22653c7b905cbadc95e48173be81de0cab40ae5`, child `306c7b047e9ae3b0152ef84d692012bcebdc3edaa46c10864c3b71437fcfee0e`. Четыре assets, tag commit, публичность и скачанный checksum manifest проверены через GitHub. Именно эта композиция прошла `native-8EB0EC`, `repair-lHEBOt` (28 checks), `native-inbound-utp-meHVzA`, `mixed-native-3GXwQ3`, `discovery-Laiw3y`, `gateway-ipv6-utp-dht-kYKhBj`, Qt `qt-acceptance-ppkuMm`, appearance `qt-acceptance-60pgMD` и 11 updater HTTPS/Qt checks `qbutt-update-0kptpa97`. В release приложен компактный `verification.json`; это alpha, а не закрытие всех gates или новая WAN-проверка.

После alpha.2 устранено искусственное разделение одного настроенного сервера по имени узла/порту/протоколу. qbutt-net `492513a4` вычисляет нормализованный `configuredServerId`, повторно сверяет его перед созданием транспорта и использует IPC v5. Приложение отвергает второй активный транспорт того же Edge и произвольный API `edgeId`; смена после stop сохраняет Path ID с новым generation. `server-identity-4fYUv9` passed с 2 151 362 exact/hash-verified bytes; `mixed-native-wAQIZw` passed с четырьмя одновременно растущими соединениями и `discovery-KHPSb0` passed с DHT + HTTP/UDP trackers без ручного addPeers. Разные DNS-алиасы физически одного сервера пока не распознаются; разные IDs не доказывают независимые публичные выходы. Эти изменения не входят в опубликованный alpha.2.

Приёмка IPC v5 продолжена на app SHA-256 `e84b2627eb0ea7a73c0610d58d3658bcace17c69beaecc5811c01d8d28eb8fea`: Qt `qt-acceptance-15Hfqd`, все 24 сценария `smoke:path-auth` (fake child `92e905a3`, включая terminal rollover), `discovery-policy-1OdsvK`, HTTP webseed `webseed-lG8d9s` и PEX `discovery-qKsGKg` — passed. Private/unknown metadata сохраняли pinned scope, retired private webseed не использовал доступный соседний path или Native, публичный webseed через соседний path получил exact payload. `gateway-ipv6-utp-dht-XBhl0D` с чистым child `ecc27be6` из `492513a4` также passed: incoming DHT, 2 151 362 verified uTP bytes и generation lifecycle. Это повторная проверка затронутых контрактов, не WAN/final-release матрица.

Source-аудит libtorrent `d600f78` подтвердил сохранение общего session/torrent connection budget и dedup по исходному peer endpoint: managed dial регистрируется через штатные `insert_peer`/`set_connection`; `peer_list.cpp` не изменён относительно закреплённого upstream. Routed TCP retry повторно проверяет connection, policy/generation и лимиты, не использует `ignore_limit`. Штатный incoming slack и ограниченные gateway handshakes не равны строгому лимиту всех ОС-сокетов.

Whole-app `connection-budget-0PiBvR` и повтор после абляции `connection-budget-knyLIT` — passed на app `e84b2627`/child `ecc27be6`. Два infohash, четыре seeds и два managed paths: при session/per-torrent лимитах 3/2 максимум три соединения, после снижения per-torrent до 1 — два. В каждом состоянии 30 snapshots без превышения лимитов и duplicate original endpoints; оба payload по 2 151 362 bytes имеют exact sizes/SHA-256. Повторные candidates внесены через `addPeers`; пересекающиеся DHT/tracker responses, incoming slack и pending handshakes этим не проверены. Первый `connection-budget-rHqOkN` прошёл только начальный лимит, затем завершился HTTP connection error; причина не установлена, приложение не было доказанно аварийно завершено. В fixture добавлен failure stack/exit-code context; два последующих полных прогона прошли без изменения production code.

Отрицательная HTTPS webseed приёмка `webseed-tls-rejection-Fs5pui` на той же app/IPC v5 — passed. Python TLS responder получил четыре `TLSV1_ALERT_UNKNOWN_CA` от qbutt через authenticated SOCKS; HTTP, useful/verified bytes и обращения к достижимому Native canary равны нулю. Отдельный клиент с явно указанным временным CA и правильным numeric SAN получил exact SHA-256 payload. Системные trust roots не менялись; expiry/name mismatch/reconnect этим не закрыты. Два предыдущих варианта fixture с Bun HTTPS не давали TLS error evidence и не засчитаны как продуктовый успех или дефект.

`smoke:discovery-dedup` — passed на той же композиции: `discovery-OI3uwE` с TCP и `discovery-vT9cza` с automatic uTP→TCP. Один общий endpoint возвращён DHT обоих paths и HTTP/UDP trackers; четыре complementary seeds остались четырьмя активными original endpoints без duplicates. Последний прогон также проверил `knownPeers == 4` в 30 snapshots. Оба paths внесли полезные bytes, конечные 2 151 362 bytes имеют exact sizes/SHA-256, все seeds скачали 0 payload bytes. Это локальное outgoing discovery, без заявления об incoming/reconnect race coverage.

Managed repair на development app `e7928bc8` (`1526ffb6d`) использует snapshot реальных native mappings после ownership/drain. `repair-mappings-3UZGk4` — 6 проверок passed: v1/v2/hybrid с отдельными download/save directories, `.!qB`, `.unwanted`, selected priorities, cancel/restart и обычной докачкой/переносом; итоговые выбранные 54 149 bytes проверены по exact sizes/SHA-256. Во время analysis/apply/recheck ignored-файлы не создаются и не обрезаются, существующий лишний хвост сохраняется. Последующая обычная v1-загрузка может использовать shared boundary pieces; неизменность ignored-файла при ней не обещается. Регрессии той же сборки: `repair-XHVMzA` — 28 checks passed, `staging-AdaolL` — 10 checks passed. AutoTMM и staged update с раздельными каталогами/суффиксом пока ограничены явно.

На той же сборке A3 `repair-index-layout-zB2ZwH` — passed: пять исходных файлов переименованы, у target изменены имена, порядок и piece size 16→64 KiB. Production index выбрал источники, проверены все 33 target pieces; staged prepare/commit/restart сохранил источники и unknown files, получил exact sizes/SHA-256 для 2 151 362 bytes при 0 downloaded payload. HTTP `webseed-reconnect-ERQm0X` и private `webseed-reconnect-6YPtjt` — passed: обрыв во время загрузки, закрытие старых sockets, автоматическое продолжение public на другом path; private остаётся без новых запросов 8 с и продолжает только после reopen своего path/new generation. Оба payload по 2 151 362 bytes проверены, Native canary — 0 requests; перезапуска torrent не было.

Публичный comparative `public-swarm-TSroIP` остаётся failed: первый круг дал useful bytes на Native и всех трёх VPN в Mixed, но второй Native warmup остановился с DHT 0 и одним кандидатом от trackers. Последующий однооконный diagnostic `public-swarm-4fVb6o` на `e7928bc8` — passed: Native DHT 75, 55 836 672 новых independently checked bytes за 17,26 с. Ограниченный startup capture `qbutt-native-startup-20260920` содержит 348 исходящих/80 входящих DHT packets на physical Ethernet и 0 на Meta; остановлен на первых 512 packets за 1,84 с, не является capture всей загрузки. Причина прежних остановок не установлена; diagnostic не заменяет comparative benchmark.

AutoTMM in-place на app `b8324b45` (`e4515741a`): `repair-mappings-2OHaC4` — 8 checks passed. Помимо manual v1/v2/hybrid проверены inherited category roots, запрет изменения/удаления активной категории и её родителя, default save/download roots и enable flag во время ownership. Cancel/restart не отключает AutoTMM; после repair категория снова допускает штатное перемещение, выбранные 54 149 bytes сохраняют exact sizes/SHA-256 после restart. Повтор после абляции fixtures: `repair-index-layout-ZtykXQ` и private `webseed-reconnect-5y4I1I` — passed на предыдущем app `e7928bc8`. Публичный comparative `public-swarm-xpfvr2` снова failed на Native warmup (DHT 0); новых comparative выводов нет.

Объединённый Qt driver после AutoTMM (`5314e318`), `qt-acceptance-NDMiRJ` — passed с protocol v5, тремя открытыми/отозванными paths и 15 authenticated connections. Новый Native diagnostic `public-swarm-A3e3UG` передал 524 288 independently checked bytes при DHT 0; capture `qbutt-native-retry-20260920` за полный запуск содержит 14 исходящих DHT packets на physical Ethernet, 0 входящих UDP и 0 packets на Meta. Предыдущий успешный DHT capture и этот отказ отличаются наблюдаемыми ответами сети; причина их отсутствия не объявлена исправленной.

Staged layouts на app `aa934149` (`dd6f09053`): `staging-receipt-mappings-IGDL1f` — 4 сценария passed. Manual и AutoTMM сохраняют отдельные save/download roots и `.!qB`; preview cancel/restart не пишет оригиналы, commit использует штатный перенос/переименование после retirement журнала. Проверены реальный отказ записи resume, последующий restart/recovery, rollback с сохранением оригиналов и загрузка старого v2 journal, где resume называл временный payload. Новый journal v3 сохраняет логическую политику отдельно от временного physical root. Три commit-сценария дали по 2 151 362 bytes с exact sizes/SHA-256 и штатным engine recheck; unknown/source files сохранены. Первый прогон `staging-receipt-mappings-Z3jFBC` остановил legacy recheck раньше завершения; исправлено ожидание в fixture, production code повторно не менялся.

На той же сборке независимый WAN TCP `gateway-wan-2mGt5m` — passed: контролируемый peer через отдельный VPN exit первым подключился к публичному gateway endpoint. Scoped SYN capture подтвердил source IP, отличный от домашнего и gateway; libtorrent сохранила точный remote endpoint/path/generation и проверила 524 288 bytes по SHA-256. Terminal lease retirement также прошёл. Предыдущий preflight использовал фиктивный HTTP `Host: observer`; после замены на настоящий numeric destination запрос дошёл и получил ответ. Старые отказы не доказывали сетевую недоступность gateway. Это реальный входящий TCP с независимого VPN-выхода, но не отдельный физический peer host, не доказательство CGNAT у провайдера и не WAN UDP/uTP/DHT.

Публичный benchmark `public-swarm-eg3p8b` на app `b8324b45` использовал один peer port во всех последовательных окнах. Семь окон завершились с independently checked pieces, два Mixed окна дали полезные bytes по всем четырём paths. Восьмое окно unchanged upstream завершилось timeout с DHT 0, поэтому весь comparative run — failed. Отдельная UDP-проба без qbutt показала зависимость ответов от source port; одинаковый порт убрал эту переменную, но не нестабильность внешнего discovery. Ни 5% Native gate, ни устойчивый публичный speedup этим прогоном не закрыты.

На `aa934149` регрессия `staging-qyaxO1` — 10 checks passed (full/selected v1/v2/hybrid, hardlink/reparse и изменение содержимого при сохранённом mtime). Пересобранный Qt driver `67d36f9d`, `qt-acceptance-h8vXjK` — passed. `policy-transition-dzosXt` — passed после абляции и добавления автоматической очистки: один активный torrent прошёл Mixed → Tunnels only → Pinned, старый Native TCP socket закрылся, доступный Native canary не получил соединений, счётчики/сокеты второго tunnel перестали расти, оставшийся pinned path завершил 2 151 362 bytes с exact sizes/SHA-256. Это TCP peer matrix, без заявления о discovery/uTP/webseed transitions.

Journal regression `staging-journal-aaKeH5` на `aa934149` — 27 checks passed: malformed identities/selection/index/size/path/state/version/JSON отвергаются без записи и после shutdown, валидный journal восстанавливается, v1 остаётся rollback-only с сохранением данных. Native recheck и startup hold продолжают использовать один engine writer.

Контролируемый capacity benchmark `network-benchmark-sXgEQM` на `aa934149` — passed: 16 окон, четыре rounds с перестановкой порядка, unchanged upstream `9393e0c5`. Медианы verified bytes/s: upstream Native 86 612, qbutt Native 87 929, один tunnel 86 617, Mixed 212 932; Mixed 2,458×, Native median gate ≤5% выполнен (наблюдаемая разница −1,52%, не заявление об ускорении Native). Каждый payload — 2 151 362 bytes с exact sizes/SHA-256, redundant payload 0. Медианы полного completion 37,27 / 32,05 / 37,12 / 38,47 с учитывают startup. Стенд с раздельными лимитами путей не доказывает public-swarm speedup или общий physical last-mile; CPU/RAM/I/O этим прогоном не измерялись.

В WAN UDP-прогоне `gateway-wan-utp-dht-gBh1tf` на `aa934149` отдельный checkpoint входящего DHT прошёл: через независимый VPN exit получены связанные transaction ID ответы `ping/get_peers`, постоянный node ID и token, запросы и ответы наблюдались на публичном lease port. Весь прогон — failed: uTP SYN дошёл до gateway, но native peer/payload за 45 с не появились. Сам по себе этот DHT checkpoint не закрывает WAN uTP, physical carrier bypass или BEP42 enforcement.

Последующий `gateway-wan-utp-dht-8bw1KW` на неизменённом `aa934149` — passed, 8 checks. Independent VPN source передал 524 288 bytes по uTP через публичный gateway, libtorrent сохранила original endpoint/path/generation, exact size и SHA-256 сошлись; исходящий source peer получил 0 payload bytes и видел только leased endpoint. Ограниченный захват 32 записей SYN/STATE/DATA содержит только 20-byte uTP headers, без BitTorrent payload. Входящий DHT снова прошёл; terminal lease retirement и cleanup выполнены. Отказ был в тестовом source: `outgoing_interfaces=127.0.0.1` несовместим с proxy-backed uTP local endpoint stock libtorrent (`No such device`). Локальный preflight сначала воспроизвёл этот отказ, после удаления только этой привязки передал exact 512 KiB (`wan-source-preflight-hqjfdc`); proxy для peers остался обязательным. Это independent VPN ingress, не отдельный физический peer host, не подтверждение CGNAT у провайдера, physical carrier bypass или BEP42 enforcement.

`profile-import-Agmms7` на `aa934149` — 10 checks passed после пересборки service driver на текущих native services. Помимо Bencode/SQLite, writer exclusion, mappings, interruption/backup recovery и неизменности источников проверена реальная связка импорта с completion engine: три импортированные раздачи имеют matching removal rule, проходят native recheck и restart, остаются до acknowledgement. Подтверждение одной удаляет только её задачу, сохраняет все исходные payloads и не повторяется после следующего restart. Generated profiles/payloads удалены после проверки; compact evidence сохранён.

Completion faults на отдельном app `c8ccef25` с двумя fault flags ON — 7 сценариев passed: `before_claim-r7Op5C`, `claimed-DuylM7`, `dispatched-kDP9uG`, `acknowledged-xH1ugt`, `journal-write-uxgfgl`, `queued-EFIWPj`, `modal-H4Qpy6` (полные prefixes `policies-fault-*` или `policies-*`). Проверены crash/two restarts, durable at-most-once claims, реальный отказ замены журнала и повторная проверка repair ownership из очереди Qt и modal loop. Все payloads по 2 151 362 bytes сохранили exact hashes/sizes; generated bulk удалён после успеха. Обычный app `aa934149` не содержит fault overrides; конфигурация основного build возвращена в OFF.

На текущем C++ source `dd6f09053`, отдельный fault app `c8ccef25`: `staging-faults-byrn00` — все 106 crash/restart cases passed, включая rollback/forward recovery, empty target, v2/hybrid. Проверки выполнены после shutdown; источники и unknown files сохранились. Каждый завершённый case удаляет свои payloads сразу, после всего прогона удалены generated fixtures/profile. `staging-receipt-mappings-XRnKFk` — четыре сценария layouts/completion passed; focused `staging-receipt-mappings-Tjiflz` дополнительно подтвердил отсутствие completion claim при реальном отказе native resume persistence. После восстановления действие использует конечный каталог, после restart не повторяется. Это проверка fault binary и текущего normal app, не будущего release bundle.

`direct-transition-RCNt45` на `aa934149` — passed: один активный torrent без stop/start прошёл Direct → Tunnels only → Direct. В начальном Direct получены 34 538 payload bytes, через tunnel — 51 771, после возврата Native — положительная передача и полный exact payload 2 151 362 bytes. Исходный Native socket закрыт; достижимый canary не получил новых соединений за 5588 мс в Tunnels only, пока tunnel передавал данные. Child/listener закрыты при возврате Direct; retired relay counters больше не росли. Это TCP peer policy test, не доказательство полного UDP/discovery scope.

Storage на normal app `aa934149`: `storage-faults-oaHo9z` подтвердил orphan-owner cleanup, disk-full до создания staging, отказ после исчезновения тома и подмены volume identity. Прогон остановился на ошибке стенда: проверял английское слово в локализованном ответе `fsutil`. После удаления этой избыточной проверки `storage-faults-7r2jwS` — passed для оставшихся long/case и third-party sharing cases: путь 272 символа, разные `Save.dat`/`save.dat`, отказ перед записью при чужом handle и успешный recovery после его закрытия. VHDX без drive letters отсоединены и удалены; чувствительность к регистру проверена фактическим содержимым обоих файлов. Все шесть сценариев выполнены на одном app, полного нового шестисценарного прогона после изменения только проверки локализованного текста не было.

Repair preview driver пересобран на текущих native libraries (`dd6f09053`): `repair-product-IIJhQk` — 6 checks passed. Реальный offscreen Qt проверяет target `.torrent`, explicit mapping, раздельные candidate/verified/network/temp bytes, in-place/staged выбор, отмену анализа дерева из 4000 файлов и отказ для hardlink/reparse/несуществующего source. Снимки содержимого до/после совпали, profile/resume/journal не созданы до согласия. Light/Fusion screenshot просмотрен вместе со свойствами controls; generated bulk удалён, evidence/PNG сохранены.

### 9.2. Обязательные условия по направлениям

Статусы: **закрыто** — выполнен весь указанный результат; **частично** — есть конкретная реализация/проверка, но условие справа остаётся; **открыто** — сквозной результат не реализован или не подтверждён. Статус меняется только вместе с проверенными revision, результатом процесса и evidence. Будущие API в этой таблице не объявляются существующими.

| Условие / статус | Реализация и имеющиеся доказательства | Что необходимо для закрытия |
| --- | --- | --- |
| **T00 — закрыто для функционального baseline** | [Lock](../upstream-lock.json), [build entrypoint](../scripts/build-windows.ps1), [baseline](baseline.md), alpha.1 native evidence; отдельный профиль в [profile_p.cpp](../src/base/profile_p.cpp). | Сохранять воспроизводимую сборку и native regression при обновлении компонентов. Сравнение производительности закрывается отдельно в разделе 9.3. |
| **T01 — частично** | [Генератор](../tests/fixtures/generate.ts), [native](../tests/native-smoke.ts), [repair](../tests/repair/smoke.ts), [Mixed lab](../tests/network-lab/mixed.ts), [staging faults](../tests/repair/staging-faults.ts). Есть несколько piece sizes, v1 boundary, Unicode, filesystem negatives, два дополняющих peer за разными выходами и crash recovery. | Полный storage/network corpus из архитектуры §9, включая long/case/traversal layouts, смену тома и нехватку места; отрицательный baseline Mixed и итоговый Native + несколько remote paths. |
| **T02 — частично** | [PathManager](../src/base/net/pathmanager.cpp) владеет несколькими path/generation и bounded control; [RepairService](../src/base/bittorrent/repairservice.h) и [StagingOperation](../src/base/bittorrent/stagingoperation.cpp) — repair, journal и recovery. [API repair](../src/webui/api/repaircontroller.cpp) проверяет operation id и consent. Native resume revision подтверждается фактическим завершением записи. | Объединить Edge/Transport/PathContext/NetworkPolicy и CompletionEvent между владельцами; повторить malformed/oversized/incompatible child, stale generation и storage failures на итоговом bundle. Не вводить отдельный RPC-фреймворк или копию session state. |
| **A1 — частично** | [repairanalysis.cpp](../src/base/bittorrent/repairanalysis.cpp) читает целевые v1 pieces и v2 roots/piece layers под [guard](../src/base/bittorrent/repairfileguard.cpp); worker вынесен из UI. Alpha подтверждает read-only и verified bytes для управляемой остановленной раздачи. | Дать анализ целевой `.torrent` и выбранных каталогов до запуска записи, включая выбранные файлы и найденные mappings; честно разделить candidate/verified/download/temp bytes. Проверить отмену анализа и отсутствие любых изменений, включая создание отсутствующих файлов, на всех поддержанных layouts. |
| **A2 — частично** | [RepairService](../src/base/bittorrent/repairservice.cpp), [TorrentImpl](../src/base/bittorrent/torrentimpl.cpp), [SessionImpl](../src/base/bittorrent/sessionimpl.cpp): admission, drain, snapshot физических mappings, selected-only изменение размеров/создание пустых файлов и стандартный recheck. `smoke:repair-mappings` проверяет selected/ignored, `.!qB`, `.unwanted`, отдельный download path, AutoTMM, cancel/restart и штатную докачку с exact sizes/hashes (9.1). Исключение из `truncate_files` обосновано в ADR. | Итоговая совмещённая fault-проверка ownership; ограничения staged update относятся к A4. Не обещать rollback для in-place. |
| **A3 — частично** | Staged analysis индексирует выбранные корни, принимает явные mappings и проверяет кандидатов в целевом v1/v2 layout; native staging fixture покрывает renamed sources и изменение содержимого при восстановленном mtime. | Завершить полный corpus перестановок, другого piece size, отмены и больших деревьев; проверить GUI сопоставления и границы read-only index. Не обещать универсальную delta после вставки bytes. |
| **A4/A5 — частично** | [StagingOperation](../src/base/bittorrent/stagingoperation.cpp): независимые payload/backup, source guards, точные sizes/hashes, durable journal, commit/rollback, native resume receipt и startup hold. Обычные и аварийные проверки перечислены в 9.1. | Повторить полный fault corpus вместе с completion policies на итоговом bundle; добрать реальные disk-full/volume/layout сценарии. Disk-full не переключает режим в in-place; неизвестные файлы и незавершённые журналы сохраняются. |
| **B1 — частично** | [PathManager](../src/base/net/pathmanager.cpp) говорит с bundled qbutt-net по private inherited stdio protocol v5: strict hello/envelopes, кадр до 64 KiB, authenticated loopback payload и bounded status с семью wire counters. [Path auth](../tests/network-lab/path-auth.ts) проверяет incompatible child, malformed replies, stale generations, delayed status и terminal rollover; [gateway](../tests/network-lab/gateway.ts) — реальный child и lease lifecycle. | Повторить совмещённые crash/EOF/backpressure, лимиты и освобождение TCP/UDP listeners на итоговом bundle; локальные component/fixture passes не заменяют эту release-проверку. |
| **B2/B4 — частично** | [PathManager](../src/base/net/pathmanager.cpp) импортирует обычную подписку или локальный YAML, открывает выбранные adapters как отдельные path/generation и сохраняет fail-closed policy. Один torrent использовал Native + три real VPN TCP выхода; локальные DHT/tracker и PEX дали кандидатов через разные paths (9.1, `discovery-5ZdmXd`). | Проверить aliases одного edge, transport fallback и смену endpoints/generation, recovery выбора, реальные UDP/IPv4/IPv6 probes и пределы adapter capabilities; непроверенное оставлять unknown. |
| **B3/B5 — частично** | Физический interface передаётся child; приложение не устанавливает TUN и не импортирует глобальные routing/DNS правила. Старые HTTPS probes при Koala on описаны отдельно. | Проверить реальный Native/remote egress, bootstrap/DNS, отсутствие recursion/double tunnel при Koala off/on/restart, смене default route, sleep/resume и отказе интерфейса. Итоговый diagnostics export не содержит secrets/личные адреса без выбора пользователя. Изменение Koala требует отдельной авторизации; локальные проверки не ждут её. |
| **C1 — частично** | [SessionImpl](../src/base/bittorrent/sessionimpl.cpp) сохраняет одну libtorrent session и стандартный picker на torrent; peer I/O получает route context через selector, без session-wide SOCKS. `smoke:path-auth` проверяет отсутствие глобального proxy; локальный Mixed и WAN с Native + тремя VPN подтвердили параллельные TCP peers одного infohash. | На итоговом engine повторить Native baseline и сквозную матрицу peer/tracker/DHT/metadata/webseed/UDP call sites; успешные TCP peers не закрывают остальные классы. |
| **C2 — частично** | [Lock](../upstream-lock.json) фиксирует qbutt-libtorrent с Native/SOCKS routes и verified-block attribution. Одна session/picker собрала disjoint pieces через два локальных пути и Native + три реальных VPN; peer/path/generation и original endpoints видны отдельно (9.1). | Проверить общий connection budget, dedup и route-local reconnect при плохом Native/remote пути, aliases и default Native regression на финальном engine; незавершённый Mixed warmup не считать успехом. |
| **C3 — частично** | Managed UDP/DHT, route-local bootstrap DNS и физически привязанный Native DNS реализованы; `path-dns-lyQ54q` прошёл с сохранением двух SOCKS paths и child после DNS ответа. Libtorrent валидирует BEP42 hint по endpoint/transaction/generation. Whole-app DHT + HTTP/UDP tracker union, automatic uTP→TCP retry и локальный PEX через другой path приведены в 9.1. Там же controlled IPv4/IPv6 gateway lease проверил входящий DHT и uTP с exact payload. | Нужны gateway UDP tracker и SSL-uTP, family mismatch fail-closed, WAN UDP/DHT, а также HTTPS/webseed и public/private/metadata scope в полной policy matrix. Проверить external source/port, announce и self-connection в независимой WAN-сети; controlled loopback не считать таким доказательством. |
| **C4 — частично** | [PathManager](../src/base/net/pathmanager.cpp) синхронно меняет Pinned/Mixed/Tunnels only и отзывает generation; child death оставляет managed blocked. `smoke:tunnels` проверяет живую задачу Pinned→Tunnels only, два пути и retry; `smoke:path-auth` проверяет отказы и terminal rollover. Private/unknown metadata остаются pinned. | Полная Direct/Pinned/Mixed/Tunnels only матрица с активными задачами и TCP/UDP отказами: закрытие старых sockets, отсутствие Native discovery/payload при Tunnels only, private torrent без DHT/PEX вне своего source scope. |
| **C5/C6 — частично** | [PeerRouteSelector](../src/base/net/peerrouteselector.cpp) учитывает per-peer/path failures, demand/choke и hash-verified block credit, ограничивает exploration; двухпутевой wrong-route retry и локальный PEX B на другом path прошли. Automatic uTP→TCP retry остаётся в том же разрешённом generation. | После исправления Native DNS Mixed снова передаёт данные через четыре реальных пути; остаются changing IP, bad-path/no-benefit/last-mile сравнения и итоговые aliases/restart проверки 9.3. Reward не должен смешивать verified bytes с relay/wire или повторным recheck. |
| **D1 — частично** | [RepairDialog](../src/gui/repairdialog.cpp), [PathsWidget](../src/gui/pathswidget.cpp) и Qt `qt-acceptance-Fkwso1`: поиск 30 000 файлов с отменой, staged repair/commit, три пути, completion stop, диагностика и 2001 transfer row с фильтрами path/source; максимальный event-loop gap 7 мс (9.1). | Повторить на итоговом bundle и проверить долгие реальные I/O/probe/cancel и Windows desktop/native dialogs; offscreen Qt не доказывает физическое взаимодействие. API остаётся authenticated и versioned. |
| **D1 diagnostics — частично** | [NetworkDiagnosticsDialog](../src/gui/networkdiagnosticsdialog.cpp), фильтры и authenticated API имеют bounded history, route admission, verified-block credit и отдельные relay/carrier counters. WAN проверил credit после закрытия peers и отсутствие повторного credit при recheck; `qt-acceptance-Fkwso1` проверил 300 samples, семь wire counters и redacted export с malicious names, passkeys, URL и адресами. | На реальных отказах проверить смысл demand/choke/disk/rate и удержание privacy между private/public paths. Verified credit не является глобальной дедупликацией повреждённых повторных загрузок или wire traffic. |
| **D2/D5 — частично** | [CompletionPolicy](../src/base/bittorrent/completionpolicy.cpp) реализует правила `wanted_files_committed` с category/tag/ratio/time, preview, reason journal, idempotent claims и отдельное согласие на delete-data; `smoke:policies`/fault fixtures покрывают repair/staging/move/restart сценарии, Qt acceptance — completion stop. `remove_torrent` сохраняет payload. | Повторить rule/fault corpus на итоговом bundle вместе с импортом и длительными UI races; подтвердить отсутствие premature action при queued/nested-dialog и failed migration. |
| **D3 — частично** | [ProfileImport](../src/base/profileimport.cpp), [Qt dialog](../src/gui/profileimportdialog.cpp) и startup recovery копируют разрешённые настройки/metadata в отдельный профиль. Bencode/SQLite, mappings, writer exclusion, crash/backup recovery и неизменность источника проверены в 9.1. Импортированные раздачи stopped/manual и сохраняют completion preview marker. | Проверить marker вместе с полным D2 rule engine и финальным UI. Явно показать ограничения неподдержанных source schemas/неполной metadata; source profile и глобальная сетевая политика не переносятся целиком. |
| **D4 — частично** | [ReleaseUpdater](../src/base/releaseupdater.cpp) и [Qt dialog](../src/gui/releaseupdatedialog.cpp) проверяют и скачивают полный GitHub bundle вручную; 11 HTTPS/Qt fault scenarios и live check passed (9.1). Ошибки отличаются от no update, atomic download сохраняет существующий файл. | Проверить опубликованный новый bundle на окончательной композиции. Подпись требует доверенного ключа; digest не заменяет её. По уточнённому scope updater только скачивает ZIP: installation/profile migration не выполняются и не обещаются. |
| **E1–E3 — частично** | Trusted inbound libtorrent adapter и qbutt-net v5 lease подключены через [PathManager](../src/base/net/pathmanager.cpp). Whole-app TCP/uTP и входящий DHT прошли controlled IPv4/IPv6, включая original endpoint, renewal/reopen/terminal rollover. Independent VPN ingress подтвердил TCP (`gateway-wan-2mGt5m`) и uTP/DHT (`gateway-wan-utp-dht-8bw1KW`), exact 524 288 bytes в каждом прогоне (9.1). | Остались правильные TCP/UDP announces и полная family/fault matrix. Источник через отдельный VPN exit доказывает внешний вход на публичный endpoint, но не отдельный физический third-party host, CGNAT у провайдера или обход действующего system VPN. |

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
