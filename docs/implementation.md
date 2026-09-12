# qbutt: задание для реализации агентами

Читать вместе с `qbutt-architecture.md`. Этот файл является рабочим backlog и контрактом приёмки, а не утверждением, что соответствующие API уже существуют.

Уточнение требований: qbutt архитектурно независим от частного сервиса. Настройки принимают обычную Mihomo-подписку и позволяют выбрать узел; аккаунты, API сервиса и заранее заданные серверы не требуются. Обозначения S1/S2/S3 ниже — примеры независимых выходов для лаборатории. Публичный qbutt-net начинается с закреплённого публичного upstream без истории частных форков; обоснование приведено в `adr/0001-first-slice.md`.

## 1. Цель

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

Оценить WinSparkle для installed build; signed bundle, соответствие версий app/net, ошибка проверки отличается от отсутствия обновления, profile migration rollback. Portable сценарий тестировать отдельно.

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

Сначала T00 → T01 → T02. Затем независимо A1/A2, B1/B2 и D1 на fake data. C2 начинается, когда B2 уже доказывает один корректный path. После C2 параллельно C3/C4, A4 и E1. Автоматическая адаптация C5 появляется после измеримого и корректного ручного Mixed режима.

Не откладывать работающий qbutt до завершения всей дорожной карты: первый самостоятельный артефакт уже должен уметь repair с exact sizes и один явно выбранный сетевой путь. Каждый следующий артефакт расширяет проверенный сквозной сценарий.
