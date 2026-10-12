# qbutt, qplay и qcatch: актуальный план

**Обновлён:** 12 октября 2026. **Текущий этап:** P2–P5 и P7; P6 остаётся ниже полного поиска. **Сейчас:** интеграция общей браузерной сессии, единственного RuTracker и FitGirl; параллельно — native текстовый reader и пять отдельных текстовых провайдеров. Rutube подтвердил фактические 1080p, компоновка вариантов исправлена. В основном Rust-реестре пока семь провайдеров; готовый модуль в worktree ещё не означает поддержку в продукте.

Цель — законченные приложения для поиска, скачивания, просмотра и чтения с понятным интерфейсом и простой поддерживаемой архитектурой. Перенос на Rust/QML должен одновременно устранять лишние слои и доводить реальные пользовательские сценарии. Сам по себе новый стек не считается результатом.

## Как поддерживать этот файл

Это единственный текущий план трёх продуктов. [AGENTS.md](../AGENTS.md) хранит долговременные правила, [контракты контента](agents/content.md) — предметные ограничения, [архитектура](qbutt-architecture.md) — устройство ядра, [tests/README.md](../tests/README.md) — проверенные команды и стенды.

- Перед работой сверять текущий этап, зависимости и незакрытые критерии. После существенного изменения обновлять соответствующую строку, оставшуюся работу и доказательство прямо на месте. Не дописывать хронику прогонов.
- Статусы: **запланировано**, **в работе**, **требует проверки**, **готово**, **внешнее условие**. «Готово» требует всего критерия, точной сборки/ревизии и проверенного результата. Реализовано в исходниках, проверено на fixture и проверено пользователем в установленной программе — разные факты.
- Для завершённого этапа оставлять компактную ссылку на актуальное доказательство: ревизия, среда, сценарий, результат и оставшееся ограничение. Старое доказательство не переносится на заменённый код автоматически. Сырые логи, большие медиа и промежуточные сборки в план не складывать.
- Обратимые улучшения, порядок независимой работы, выбор компонентов и удаление лишнего агент решает самостоятельно. Если найдено более простое решение, обновляет план и применяет его в рамках активного этапа. Существенное изменение архитектурных границ объясняет до реализации. Внешнее условие одного сценария не останавливает независимую работу.

## 1. Исходное состояние

Проверенная исходная ревизия qbutt: `5c172e59f19399e6625627c7a5af765e07aa896f`. Промежуточные версии перед редизайном: qbutt 1.4.0, qplay 1.1.0, qcatch 1.0.0. Их поставка не означает закрытия новой приёмки.

| Область | Что существует сейчас | Что меняем |
| --- | --- | --- |
| qbutt | C++/Qt Widgets, Qt 6.10.1, собственные экраны поверх qBittorrent | Собственный интерфейс проектируем на QML; прикладную логику поиска и контента переносим на Rust |
| Поиск и контент | `components/content/`: Bun/TypeScript, отдельный `qbutt-content.exe`, SQLite, провайдеры, чтение, перевод; `components/search-engine/`: отдельный Go-поиск | Одна внутрипроцессная Rust-подсистема с собственными отобранными провайдерами |
| Браузер и секреты | qcatch, C++ WebView2, PowerShell broker и вызовы PowerShell для DPAPI | Сохраняем необходимые границы браузера; собственный broker/DPAPI вызываем нативно из Rust без PowerShell |
| qplay | Самостоятельный mpv-плеер, C# launcher, Lua-интерфейс и профиль, GPU-функции | Перепроектируем управление; проверяем Rust/QML-оболочку с libmpv и удаляем заменённые слои после приёмки |
| qcatch | Браузерное расширение TypeScript и Rust native host | Сохраняем подходящие браузеру технологии, упрощаем UI, контракты и передачу в qbutt/qplay |
| Торрентное и сетевое ядро | C++ qBittorrent/libtorrent, отдельный qbutt-net, repair/update/policies | Сохраняем работающие владельцы; пересматриваем собственные изменения по необходимости |

Предыдущий план накопил историю проверок. Она остаётся доступна в [закреплённой версии до редизайна](https://github.com/qbutt-org/qbutt/blob/5c172e59f19399e6625627c7a5af765e07aa896f/docs/implementation.md). При работе с неизменённым ядром можно ссылаться на её конкретные доказательства. Текущие приоритеты и незакрытые обязательства определяет этот файл.

## 2. Объём и границы

Обязательно:

1. Перенести собственный поисковик и контентную подсистему на Rust: поиск, оставленные провайдеры, объединение работ, inspect/resolve, библиотеку, историю, прямые загрузки, браузерное взаимодействие, чтение и перевод.
2. Перепроектировать собственные экраны qbutt на QML/Qt Quick. Существующий Widgets-layout не является ограничением нового дизайна.
3. Удалить заменённые TypeScript/Bun и Go runtime, stdio-протоколы, PowerShell-вызовы системных API и дублирующее состояние. Breaking changes допустимы; перенос старых профилей, настроек и форматов не является обязательством.
4. Закрыть пользовательские сценарии разделов 5–6, включая уже заявленные функции. Проверить qplay/qcatch самостоятельно и совместно с qbutt.
5. Довести установку, обновление, документацию, сайты и публичное представление всех трёх продуктов.

Избирательно пересматриваем прежние собственные C++-изменения: настройки, lifecycle, updater, repair, policies и сетевой UI. Не переносим штатную session-модель qBittorrent, очереди, RSS и libtorrent в Rust ради единообразия. Один writer и один torrent engine остаются обязательными.

qplay и qcatch остаются самостоятельными продуктами. Браузерный TypeScript необходим расширению; внешние процессы WebView2, mpv/медиатулзов и qbutt-net оцениваются по реальной функции и изоляции. Цель — убрать искусственное разделение нашей прикладной логики. Отдельный сетевой движок не становится обязательным Rust-rewrite в этой очереди.

### Отбор и качество источников

Целевое покрытие — русские и английские источники, переводы и полезные азиатские оригиналы; ближайшие оригиналы — японские Kakuyomu, Syosetu, AlphaPolis и E★エブリスタ. Нужны фильмы, аниме, игры, программы, манга и ранобэ, включая торренты, обычные сайты и прямые загрузки. Арабские и прочие региональные каталоги без нужного нам покрытия не переносим. Прежнее исключение китайских и платных источников сохраняется; у смешанного сайта рассматривать только проверенное бесплатное покрытие, не обещая платные главы.

Новый набор формируется явным списком допуска: у каждого включённого провайдера наша поддерживаемая Rust-реализация в отдельном модуле и пройденные критерии ниже. Сторонние адаптеры, Cardigann и браузерные расширения используем как референс или источник проверенного кода с сохранением лицензии. Перенос универсального Cardigann engine, всех vendor definitions и их автоматического импорта больше не является целью. Обновляем собственные провайдеры вместе с приложением; отдельный механизм обновления вводим только при доказанной необходимости. Непроверенное внешнее обновление не меняет код, домены, допуск источника или пользовательскую конфигурацию.

Для отбора 12 октября сверены [Megathread](https://www.reddit.com/r/Piracy/wiki/megathread/), его разделы [All Purpose](https://www.reddit.com/r/Piracy/wiki/megathread/all_purpose/), [Games](https://www.reddit.com/r/Piracy/wiki/megathread/games/), [Anime](https://www.reddit.com/r/Piracy/wiki/megathread/anime/), [Books](https://www.reddit.com/r/Piracy/wiki/megathread/books/), [Movies & TV](https://www.reddit.com/r/Piracy/wiki/megathread/movies_and_tv/) и [Software](https://www.reddit.com/r/Piracy/wiki/megathread/software/), а также [Unsafe Sites](https://www.reddit.com/r/Piracy/wiki/megathread/unsafe-sites/). Это основания для нашей очереди и исключений, а не готовая приёмка. Перед реализацией и поставкой сверять актуальную репутацию, канонический домен и происхождение зеркал. Рекомендация каталога не подтверждает безопасность каждого пользовательского файла.

Не включать источники и клоны, отмеченные Unsafe Sites: в частности `thepiratebay.org`, `1377x.to`, `yts.mx`, `watchsomuch.to`, `igg-games.com`, `steamunlocked.net`, `getintopc.com` и `haxnode.net`. Это решение по указанному списку, а не наше заключение о каждом файле. `1337x.to` и `1377x.to` — разные домены; сходство имени не подтверждает зеркало. Остальной vendor corpus также сверить, чтобы небезопасные определения не остались в доступном legacy UI или упаковке.

В новый набор допускаем источник после следующих проверок:

1. Подтверждены домен, нужные языки/категории и полезное покрытие. Живые ограниченные запросы с хоста отличают ответ сайта, challenge, отсутствие входа и сетевую проблему. Устойчиво неработающий источник исключается с конкретным основанием; один timeout, ограничение VM или CAPTCHA таким основанием не являются.
2. Адаптер прошёл review и ручной проход по коду: запросы, разбор, точная идентичность публикации и отображение результата читаются последовательно.
3. Работает полный `search → inspect → resolve → действие` в установленном приложении. Проверены непервый вариант, известные версия/качество/язык, пустой ответ, исчезнувшая публикация, отмена, сбой и точный повтор. HTTP 200, успешный парсер и наличие пункта в Sources отдельно поддержкой не считаются.
4. Авторизация по возможности использует доступную браузерную сессию автоматически; проверка сайта сначала проходит тихо. Когда действительно нужен человек, виден конкретный источник и одно понятное действие с продолжением исходной операции. Проверены истечение сессии, обновление cookies/UA и отсутствие секретов в логах, истории и общих ссылках.
5. Интеграционные/fault-сценарии и реальный pointer-путь подтверждают результат, затем выполнена абляция и повтор затронутых проверок. Для payload-проверок использовать разрешённые контрольные материалы; не запускать неизвестный игровой/программный EXE ради проверки адаптера. В строке покрытия хранить компактное доказательство, ограничения и текущий статус. При изменении сайта или адаптера повторять затронутый путь до новой поставки.

Bun/PowerShell/Python в существующих build/test-инструментах не равны продуктовому backend. Их сокращаем там, где это упрощает поддержку; не переписываем рабочий стенд целиком до первой пользовательской функции. Новые вспомогательные runtime не добавляем по привычке.

## 3. Целевая архитектура и инструменты

```text
qbutt.exe
  QML / Qt Quick: собственные экраны, модели представления, взаимодействия
  Rust: поиск, источники, библиотека, загрузки, браузерные и reader-сервисы
  типизированная граница с Qt и существующим C++
  C++ qBittorrent / libtorrent: session, peer I/O, storage и проверка данных

qplay: самостоятельный плеер на mpv
qcatch: браузерное расширение и Rust native host
WebView2: изолированные проверки источников
qbutt-net: существующий отдельный сетевой процесс
```

Долгие операции выполняются вне GUI-потока. У операции один владелец отмены, состояния и завершения; поздний ответ не меняет другую работу или выбранный вариант. QML получает типизированные модели и изменения, а не копию прежнего JSON-RPC внутри процесса. Парсинг, сеть, SQLite и бизнес-правила не переезжают в QML JavaScript. Отмена, ограничение параллелизма и корректное закрытие нужны и без дочерних backend-процессов.

| Решение | Направление | Проверка перед закреплением |
| --- | --- | --- |
| Qt | Qt 6.12.0, физическая установка SDK в `C:/Tools/Qt/6.12.0/msvc2022_64` | SDK, Rust/QML-компонент, acceptance executable и основное приложение собраны. CLion использует MSVC 2022 preset. SDK-free portable, настоящие install/window/uninstall и ContentView с упакованным runtime прошли в VM |
| Rust и мост | Rust 1.95.0, CXX-Qt 0.10.0, типизированные QObject и QAbstractItemModel | Выбран CXX-Qt после проверки двух мостов: он защищает указатель QObject при enqueue, у Qt Bridge 0.3.0 обнаружен разрыв между проверкой alive и использованием указателя. Контроллер прошёл search/inspect/resolve, отмену с новым запросом, точный magnet и закрытие с активным запросом. Native pointer и штатный торрентный диалог с отменой без payload подтверждены |
| UI-компоненты | Qt Quick Controls и `QtQuick.Templates`; собственные составные компоненты только для повторяющихся задач продукта | Клавиатура, фокус, скролл, состояния, accessibility и реальные длинные списки |
| Тема | В P1 остаются Basic Controls и общая QPalette; пригодность StyleKit для новой композиции уточняется в P2 | Mixed probe Qt 6.12 подтвердил общий Style.qml и живое Light/Dark/System для кнопок и полей. Но дерево и header Widgets остаются светлыми в Dark; `standardPalette()` это не исправляет. Глобально заменять Fusion сейчас не будем. Доказательство: `C:/Tools/qbutt/stylekit-probe/result.json`; native pointer и смена системной темы этим probe не проверялись |
| Встраивание | Один QML shell через `QQuickWidget`, включая chrome области, которую должен закрывать picker | `Popup.Item` и `Overlay` затемняют и блокируют только свою сцену: соседние Widgets остаются доступны. Отдельный Qt 6.12/D3D11 probe подтвердил blur, backdrop/Escape и сохранение выбора/фильтра/scroll. Нужны реальные переключение приложений, IME, DPI и render cost в VM; дополнительный overlay поверх Widgets не вводим |
| Windows API | Rust Windows bindings или существующий нативный слой | DPAPI и private pipes без shell; права доступа, удаление секретов, shutdown |
| Инструменты Qt | Проектные Qt Docs и QML Live Preview MCP, официальные Qt skills | Оба MCP доступны в сессии. Qt Installer установил `aitools.qmllivepreview_msvc2022_64` 1.0.0 рядом с Qt 6.12; handshake и `run_qmlPreview` проверены. Стенд с настоящим ContentView запускается, но hot reload ещё не подтверждён. Видимые проверки выполняем в VM; окно на рабочем столе закрыто. Отдельной preview-сцены нет |
| Проверки | `qmllint`, Rust lint/format, QML Profiler, существующие интеграционные и GUI-стенды | Воспроизводимые команды записать после запуска в `tests/README.md`; сохраняется запрет unit-тестов |

QML debugging включается только в development-сборку. Выбрать минимальный работающий набор инструментов; неподходящий инструмент не оправдывает ещё одну продуктовую прослойку.

## 4. Этапы и текущие статусы

| Этап | Статус | Результат и условие перехода |
| --- | --- | --- |
| P0. План и правила | готово | План и связанные инструкции сверены с текущим кодом и требованиями. `git diff --check` и проверка 30 локальных Markdown-ссылок прошли. Продуктовый код не менялся |
| P1. Интеграция Rust/Qt | готово | Живой search/inspect/resolve, reentrant отмена, точный вариант, штатный торрентный диалог и закрытие прошли. Native pointer, Tab/Home/End и отменённая/пустая выдача подтверждены. Исправленная SDK-free упаковка, настоящая установка и удаление проверены. Startup/idle baseline измерен; бюджеты ниже. MainWindow использует ContentView на `e0fe26f4b`; совместная native-приёмка нового основного окна относится к P2–P5 |
| P2. Дизайн и навигация | в работе | Поиск → работа → точный торрент → штатное добавление, общие контролы и Search/Favorites/History прошли native Light/Dark и визуальную абляцию. Quick reader, picker глав и фильтры вариантов прошли native pointer, blur, фокус и reopen. Browser-action экраны ещё не перенесены |
| P3. Основа Rust-сервисов | в работе | AniLiberty, MangaDex, RuTor, YummyAnime, MangaLib и Nyaa публикуют результаты в typed Qt model; доступны точный торрент, чтение выбранной ветки и Aksor/CVH/Kodik → qplay. Седьмой провайдер Rutube реализован, но живой просмотр выявил неверное выбранное качество; его приёмка открыта. Rust SQLite сохраняет всю известную работу, точный вариант, фильтры, историю, избранное и настройки чтения. MainWindow, Sources и Continuous объединены; единственный владелец при hide/reopen и запись после Exit проверены. Защищённый HTTP/CONNECT transport и Rust browser broker проверены с настоящим mpv. Общая System-приёмка, браузерные сессии провайдеров и оставшиеся consumer открыты |
| P4. Полный поиск | в работе | RuTor подключён к native search/inspect/resolve и Library на `23bc872c5`; категории подтверждаются source routes, exact hash и raw version title сохраняются. Generated HTTPS/fault, живой Software и Qt/SQLite прошли: `C:/Tools/qbutt/rutor-bridge-qa/evidence.json`. Категория на `799129fe4` фильтрует исходную коллекцию Qt proxy без нового поиска; process EN/RU, равное число строк, скрытый выбор и viewport прошли: `C:/Tools/qbutt/category-root-final/evidence.json`. Native RU/Dark и EN/Light при 1280×720 и 1440×900 прошли pointer/popup/focus/empty/long-label проверки; fresh source/render review без findings. Root повторил возврат точной строки/варианта: `C:/VMShared/qbutt-p3-reader-qa/category-final-native.json` и `category-root-personal.json`. Combined handoff на `ac578b364` прошёл для RuTor и непервого варианта AniLiberty: exact hash в штатном диалоге, Cancel оставляет ноль торрентов и пустой payload. Evidence: `C:/Tools/qbutt/rutor-handoff-root/evidence.json`; это synthetic Qt input, загрузка payload и native pointer RuTor ещё не приняты. Объединение публикаций открыто. Условие завершения: все используемые классы источников, S1–S9 и B1–B4 приняты, последний Go search consumer заменён и runtime удалён |
| P5. Чтение и перевод | в работе | MangaDex reader: точная ветка, главы, постраничное чтение, Height/Width и ограниченная предзагрузка реализованы. Native picker, восстановление главы/страницы/scroll после Back, restart и перехода из History приняты. Сохранение Height/Width для профиля, независимая ошибка записи и точный retry проверены в процессе и native RU/EN. Continuous с виртуализированными страницами, ограниченным кэшем и точным anchor объединён с MainWindow; process-сценарии прошли, совместная native-приёмка выполняется. Ранобэ и обратимый перевод ещё не перенесены |
| P6. qplay | в работе | Переход оболочки на Rust/QML подтверждён пользователем; приоритет ниже полного поиска. Renderer feasibility: одна Qt Quick сцена с libmpv/OpenGL декодировала legal Sintel 1080p через NVDEC, с существующим Anime4K shader, QML controls, pause/seek/resume и чистым shutdown. После удаления альтернативной оконной схемы проверка повторена: `C:/Tools/qbutt/qplay-native-foundation-qa/receipt.json`. Это offscreen C++ probe; Rust owner и первая рабочая продуктовая сцена начинаются в изолированном worktree; текущая поставка C#/Lua остаётся до приёмки замены. V1–V5, standalone/IPC/resume/subtitles/HDR/framegen и native GPU-приёмка остаются открытыми |
| P7. qcatch | в работе | На `9194c644795d7ee3d7fafd45c9250cff67658be0` background/session владеет действием и receipt; исправлена идентичность захвата HTTP-страницы. Production integration/fault, typecheck/package и независимое ревью прошли. Native popup close/reopen при реальной отправке, replay и явное «Снова открыть» подтверждены без двойного POST. qcatch → qplay → mpv воспроизводит legal Sintel с выбранными Beta audio/subtitle и растущим временем. Evidence: `C:/Tools/qbutt/qcatch-action-intent-qa/native-qa-9194c64/receipt.json`; ZIP SHA-256 `51325527749639f85813d146c4a0b15e0afe670e427749ded0997d23db1ae96b`. Стенд очищен, прежние процессы/настройки восстановлены. Установка/CTA, actual qbutt accepted capture/download и GUI uncertain/lost-response ещё открыты; ACK принимающего qbutt отсутствует |
| P8. Остальной форк | запланировано | Пересмотр собственного старого UI и лишних слоёв; затронутые torrent/storage/network/profile-регрессии проходят |
| P9. Публичная поставка | запланировано | D1–D4, полный U1–U12 на итоговых установщиках, внешний пользовательский аудит, актуальные сайты/README и релиз |
| P10. Дополнительные платформы qcatch | запланировано | Отдельная матрица раздела 7 после основного поиска, плеера и расширения |

Проверенные границы P1 позволяют продолжать P2–P3. Проверки overlay/IME/DPI/render cost выполняются на первом настоящем modal picker в P2/P5, чтобы принимать продуктовый компонент, а не размножать стенды. Непроверенная граница остаётся условием приёмки зависящего от неё экрана. P2 и небольшой Rust-сценарий уточняют друг друга. P3–P5 выполняются законченными вертикальными срезами: сначала один источник и действие от интерфейса до результата, затем расширение. Не откладывать весь UI и практическую проверку до завершения всего backend. qplay/qcatch можно вести независимо после фиксации их границ, сохраняя одного владельца каждого изменения.

Текущее process-доказательство: `d2d8c954c` с рабочими изменениями, Qt 6.12.0/Rust 1.95.0. `qbutt-quick-acceptance --exercise` завершился с `PASS search_outcomes=ok boundary_keys=ok`, включая reentrant отмену, точный magnet и 46 секунд наблюдения за поздними ответами (`C:/Tools/qbutt/quick-search-outcomes.log`). Rust enum заменил дублирующий QML `searchStarted` и отдельный `busy` state. `metadata-cancel` на `0c5c9f75d` прошёл с исправлением; отрицательный контроль без четырёх строк воспроизвёл `DuplicateTorrent`. Доказательство и hashes обеих сборок: `C:/Tools/qbutt/metadata-control-evidence/result.json`.

Передача `ContentView → AddTorrentManager → диалог добавления` прошла в combined acceptance, SHA-256 `0533f62d3d6d8ef0a107e8e6c90499bf6189edb028d643c240f9fe093eb44a13`: выбран второй вариант AniLiberty, в диалоге тот же hash, ровно одна передача и отмена, session/payload пусты. Доказательство: `C:/Users/cawa1/AppData/Local/Temp/qbutt-quick-handoff-pyGskx/result.json`. Этот стенд синтезирует события Qt; native ввод проверяется отдельно. Runner переиспользует runtime вместо копирования полного bundle и создания новой firewall-записи на каждый запуск.

Native pointer EN/Light и RU/Dark, Tab и Windows-QPA handoff подтверждены в `C:/VMShared/qbutt-p1-quick/qa/native-2ac8349a/native-summary.json`. Исправление nullable external ID `d2d8c954c` проверено запросами «а» и One, прокруткой и повторным точным Frieren handoff: `C:/VMShared/qbutt-p1-quick/qa/native-df86df28/native-summary.json`. На executable `CFC5A31E437E281F5BC3A078C1AFC11D6F69AFCC3A1AC6BAEB70D66028C18513` native Home/End выбирают последнюю/первую работу; успешная пустая выдача показывает свой текст, реальная отмена клавишей Stop — нет. Доказательство: `C:/VMShared/qbutt-p1-baseline-qa/result.json`. Технический overlay probe и отрицательный контроль blur: `C:/Tools/qbutt/overlay-probe/evidence.json`; это не продуктовый UI и не доказательство реального переключения приложений.

SDK-free проверка ContentView выявила отсутствующий `Qt6QuickWidgets.dll`; официальный builder исправлен через `--quickwidgets`. Пересобранная поставка `d2d8c954c + dirty`, Qt 6.12.0, прошла portable startup, install → настоящее окно → Exit → uninstall qbutt/qplay под отдельным standard user. Это поставка до замены основного поискового consumer. Исходные ассоциации VM сохранены, тестовые учётка/профиль/копии удалены. Доказательство и hashes: `C:/VMShared/qbutt-p1-baseline-qa/result.json`.

Основной consumer заменён на `e0fe26f4b` (`80748661d` в отдельном worktree): ContentView использует выбранный профиль и один controller на весь lifetime MainWindow; Exit уничтожает его перед drain Qt pool. Удалены LibraryWidget и неиспользуемая прежняя acceptance-сцена. Локальное чтение, direct downloads через View → Downloads / Ctrl+J, qcatch и AI-настройки плеера сохраняют действующих владельцев. Совместная SDK-free сборка с Sources и Continuous (`4a179605f`, storage fixture `cd926cf4a`) прошла основной torrent handoff/Cancel без payload, hide/reopen во время записи и SQLite после Exit. Найденный startup crash происходил из-за устаревшего `application.cpp.obj` без зависимости от изменённого MainWindow; повторная компиляция восстановила зависимости и нормальный запуск. Evidence: `C:/Users/cawa1/AppData/Local/Temp/qbutt-quick-handoff-IKOXtz/evidence.json`, обычный exe SHA-256 `aa4fde607958d63b307bb4e62a74d379d8b00ceb6ae98b7f1c9812af73d8a573`. Native Light-проход этой сборки выполняется в VM. Палитра исправлена на `ca47815d6` через стандартный SystemPalette; process-проверка сравнила реальные QML background/text с QApplication при Light/Dark/live switching, а не QVariant с неверным типом. Native/System ещё требуют проверки. Прежний поиск не запускается на новом пути, но Bun/Go остаются владельцами других ещё не перенесённых функций.

Sources на `adc707f07` и QML `0f6c1d0d7`–`7b05c16ef` используют один SQLite worker и typed SourceModel: enablement, redacted failure и точный повтор Search/Inspect/Resolve. Background failure показывается при ручном открытии Sources. Process-сценарии проверили restart, отказ записи, FIFO, stale/ABA и reentrant notification; strict lint прошёл. Evidence: `C:/Tools/qbutt/sources-bridge-qa/evidence.json`. Совместная VM-проверка уже подтвердила открытие/закрытие backdrop и запись переключателей в Light; полная native-приёмка и браузерная авторизация остаются открытыми.

Continuous на `3fcbea56f`/`7cfaa8912` использует виртуализированный PageModel, bounded acquisition/cache и точный page-relative offset. Создание delegate во время ввода больше не возвращает прежнюю позицию; дублирующие сбросы состояния движения удалены. No-op клавиши и колесо на границе сохраняют желаемый offset. Root combined build, strict lint и `--reader` прошли, включая PgUp/PgDown, reopen, late decode, drag/release и SQLite. Удаление input guard воспроизвело прежний отказ: `C:/Tools/qbutt/continuous-reader-native-qa/reopen-evidence.json`. Root лично прошёл EN/Light MainWindow при 1280×720: Favorites → Read → Height → PgDown/PgUp/wheel → scrollbar drag → Back/History/Read → Down, затем Width и 1440×900; глава, страница и точный offset сохранены. Evidence: `C:/VMShared/qbutt-main-combined-qa/root-reader-input.json`. Native restart на итоговом исправлении, совместные ошибки, System/IME/DPI и ресурсный бюджет остаются открытыми.

Native media gateway на `164d9b625`/`fb0a24ebd`/`910958734` использует существующий Tokio runtime и защищённый loopback HTTP/CONNECT с проверкой фактических outbound адресов. Реальный mpv декодировал generated legal HLS по HTTP/HTTPS; проверены auth/framing, лимиты, retire/Drop, TLS trust, общий idle watchdog и непрерывный single GET длительностью 67 секунд. Evidence: `C:/Tools/qbutt/native-media-gateway-qa/evidence.json`.

DirectVideo producer `748a039c4`, Player `44173c25a` и основной consumer `068b14096` объединены. Сквозной process-сценарий основного окна реально воспроизвёл точный AniLiberty 480p через packaged qplay/mpv и публичный CDN: до file-loaded история пуста, после ACK сохранены все 86 известных вариантов, точная серия/качество и preferences; stop завершил token один раз и закрыл listener. Root повторил сценарий на совместной SDK-free сборке: `C:/Users/cawa1/AppData/Local/Temp/qbutt-main-watch-qzYqwn/evidence.json`. Producer SQL/FIFO fault/retry: `C:/Tools/qbutt/playback-bridge-qa/evidence.json`. Root native pointer Watch также воспроизвёл AniLiberty 480p, gpu-next, WASAPI и Pause с полной работой/точным вариантом в SQLite: `C:/VMShared/qbutt-main-combined-qa/root-native-watch.json`. Проверка обнаружила видимую кнопку неподдерживаемого Download: video_profile затирал qplay script options. На `395bcf6a8` параметры дополняются, а C++ Player самостоятельно обнаруживает и асинхронно проверяет qplay; Bun mediaRuntime command удалён. 11 runtime/fault cases и настоящий Main Watch без helper прошли в worktree (`qbutt-native-media-runtime-oZNpcq/result.json`, `qbutt-main-watch-P8pOAE/evidence.json` в Temp); совместная сборка `395bcf6a8` и Root Main Watch прошли (`C:/Users/cawa1/AppData/Local/Temp/qbutt-main-watch-WSlkKW/evidence.json`); Root также повторил native History → точный 480p → Watch в VM с физически удалённым helper: gpu-next/WASAPI работают, неподдерживаемой кнопки Download в controls больше нет. Evidence: `C:/VMShared/qbutt-main-combined-qa/root-native-runtime.json`. Native установленный registry path ещё не проверен. Остальные Bun subtitle/download/reader consumers сохраняются до их замены. Прямая загрузка native video требует своего queue owner и пока не предлагается.

На совместном executable `98f8a7dc43490b28c04701fe2a3042af37ff556e92250b531eaf007315fb7518` Root лично проверил native Sources в EN/Light при 1280×720: сохранённый timeout отмечен красным, pointer Retry выполняет настоящий Search и очищает failure в SQLite. Live Light → Dark поменял основное QML-поле и Sources; product frames `C:/VMShared/qbutt-main-combined-qa/frames/root-source-failure-en-light-1280.png` и `root-sources-en-dark-1280.png`. Общая System/OS-проверка ещё открыта. Исправление Continuous и повторная native-проверка описаны выше; весь P5 не закрыт.

Baseline опубликованного `5c172e59f`/Qt 6.10.1: три fresh Light профиля, startup до первого видимого окна 1708/325/321 мс, дерево процессов 209–219 MiB private, idle CPU 0.16–0.65% одного ядра, максимум 3.79 мс на 60 WM_NULL probes. Первый запуск после копирования и два запуска с тёплым OS cache, без перезагрузки; ресурсный профиль снят при 1440×900. Доказательство: `C:/VMShared/qbutt-p1-baseline-qa/baseline-runs.json`. Это не измерение первого результата/reader и не сравнение скорости Rust.

Бюджеты для следующей сопоставимой сборки при тех же условиях: первое видимое окно ≤2 с, два тёплых запуска ≤500 мс каждый; средняя private memory всего дерева пустого приложения ≤250 MiB, idle CPU ≤2% одного ядра, каждый idle WM_NULL probe ≤50 мс без timeout. Это критерии последующей приёмки, а не измерения нового Rust-интерфейса. Первый результат, длинный список и reader измерять на законченных P2/P5 срезах.

Rust-библиотека на `7ef55d168` прошла повторный процессный сценарий: два открытия отдельным процессом, сохранность полных вариантов и фильтров, partial/full metadata, независимость избранного/истории, откат отвергнутой записи и SQLite abort, отказ от повреждённых refs/metadata/schema. Свежий review повторил найденные негативные случаи после исправлений. Это самостоятельное storage-доказательство; Qt-потребитель проверен ниже. Успешный resolve не записывает просмотр.

Общие контролы на `12f1f72fd` с удалённым лишним `verticalPadding` прошли native EN/Light и RU/Dark при 1440×900 и 1280×720: ввод/выделение, штатное меню, Tab, hover/pressed/disabled, Enter, Stop/повтор и точный второй AVC handoff. После удаления высота остаётся 42 px, действие и фокус сохраняются; снимки лично сверены, свежий read-only review не нашёл новых нарушений. Executable `D07DC5870FEF0652B4E00DC3932C3242CF876AB53019E927B1DD8E9770278DCD`, evidence `C:/VMShared/qbutt-p2-controls-qa/summary.json`. IME composition и живое переключение System в продуктовом shell ещё не приняты. Семь переводов штатного editing menu проверены реальным указателем в следующем срезе.

Qt-избранное на `ad438527f` с итоговыми QML/C++ изменениями прошло `all_qmllint`, `--exercise` и `--library --lang ru`: точный End/Enter, восстановление выбора до SQLite completion, nested model notifications, порядок ошибок, поздняя загрузка, initial failure и accepted Loading/Ready write после закрытия QObject. Старые дефекты фактически воспроизведены отрицательными контролями; удалены дублирующие caches и временный scheduling-код. Process evidence: `C:/Tools/qbutt/quick-favorites-final-checks.json`. Закрытие здесь означает уничтожение QObject при живом процессе; оно не доказывает drain при Exit всего приложения.

Та же сборка `BC149120A9B933B0EBB11C151A3B2CA790B57685574120A7FBAC6B86F0AC6979` прошла native EN/Light и RU/Dark при 1440×900 и 1280×720: SQL-подтверждённая красная закладка, вся работа и оба варианта в «Избранном», сохранённые query/results, возврат к точному AVC, удаление/повтор, четыре точных handoff и русское меню. UIA подтверждает selected у нужных work/variant; bookmark остаётся Button с актуальным Add/Remove именем без TogglePattern. Tooltip исправлен штатным Basic-компонентом; Component/import warnings отсутствуют. После удаления полужирного начертания вкладок действия и композиция сохранились. Final кадры лично сверены, свежий source/render review нарушений не нашёл. Evidence: `C:/VMShared/qbutt-p2-favorites-qa/summary.json`. Native-рендер ошибки библиотеки и исчезнувшего сохранённого варианта ещё не проверен; процессный сценарий подтверждает только их состояние.

Текущий MangaDex-срез — `28f12c60a` с рабочей QML-интеграцией, Qt 6.12.0/Rust 1.95.0. Executable `ADDF57E88E5287835AE357F3841A89EFD2281520341BC51E3EC3EE3EA6C105B7`: `--reader` RU/EN подтверждает search → inspect → точную русскую ветку → реальные декодированные страницы, фильтрацию без потери глав, reentrant query, fit, границы страниц, отмену поздней главы и cache failure/retry/cleanup. Регрессия проверяет геометрию Image/scrollbar на 1280×720 и 1440×900, фокус на входе и после fit, Tab и Up/Down с сохранением offset. Ручной расчёт высоты Image и переопределение штатного ScrollBar удалены. `qbt_app`, Quick host и `all_qmllint` собраны без предупреждений; fmt/clippy прошли на том же Rust-коде. Evidence: `C:/Tools/qbutt/native-progress-qml-focus-{reader.stderr,reader-en.stderr,build,app-build}.log`. Предыдущая 6B версия прошла `--reader-progress` и `--worklist` на неизменённом Rust-коде.

Выбор Qt ListView синхронизируется вне binding. Дефект переиспользования делегатов при Search/Favorites воспроизведён при правильных модели и SQLite; `reuseItems` удалён на `553255dd5`. `--worklist` проверяет реальные роли строк, Label, accessibility, selection, click/remove и переключения равных по размеру моделей с другим порядком. Возврат одной удалённой строки снова показывает чужую работу. Source/process evidence: `C:/Tools/qbutt/favorites-model-evidence/summary.json`. Native AD RU/Dark и EN/Light подтвердили правильную работу, всю русскую ветку и варианты в Favorites, добавление/удаление/повтор и сохранность Search/query. Кадры `C:/VMShared/qbutt-p3-reader-qa/r4-ad-{ru,en}-favorites-*.png` лично сверены.

Native RU/Dark и EN/Light при 1440×900 и 1280×720 подтвердили реальные страницы, Arrow/Home/End, fit/scroll, query/selection reopen, Escape/backdrop и сохранность picker при другом активном приложении. Оглавление из 104 глав сохраняет позицию при повторном открытии. Боковые зоны используют paintedWidth: видимые края портретной страницы листают, центр — нет; Width и пустые боковые поля проверены. Evidence: `C:/VMShared/qbutt-p3-reader-qa/ablation-observations.json`, `working-en-light-observations.json`, `painted-fix-en-light-*.png`. Удаление лишнего счётчика и row-контейнера прошло визуальную сверку. Height→Width исправление на `b7fd7c731` прошло native RU/Dark min без прежних binding warnings; кадры Height и Width лично сверены, evidence `height-positive.json`. Qt scale 1.25 и entry/exit picker подтверждены в `scale-motion.json`; это не переход системного DPI и не измерение FPS. IME отсутствует в текущем guest и ещё не проверен.

Очистка кэша принята в isolated Quick host. Qt pool переживает уничтожение QObject; bounded retry Windows 32/33 сохраняет необходимую очистку при кратком decoder lock. Directory owner удерживается до завершения file cleanup на `53e8342ff`, устраняя воспроизведённую гонку Windows error 5. Graceful Exit на `28f12c60a` передаёт существующий Services/Runtime в тот же Qt pool и дожидается принятых SQLite-записей, decode и удаления кэша. Удаление handoff воспроизвело оставшийся payload и потерянную запись; восстановленный код прошёл active QImageReader и короткий/длинный cleanup, включая чистый код после снятия instrumentation. Evidence: `C:/Tools/qbutt/reader-cache-owner-{ablation-negative,ablation-restored,repeat-summary}*` и `reader-exit-*`. Source review подтвердил порядок Qt shutdown. Это не приёмка Exit старого shipping MainWindow.

Точная позиция реализована в Rust/SQLite и подключена к QML. `--reader-progress` подтверждает decoded page acknowledgement, всю известную работу и prefs, duplicate/stale URL, finite bounds, reopen/controller restart, write/queue/read faults, immutable retry после перехода к неподтверждённой странице, повтор того же SQLite owner после startup failure и явный отказ вместо подмены исчезнувшей главы/страницы. Native AD RU/Dark и EN/Light прошёл wheel/правый scrollbar/Up/Down → Back/reopen → graceful Exit/restart и SQL abort → pointer Retry. SQL и UIA подтвердили главу 142, страницу, русскую ветку и scroll; default Height не затирает offset, Width восстанавливает его с допустимым округлением меньше одного пикселя. Tab достигает области чтения; fit и закрытие picker возвращают туда фокус. Быстрый wheel → Back/fit сохраняет изменявшуюся позицию, но свойство Qt `moving` через UIA не проверено. Кадры error/retry/restart лично сверены, свежий source/render reviewer новых нарушений не нашёл. Evidence: `C:/Tools/qbutt/native-progress-failurefix-evidence.json`, `native-progress-qml-root-progress.stderr.log`, `C:/VMShared/qbutt-p3-reader-qa/r4-ad-final.json`.

History на `cb6acc2ae` с QML-интеграцией использует прежний SQLite worker и typed WorkModel. `--history` подтвердил полную работу, точный вариант, другие переводы, порядок открытий, независимое удаление и ошибки SQLite/очереди/reentrant notification. Отказ чтения snapshot не отменяет подтверждённую запись и не перекрывает более новую ошибку. `--worklist` RU/EN на executable `EF6CFAEACAB32EB00F306AD9441121354C1C007DFBD5525586226B43ECEE5C50` прошёл Search/Favorites/History → QML Image.Ready → reopen той же страницы и видимой прокрутки → удаление History с сохранением bookmark/progress. `--library`, `--reader`, `--exercise`, `qbt_app` и `all_qmllint` прошли. Native RU/Dark и EN/Light на обоих размерах подтвердили restart, точный перевод, главу 141/страницу 2/scroll, удаление и повтор после SQL abort. Основной агент сам сменил перевод и вернул русский, прочитал сохранённую страницу, удалил History и проверил полную работу в Favorites. Shared IconButton заменил дублирование закладки; временная диагностика удалена до финальных проверок. Свежий source/render review нарушений не нашёл; binding/component warnings — 0. Evidence: `C:/Tools/qbutt/history-qml-process-final.json`, `C:/VMShared/qbutt-p3-reader-qa/history-ef-final.json`, `history-ef-root-personal.json` и `history-ef-cleanup.json`. Native отказ чтения snapshot не воспроизводился; его состояние проверяет process-сценарий. MainWindow/System/DPI/IME остаются открытыми.

Height/Width на `b80f2030f` с итоговыми QML/fixture изменениями использует прежний SQLite worker и schema 3. Core `--reader-settings` проверил позднюю загрузку, ABA/reentrant intent, отказ очереди, SQL abort, независимость ошибок и accepted write при graceful Exit; отрицательные контроли воспроизвели потерю intent без generation/load guards. Итоговый executable `8B8F3A544148E02687205481BFF7588B121D7446454AACED97886E0A924F0F36` прошёл build, `all_qmllint` и `--reader` RU/EN с QML error/retry. Process evidence: `C:/Tools/qbutt/reader-settings-native-qa/evidence.json` и `reader-settings-ui-process-final.json`.

Native RU/Dark и EN/Light при 1280×720 и 1440×900 подтвердили fit после restart без повторного выбора, сохранение главы 141/страницы 2/scroll, обе стороны SQL abort и точный retry. Успешные History/progress/bookmark операции не скрывают ошибку fit. Основной агент сам нажал Height при отказе записи, повторил его и вернул Width, сверив UIA, SQLite и кадры. Удалён лишний wrapMode; повторные проверки и свежий source/render review прошли. Evidence: `C:/VMShared/qbutt-p3-reader-qa/settings-final.json` и `settings-root-personal.json`. Native Saving-at-Exit не наблюдался; его подтверждает process-сценарий. MainWindow/System/DPI/IME остаются открытыми.

Live RU/EN в MainWindow и Options использует существующие переводчики Application; смена Locale обновляет QML, роли загруженных вариантов и ошибки источников/библиотеки через текущий ContentController без нового запроса или потери выбора. `bun tests/qt-acceptance/language-switch.ts` прошёл четыре запуска с Apply, меню, QML, видимой подписью и accessibility видео, сохранением Locale и restart. Сгенерированные timeout и SQLite abort проверяют уже видимые ошибки Sources и библиотеки через RU→EN→RU. Штатная поставка `windeployqt` с объединённым `qt_ru.qm` прошла без SDK-путей; отдельный `qtbase_ru.qm` ей не нужен. Поочерёдное удаление обновлений моделей и уведомлений Sources/Library воспроизвело старые подписи и ошибки, восстановление прошло. Evidence: `C:/Tools/qbutt/language-switch-qa/evidence.json`. Native pointer и визуальная абляция этого исправления ещё не проверены; переводы ошибок чтения и активного playback этим сценарием не приняты.

Совместная native-приёмка Continuous/MainWindow, ранобэ и перевод остаются открытыми. ReaderMode уже управляет работающим Continuous. Прежние Bun/Go потребители удаляются после их замены и приёмки.

В Rust каждый из шести перенесённых провайдеров имеет отдельный модуль; сетевые запросы отделены от разбора ответа, YummyAnime отдельно содержит Aksor/CVH. Concrete `Catalog` владеет их dispatch, `controller/selection.rs` — выбором работы и варианта. Чтение разделено на `reader/{mod,chapters,viewport,progress}.rs` с одним ReaderState; SQLite schema и profile-wide reader settings вынесены из Library без дополнительного соединения. Удалены дублирующая magnet-валидация и избыточные проверки, побочные эффекты вынесены из сложных условий. Исправлен поздний ACK настроек после ошибки загрузки базы: он больше не отменяет точный retry. Сборка, fmt/clippy, provider fixtures/live-сценарии, Library и Qt reader/progress/settings/worklist прошли; независимое ревью проверило сохранность 53 методов чтения и последнюю абляцию. В основном окне лично проверены MangaLib → глава/страница/масштаб → History → restart с чтением прежней страницы. Evidence: `C:/Tools/qbutt/maintainability-final.json`. Это проверка изменённых content-сервисов и Qt bridge; весь upstream C++ не объявляется проверенным. Прежний TypeScript удаляется вместе с принятыми заменами.

MangaLib/Nyaa на `119a39cea` прошли общую сборку, clippy и Qt-сценарии `--mangalib`, `--nyaa`, `--sources`, `--reader-progress`. В SDK-free основном окне EN/Light без `qbutt-content.exe` лично проверены MangaLib Yotsuba → избранное → глава 1/страница 2/Width/scroll → Exit/restart с той же позицией, picker 113 глав при 1280×720 и Nyaa-only Frieren → конкретная книга → штатный диалог с точным hash → Cancel. Payload остался пустым. Evidence: `C:/VMShared/qbutt-main-combined-qa/root-native-sources.json`, process-логи `C:/Tools/qbutt/main-combined-qa/native-sources-*.log`. MangaLib точная ветка может быть `null`; это идентичность API. Nyaa live Anime и Books проверены, Games/Software покрыты generated fixtures. Native новые источники в RU/Dark, оформление Markdown-описаний Nyaa и полная браузерная поддержка остаются открытыми.

Фильтры вариантов на `83ffe71ff` используют Qt proxy над полной моделью и единственные Rust work preferences. Серия, озвучка, источник/плеер, язык и текстовый поиск сохраняются для каждой работы; скрытый фильтром точный выбор не теряется. Поздние playback/reader ACK записывают подтверждённый контент, сохраняя актуальные фильтры. Build, qmllint, fmt/clippy и process-сценарии filters EN/RU, History, Library, reader progress, playback и worklist прошли. Исправлена отдельная причина недоступности: отключённый источник больше не считается исчезнувшим вариантом.

Основной агент лично проверил группу YummyAnime из 1812 вариантов: серия 12/AniLibria/CVH, пустой поиск и сброс, переход между работами, Exit/restart и полную SQLite-запись. Выбранная серия 12 не подменяет реально просмотренную серию 1. На executable `E2C8D18B03526ACC67C786A31D8F2B50EF4C9D55787E43C0F93C133C06D8D07B` проверены EN/Light и RU/Dark при 1280×720 и 1440×900, live смена языка/темы, закрытие picker по backdrop, сохранение прокрутки после reopen и переключения приложений, выключение/включение источника с правильной подсказкой и действием. Временная диагностика удалена, History/Library повторно прошли; свежий source review не нашёл нарушений. Evidence: `C:/Tools/qbutt/variant-filters-final.json`. IME, DPI transition и стоимость рендера picker остаются открытыми.

Native YummyAnime → qplay принят для серии 1/AniLibria через Aksor и Kodik: pointer pause/seek/resume, закрытие плеера, полный Exit/restart qbutt и продолжение с сохранённой позиции. Aksor на предыдущей сборке: 1920×1080, `start=318.089`, `C:/Tools/qbutt/yummyanime-aksor-final.json`. Kodik на `18af0e3c3`, exe SHA-256 `0964F874BE874DA48F0BA1008B0A9BC4E41780BBCAD96B8A4F1089AA32FFF1E9`: 1280×720, gpu-next/WASAPI, `start=317.025`, `C:/Tools/qbutt/yummyanime-kodik-final.json`. SQLite сохранила все 1812 вариантов, точный выбранный/просмотренный вариант и фильтры; собственные процессы завершились, stdout/stderr пусты. Kodik проверяет свежую серию, озвучку и идентичность после редиректов; соответствующие generated/live проверки прошли. Это приёмка текущего упакованного mpv/qplay, новая Rust/QML-оболочка относится к P6.

Rust browser core и Qt/legacy relay объединены на `94597e886`; `private-pipe.ps1` и его упаковка удалены. Current-user DACL, точная идентичность, отмена, массовая отмена, лимиты, shutdown, безопасные ошибки и гонки необязательных cookies прошли integration: `C:/Tools/qbutt/native-browser-owner-qa/receipt.json`. Root собрал основное приложение и лично проверил в VM fresh профиль с выключенным Search: подключение qcatch через пять секунд → один точный capture → legal Sintel в packaged qplay, 1280×720 H.264/AAC, gpu-next/WASAPI → pointer pause/seek/resume → Exit. Discovery и собственные процессы удалены, Search остался выключенным. Доказательство точной сборки и границ: `C:/Tools/qbutt/native-browser-main-qa.json`. Отдельный production relay сценарий дождался подключения через 31 секунду без ручного retry. Повторены IPC, helper restart/crash, exact capture/download и VK-search fixtures; свежий read-only reviewer не нашёл оставшихся замечаний. В основном окне использован настоящий qcatch native host с generated browser peer; реальный DOM расширения, установка/CTA, receiving ACK и native YummyAnime VK ещё не приняты.

### Источники: актуальное покрытие и очередь

На `2385394c5` реестр `Source::ALL` содержит семь native провайдеров, все включены в новых профилях. Новый поиск и QML Sources используют только этот реестр.

Прежний Bun Catalog объединяет встроенные TypeScript-провайдеры и каталог отдельного Go-поиска. Settings ещё открывает его SourcesDialog: в Go встроены 574 YAML, включая четыре арабских private tracker, выключенных по умолчанию. Это число файлов, а не рабочих интеграций.

Legacy updater при запуске Go-процесса пытается загрузить Jackett `master`, если после последней успешной проверки прошло не менее суток; после ошибки попытка может повторяться при следующем запуске. Он проверяет совместимость схемы/engine и заменяет definitions; allowlist проверенных нами сайтов у этого пути нет. Список default-enabled лишь задаёт начальные переключатели, не фильтрует каталог.

В legacy подтверждены `thepiratebay.org` как основной адрес и `yts.mx` только как старый адрес YTS; текущий основной адрес определения YTS — `yts.gg`, а его ID включён по умолчанию. Это не основание объявлять новый домен проверенным. При ближайшей очистке убрать исключённые определения и доступ к ним из старых настроек, затем удалить весь ненужный loader/updater с последним Go-потребителем. Не переносить сторонний каталог в новый picker и не оставлять автоматическое скачивание `master` в следующей поставке.

| Покрытие / источник | Текущее состояние | Следующий результат |
| --- | --- | --- |
| AniLiberty, YummyAnime | Native поиск и варианты; приняты конкретные direct/Aksor/CVH/Kodik пути, описанные выше | Закрыть внешние плееры и сверить весь оставленный набор с новыми критериями допуска |
| RuTor, Nyaa | Native search/inspect/resolve, категории и точные torrent refs; интеграционные доказательства выше | Дополнить применимые native действия и законченное скачивание; Books здесь означает torrent-категорию, не чтение текста |
| MangaDex, MangaLib | Native главы/страницы, выбор перевода, History/progress/fit; приняты указанные reader-пути | Общая Continuous/System-приёмка и обратимый перевод |
| Rutube | Native провайдер и точный child playlist; фактические 1080p со звуком, pause/seek/resume и History подтверждены. Список вариантов исправлен; доказательство в приоритете 1 | Принять restart/resume из History. Варианты выше 1080p и внешние группы дорожек пока не поддержаны |
| RuTracker | Единственный `.org` native адаптер и session fixtures в отдельном worktree; live tracker/topic требуют проверки сайта | Объединить общую сессию, CP1251 и точный handoff; принять доступную авторизацию без лишнего ввода |
| VK Video, YummyAnime [VK] | Rust broker принят, native consumer внешнего видео и прямой поиск VK ещё отсутствуют | Точный видео-ID, session/UA, дорожки и продолжение действия; прямой поиск проверяется отдельно от resolve известного URL |
| FitGirl | Native модуль проверил реальные search/detail и два точных magnet-варианта Cyberpunk, включая версии; ещё не объединён | Catalog и штатное native добавление без игрового payload. Direct/`.torrent` зеркала не приняты |
| DODI | Реальные search/detail и torrent landing options подтверждены; host rate limit/Turnstile мешают получить `.torrent` | Ограниченный native bytes handoff и реальный resolve; до этого не включать источник |
| Igruha | Текущий канонический домен не подтверждён: старый адрес уводит на неизвестный replacement, кандидат отвечает DDoS-Guard; браузер заблокировал проверку безопасности | Отложен до подтверждения домена и рабочего пути; native адаптер и регистрация не создавались |
| ByRut, FreeTP | На хосте доступны отдельные search/detail/torrent пути; у ByRut часть AJAX требует challenge, FreeTP использует CP1251 | Рассмотреть после первых игровых источников при полезном дополнительном покрытии; отдельно различать полную игру, patch, DLC и fix |
| RanobeLib, RanobeHub, RoyalRoad | По отдельному агенту на native provider; общий text reader/core/UI разрабатываются независимо | Русское и английское чтение, бесплатные главы, длинное оглавление и сохранение позиции; платные/AI-подкаталоги не переносить |
| Kakuyomu, Syosetu, AlphaPolis, E★エブリスタ | Kakuyomu/Syosetu проходят отдельное исследование и реализацию; AlphaPolis/E★エブリスタ остаются кандидатами | Поиск, главы, оригинальный текст и продолжение чтения только после допуска; платное покрытие исключено |
| nhentai | В native реестре отсутствует | Проверить канонический домен и текущий доступ, затем свой адаптер и reader; старый пункт в каталоге не доказывает работу |
| NNM-Club, Rustorka | Кандидаты из Megathread All Purpose; нашей новой приёмки нет | Оценить дополнительное RU-покрытие фильмов/программ после RuTracker, включая необходимость входа |
| AnimeTosho, SubsPlease | Кандидаты из Anime; нашей приёмки нет | Проверить пользу поверх Nyaa, точные релизы и пригодность direct/torrent путей; не добавлять дубли ради числа |
| Internet Archive, Project Gutenberg | Кандидаты из All Purpose/Books; нашей приёмки нет | Рассмотреть обычные страницы и прямые файлы/текст после основных источников |
| LRepacks, DIAKOV | Кандидаты из Software; нашей приёмки нет | Проверить RU-программы, версии, реальные хостинги и происхождение файлов до решения о реализации |

### Текущая работа и оставшиеся задачи по приоритету

1. **Завершить фиксацию принятого Rutube/layout среза.** На сборке с `878a9fe40` и исправленным `WorkPane.qml` лично подтверждены 1920×1080 H.264/AAC, звук, pointer pause/seek/resume, закрытие qplay и появление точного 1080p-варианта в History. Список вариантов видим при 1440×900 и 1280×720; пустой фильтр отключает Watch, сброс возвращает точное выделение. EN/Light и RU/Dark process fixtures, qmllint и свежий review прошли. Evidence: `C:/VMShared/qbutt-main-combined-qa/rutube-fixed-*.png`, JSON playback probes; exe SHA-256 `6C0CBF6C4B7ABC3958100B85A12120709FAC5172C634C8429BFFECC46F4D55EE`. Повторное открытие этого Rutube из History после restart отдельно не подтверждено. Не повторять весь просмотр из-за правок документации или независимого парсера.
2. **Общая native браузерная сессия и RuTracker.** Передать Catalog доступ к существующему BrowserService, получать cookies и UA от одного выбранного connection, сохранять domain/path/secure/expiry в scoped HTTP jar. Добавить bounded CP1251-ответы и перенос существующей тихой WebView2-проверки с одним retry. Затем один RuTracker `.org`: поиск → тема → точный torrent/magnet → штатное добавление, истечение сессии и понятный вход только при необходимости. Не переносить password-форму, подавляющую доступную сессию.
3. **Видео через браузер и VK.** Приватный typed playback descriptor должен сохранять headers/audio/subtitles, не отдавая acquisition URL и секреты в QML. Под существующей qcatch-командой resolve проверить точный VK ID, продолжение выбранного эпизода и обновить pin компонента. Отдельно реализовать/проверить VK-поиск в доступной пользовательской сессии; работа известного VK URL сама по себе его не подтверждает. Завершить оставленные внешние YummyAnime-плееры без нового broker/proxy/runtime.
4. **Игры и программы.** Реализовать Igruha/FitGirl/DODI, начиная с наиболее прямого проверяемого пути; затем отобрать полезные дополнительные сайты из таблицы. Показывать известные версии/издания, автоматически раскрывать варианты при выборе работы, сохранять исходную публикацию. Проверить magnet, `.torrent` и поддерживаемые прямые файлы, не выдавать один рабочий путь за поддержку всех зеркал.
5. **Полноценное ранобэ и оригиналы.** Создать native текстовый reader и перенести выбранные RU/EN-источники, затем Kakuyomu/Syosetu/AlphaPolis/E★エブリスタ и nhentai по очереди допуска. Нужны безопасное отображение текста, оглавление/picker, доступный оригинал, ограниченные кеш/предзагрузка и точное продолжение после restart. Этап заканчивается реальным U5, а не появлением Books в фильтре.
6. **Объединение работ, ранжирование и общие ссылки.** Разделить work identity и исходные PublicationRef, пройти корпус алиасов/сезонов/ремейков и опасных совпадений. Завершить предпочтение языка/качества, устойчивое выделение при новых ответах и один Share всей работы с preferred variant. На втором чистом профиле должны оставаться другие источники и варианты; недоступный точный вариант не подменяется.
7. **Native прямые загрузки и приём qcatch.** Перенести оставшийся DownloadQueue со staging, отменой, прогрессом и проверкой фактического файла/дорожек. В основном qbutt подтвердить принятый capture/download обратно расширению, повтор и lost-response без двойного действия. Установка/регистрация qcatch и автоматическое продолжение выбранной операции требуют настоящего GUI-прохода, не generated peer.
8. **Завершённые Sources и интерфейс поиска.** Принять overlay retry/отказ записи/enablement/restart и убрать доступ к исключённым legacy источникам. Закрыть browser-action экран, System/Light/Dark, живые RU/EN, длинные имена/списки, IME и DPI. Сверить S1–S9/B1–B4; уже принятые category/cache, bookmark, полная History и конкретные playback-пути не переписывать без причины.
9. **Манга и перевод.** Сравнить существующий перевод до редизайна с коротким автономным циклом Codex: перевод, очистка, встраивание, просмотр промежуточного рендера и исправление. Результат полностью автоматический; ручная коррекция не является обязательным шагом. Основной сценарий — уже установленный авторизованный Codex, запасной — настроенный OpenRouter; тяжёлые локальные LLM не закладываем. Koharu и другие проекты исследуются как источники полезных алгоритмов и решений, а не обязательная архитектура. Единый запрос отправлен в ChatGPT, первичные исходники проверяются независимо. Сравнить доступные GPT-6/GPT-6.1 и reasoning на одном разрешённом корпусе: качество, время до читаемой страницы, полное время, число исправлений, расход токенов и фактические cache hits. Держать инструкции, schemas/tools и полезные примеры стабильным префиксом; данные страницы — дальше. Не переносить API cache-настройки на app-server без проверки его интерфейса. Завершить Continuous/MainWindow-приёмку, восстановление при Exit, bounded read-ahead/cache, независимый оригинал и повтор без потери чтения.
10. **Удалить заменённый backend и каталог.** После приёмки соответствующих consumers убрать Go/`qbutt-search.exe`, все ненужные vendor definitions и updater, затем Bun/`qbutt-content.exe`, stdio pending maps/watchdogs, PowerShell DPAPI и прежние собственные Widgets/QSS. До новой поставки исключённый каталог и загрузка Jackett `master` уже не должны быть доступны. Не оставлять универсальный engine или runtime только ради прежней архитектуры; условия удаления — в разделе 8.
11. **Самостоятельный qcatch.** Закрыть C1–C3: реальные direct/HLS/DASH-файлы с дорожками, отмена, несколько frames, expired capture, popup reopen и самостоятельная работа при отсутствии qbutt/qplay. Проверить установку/удаление продуктов в разном порядке; приём qbutt относится к пункту 7, не дублирующей очереди расширения.
12. **qplay на Rust/QML.** После основного поиска заменить C#/Lua-оболочку поверх libmpv одной продуктовой сценой и Rust-владельцем. Feasibility OpenGL/NVDEC/shader/pause/seek уже подтверждена offscreen probe; продуктовая native приёмка ещё впереди. Закрыть standalone/IPC/resume, понятный перевод и drag субтитров, fullscreen/audio, HDR/framegen и применимые GPU-профили без потери работающего воспроизведения.
13. **Остальной собственный форк.** Пересмотреть свои настройки, lifecycle, updater и сетевой UI по необходимости, не переписывая upstream C++ ради стека. Пройти затронутые torrent/storage/repair/update-регрессии, исключительное владение payload и чистый shutdown. Все категории настроек остаются видимыми.
14. **Сквозной QA и публичная поставка.** Пройти U1–U12 на итоговых установщиках в VM, затем дать свежему агенту без знания внутренностей пройти публичный пользовательский путь. Исправить подтверждённые проблемы и выполнить кодовую, визуальную и текстовую абляцию. Обновить RU/EN README/сайты/реальные скриншоты, проверить install/update/uninstall без SDK, закрепить ревизии и выпустить только EXE/нужный portable с последующей проверкой опубликованного.
15. **Дополнительные платформы qcatch.** В последнюю очередь — TikTok, Reddit, Twitch Clips/Live, Instagram, Pinterest, SoundCloud, Яндекс Музыка и VK Музыка по отдельной матрице раздела 7. Cobalt и расширения служат референсами; наличие чужого адаптера не означает поддержку у нас.

Это порядок результатов, а не запрет параллельно закрывать независимый дефект или adapter. Проверки выполняются рядом с каждым изменением; пункт 14 не откладывает QA до конца проекта.

### Параллельная работа

Используем до 20 субагентов `gpt-6.1-sol` с `xhigh`, оставляя свободные места для независимого review и интеграции. Каждый implementation работает в отдельном worktree и сначала исследует существующий native путь, библиотеки и живой источник. Общие контракты получают одного владельца; их небольшие коммиты передаются зависимым веткам. Изменение чужого слоя сначала согласуется с владельцем.

| Владелец | Изолированная работа | Текущее состояние |
| --- | --- | --- |
| Root | Catalog, объединение коммитов, текущий план, общая native приёмка | Принимает готовые browser/provider модули; повторный большой QA — после сборки законченного сценария |
| Browser client | Общий lazy клиент существующего broker | `82d4a4fec` готов к объединению; реальные IPC/lifecycle integration прошли |
| Source session | Cookies/UA выбранного connection, HTTP jar, ограниченные document/metadata ответы | Session fixtures прошли; отдельное расширение для `.torrent` GET/form POST в работе |
| RuTracker | Единственный `.org` provider и CP1251 | Реализация и fault fixtures в работе; живой authenticated путь ещё не принят |
| FitGirl | Собственный provider и точные magnets | `866e8a627` / `f05f73361`: fixture/live прошли, 8 результатов и 2 версии Cyberpunk; Catalog/native действие впереди |
| DODI | Свои metadata и формы проверенных torrent-хостов | Каталог подтверждён; file-me rate limit и Up4Ever Turnstile блокируют live acquisition. До приёмки не включать в продуктовый реестр |
| Torrent acquisition | Ограниченные `.torrent` bytes → штатный libtorrent/AddTorrentManager | In-memory путь без временных файлов и нового writer; реализация в работе |
| Text reader core | Typed original text, кеш, позиция, Qt-модель | Контракт и Transport::Text готовы; lifecycle/SQLite/bridge в работе |
| Text reader UI | Общий reader и виртуализованный текст | Component fixtures/lint прошли; совместная приёмка ждёт core/provider |
| Kakuyomu, Syosetu, RanobeLib, RanobeHub, RoyalRoad | По одному provider на агента | Исследование canonical/live пути, затем только подтверждённая реализация |
| qplay icon | SVG и корректный многоразмерный ICO | Исправление палитры/экспорта и спокойного рисунка, независимо от rewrite плеера |

Агент сначала проверяет запросы и свой модуль, затем готовый затронутый UI в собственном изолированном экземпляре. Управление, способное забирать фокус, выполняется в AppSandbox. Root получает код, короткое доказательство, ограничения и конкретные сигнатуры, объединяет зависимости и проверяет общий пользовательский путь. Не запускать плеер после каждой правки и не повторять неизменённые проверки; существенный дефект требует повторения только затронутого сценария. Промежуточные реальные скриншоты и содержательные результаты показываются пользователю.

AppSandbox Display восстановлен запуском существующих daemon/VM, пользователь подтвердил открытие. Отдельный остаток инфраструктуры: `vm.ts` принимает старый `host.json` за готовую службу, а `display` не обеспечивает её запуск. Исправлять у единственного владельца launcher, без дублирующих обёрток.

### P2: дизайн до массового переноса

- Начать с поиска/карточки работы и reader по референсу MangaLib. Проверять реальное содержание: длинное название, много вариантов/глав, неизвестное качество, загрузку, пустую выдачу и недоступный источник.
- Пересобрать навигацию вокруг действий пользователя. «Поиск», «Избранное» и «История» имеют разные задачи. Для прямых загрузок найти понятное место; не оставлять вкладку, которая только дублирует «Торренты». Существующие функции скачивания не теряются при упрощении навигации.
- Поиск должен сразу показывать найденное; выбранная работа сохраняет контекст при новых ответах. Выбор серии/языка/озвучки не требует разбираться в источниках и служебных статусах.
- Reader даёт место контенту. Выход из чтения отделён от листания. Длинные списки открываются в переиспользуемом picker, настройки режима и fit находятся там, где их ожидают.
- Отдельно спроектировать экран необходимого действия: установка расширения, вход или ручная проверка сайта. Одно ясное основное действие и возврат к исходному контенту после завершения.
- Построить небольшой общий набор: typography/spacing/tokens, actions, списки, search fields, popup/picker, loading/empty/action-required states. Повторно использовать поведение Qt; не писать свои focus/scroll/modal primitives без причины.
- Передавать смысл расположением, знакомыми иконками, выделением и обратной связью. Удалять подписи и пояснения, которые лишь проговаривают уже понятное действие. Не прятать неоднозначность за безымянной иконкой: проверять узнаваемость в пользовательском проходе, сохранять доступные имена и необходимый короткий текст.
- HTML/CSS-прототип допустим как временный инструмент для быстрой проверки композиции. Финальную реализацию и её ввод/анимации проверять в QML. Выполнить визуальную и текстовую абляцию до размножения паттерна по экранам.

## 5. Обязательные продуктовые критерии

Строки ниже — контракт новой реализации. Их статус наследуется от этапа, пока нет актуального доказательства. Наличие похожей функции в старом релизе не отмечает строку готовой.

### Поиск и работа с вариантами

| ID | Готово, когда |
| --- | --- |
| S1 | Результаты и группы появляются по мере ответа источников; виден ход поиска и доступна отмена. Медленный источник не удерживает всю выдачу. Следующий запрос не получает поздние результаты предыдущего |
| S2 | Категория фильтрует уже накопленные и новые результаты во время поиска. Переключение обратно возвращает их без очистки и повторного запроса |
| S3 | Алиасы, русский/английский/ромадзи, варианты пунктуации и release-хвосты объединяются при достаточном основании. Манга и аниме, ремейки, явные сезоны и неоднозначные работы не схлопываются ошибочно. Корпус содержит и пропущенные объединения, и опасные похожие названия |
| S4 | Доступны серия/сезон, озвучка, язык и источник; заранее показаны известные разрешение, размер и версия игры/программы. Неизвестное не выдумывается. Исходные значения сохраняются для точного выбора |
| S5 | Ранжирование учитывает язык пользователя и известное качество. Сиды сравнивают торренты между собой и не штрафуют видеопубликации. Новые ответы не сбивают выделение и не делают список неудобным из-за постоянных перестановок |
| S6 | Карточка работы показывает источники вместо бессодержательного счётчика версий; длинный список сокращается после нескольких названий. В таблице первым слева стоит «Вариант» с иконкой конкретного типа действия, затем «Источник», размер и сиды по применимости |
| S7 | При выборе работы реальные варианты подгружаются автоматически с ограниченной параллельностью и видимым прогрессом. Нет неработающего действия «Показать варианты» и лишнего ручного раскрытия каждой публикации. Выбранный точный вариант переживает обновление метаданных |
| S8 | Закладка рядом с названием становится красной в «Избранном». История хранит всю известную работу, последний вариант и фильтры; можно выбрать другие варианты. Одна кнопка «Поделиться» передаёт работу и при наличии предпочитаемый вариант, сохраняя доступ к остальным |
| S9 | Сохранённая или общая ссылка открывает ту же публикацию после перезапуска. Исчезнувший вариант не заменяется молча другим. Просмотр/скачивание начинается один раз; отказ запуска не создаёт ложную запись успешного просмотра |

Для игр сохраняются относящиеся к выбранной публикации версия, coop/издание, инструкции и комментарии. Для видео подтверждённые рекламные варианты можно скрывать по существующему контракту, но неизвестное состояние не выдаётся за проверенное отсутствие рекламы. Длительность сама по себе не доказывает, что перед нами трейлер.

Допуск источников и актуальная очередь определены в разделах 2 и 4. Для чтения проверить русские переводы и доступные оригиналы; для каждого нового адаптера пройти применимые критерии этого раздела.

### Источники, браузер и ошибки

| ID | Готово, когда |
| --- | --- |
| B1 | Сбои конкретных источников запоминаются без технического шума в поисковой выдаче. Вручную открытые «Источники» показывают красную отметку и понятное действие. Повтор выполняет именно неудавшуюся операцию; её успех снимает соответствующую ошибку |
| B2 | Страница browser challenge не заменяет название и метаданные работы. Источник не исчезает навсегда из-за временной проверки. Штатная проверка в собственной браузерной сессии сначала выполняется автоматически; если нужна личная проверка, пользователь получает ясный переход и затем продолжает исходное действие. Окна браузера не размножаются |
| B3 | Когда действительно нужен qcatch, открывается заметный экран с краткой причиной и установкой. После установки/входа продолжается точное выбранное действие. Cookie/UA получены через разрешённый браузерный механизм, привязаны к выбранному браузеру/сайту и актуализируются; нет чтения чужих баз профиля |
| B4 | Если оставленный источник требует настроенной авторизации, она явно задаётся в «Источниках»; её удаление блокирует также сохранённые и общие публикации. Защищённые URL и cookies не попадают в публичные ссылки, историю диагностики и логи |

RuTracker должен иметь один пункт в «Источниках» и быть включён по умолчанию. Сначала использовать доступную сессию выбранного браузера; ручной вход нужен только при отсутствии действующей сессии и невозможности публичного поиска. Проверить настоящий search → выбранная раздача → штатное добавление, обновление/истечение сессии и продолжение точного действия после входа. Не переносить два прежних адаптера в интерфейс и не считать сохранённый username/password доказательством рабочей авторизации. Если требуется личная проверка, показать конкретный источник и ясный переход с последующим retry; техническая ошибка без адреса не проходит приёмку.

Проверить доступность VK-публикации и через каталог, и через сам источник, сохраняя идентичность видео и фактическую сессию. Не объявлять обязательным ручной ввод токена, пока не исследованы существующий native host и разрешённые механизмы браузерной сессии. Публичная ссылка остаётся ссылкой на публикацию, а не краткоживущим media URL.

Сетевые сбои исследовать с указанием домена и реального маршрута; при необходимости сравнивать прямое подключение и VPN без вмешательства в текущий рабочий сеанс. Обычное исполнение проверки сайтом и показ пользователю допустимы; автоматическое решение CAPTCHA не является требованием.

### Манга и ранобэ

| ID | Готово, когда |
| --- | --- |
| R1 | Конкретная манга и ранобэ находятся, открывают оглавление и читаются без блуждания по служебным диалогам. Проверить ранее проблемный запрос `Ore No Kokan Wa Bishoujo Datta No Ka` и его русский вариант; диагностировать причину отсутствия результата, не маскировать её бесконечным поиском |
| R2 | Режим чтения и fit заметны и работают: страницы/непрерывное чтение, высота/ширина, боковые клики. Новый профиль имеет согласованный удобный режим, явный выбор пользователя сохраняется; изображение использует доступное место без необоснованных полей |
| R3 | Picker глав с поиском находится внутри окна, затемняет и размывает фон. Фон и Escape закрывают его, уход в другое приложение — нет. Повторное открытие сохраняет поиск, позицию и выделение; большое оглавление остаётся отзывчивым |
| R4 | После закрытия, перезапуска и перехода из истории восстанавливаются глава, страница и положение чтения. Предзагрузка следует фактической позиции, ограничена и отменяет устаревшую работу |
| R5 | Перевод, OCR, словарь и исправления образуют обратимый слой. Оригинал доступен немедленно, ошибки перевода не ломают чтение, кеш не требует повторных AI-запросов. Ключи/аккаунты настраиваются понятно; автоматический режим использует доступный авторизованный сервис, не обещает его наличие на чистой машине |

В P5 отдельно подобрать модель и уровень reasoning для перевода манги. На одинаковом небольшом корпусе страниц с репликами, контекстными неоднозначностями, вертикальным текстом и надписями сравнить поддерживаемые провайдером модели и уровни. Оценивать точность смысла, естественность русского/английского, порядок реплик, потерянный текст, вёрстку, задержку и стоимость. Больший reasoning не считать автоматически лучшим. Выбрать практический default по результату; ручной выбор модели/reasoning сделать компактным, показывая только применимые параметры. Кеш должен учитывать влияющие на перевод настройки, чтобы смена модели не выдавала старый результат за новый. Здесь же поддерживать актуальный итог сравнения, без отдельного дневника прогонов.

Проверить существующее покрытие MangaLib/MangaDex и источников оригиналов, а также работоспособность ранее добавленных источников взрослого контента, включая nhentai. Наличие их в каталоге не требует использовать откровенные материалы в публичных скриншотах и демонстрациях.

### qplay

| ID | Готово, когда |
| --- | --- |
| V1 | Плеер устанавливается одним EXE и работает без qbutt, клонирования репозитория и shell-команд. Все runtime DLL присутствуют; install/update не ломают совместную установку |
| V2 | Основные действия доступны мышью и клавиатурой; меню дорожек, субтитров, качества и настроек понятно на русском и английском. UI соответствует остальным продуктам, не перекрывает контент без необходимости |
| V3 | Перевод субтитров легко найти и включить; видны применимые условия его доступности. Оригинальная дорожка, timing и оформление сохраняются. Субтитры перетаскиваются мышью и возвращаются в автоматическую позицию; кеш переживает seek/reopen |
| V4 | Локальный файл, прямое видео и торрент до окончания загрузки воспроизводятся с правильными дорожками; позиция возобновляется. Торрентный HTTP Range использует только проверенные libtorrent bytes, а временные priorities/deadlines освобождаются после просмотра |
| V5 | GPU-профили, frame generation и существующее качество воспроизведения сохранены либо заменены доказанно рабочим эквивалентом. Нет регрессии звука, seek, fullscreen, кадрового времени или shutdown |

В P6 проверить самостоятельную Rust/QML-оболочку поверх libmpv на реальном видео. При успешной проверке заменить launcher и UI, удалив устаревшие потребители. mpv остаётся медиадвижком. Не оставлять две конкурирующие оболочки «на всякий случай».

Перевод в прежнем плеере зависел от сессии qbutt. Новый standalone-сценарий должен давать понятный доступ к переводчику при наличии настроенного провайдера. Выбрать общего владельца этой логики по фактическим потребителям; не поднимать новый локальный сервис ради общего кода и не дублировать всю content-подсистему в плеере.

### qcatch

| ID | Готово, когда |
| --- | --- |
| C1 | Popup быстро объясняет найденное на текущей странице и основное действие; длинный список, пустое состояние, несколько frame/дорожек и неизвестный размер не превращаются в набор карточек и служебного текста |
| C2 | Расширение и native host самостоятельно скачивают поддерживаемые direct/HLS/DASH-медиа с правильными audio/subtitles. Проверка включает фактический итоговый файл, звук, seek и отмену, а не только создание задачи |
| C3 | При наличии qbutt/qplay можно передать точный выбранный media/session. Повтор установки или handoff не дублирует загрузки; устаревший capture требует повторного выбора. Удаление одного продукта не ломает остальные |

### Установка и публичное представление

| ID | Готово, когда |
| --- | --- |
| D1 | Все продукты имеют один понятный установщик, независимые настройки, рабочие update/uninstall. qbutt ставит qplay вместе с собой, qcatch — по действию в UI. Более новая совместимая установка не понижается. Пользовательские release assets — EXE и при необходимости portable ZIP |
| D2 | README каждого проекта на RU/EN: переключатель, центрированный верхний блок, стандартные бейджи и заметная ссылка скачивания как в QuiTwin/edge-revanced. Тексты описывают реально доступное, без нарисованных псевдокнопок и рекламных лозунгов |
| D3 | Одностраничные GitHub-сайты определяют язык браузера при первом визите и сохраняют явный выбор. Скачивание ведёт на актуальный установщик; проверены опубликованные страницы, ссылки, переходы между продуктами и первый запуск |
| D4 | Скриншоты показывают реальные полезные состояния окон продукта в тёмной теме. Чужих окон, desktop/taskbar и системных водяных знаков нет. Весь публичный текст прошёл стилистическую проверку и отдельную абляцию |

## 6. Сквозная приёмка глазами пользователя

Все сценарии пока **требуют проверки на новой реализации**. Для каждого записывать последнюю проверенную ревизию, исходное состояние, реальный путь, затруднения и итог; не накапливать повторные описания одинаковых прогонов.

| ID | Начало и задача | Что проверять |
| --- | --- | --- |
| U1 | Новый посетитель README/сайта хочет понять продукт и попробовать | Может ли быстро понять пользу, найти скачивание и получить работающую программу за несколько очевидных действий без знания внутреннего устройства |
| U2 | Найти конкретный фильм | Поиск по русскому/оригинальному названию, правильная работа, язык/качество, просмотр и скачивание, возврат без потери контекста |
| U3 | Найти аниме, определённую серию и перевод | Объединение источников, отделение похожих произведений, фильтры, прямой VK/каталог, torrent/video, resume и субтитры |
| U4 | Найти мангу и продолжить чтение | Понятный режим, fit, глава, оригинал/перевод, picker, перезапуск в середине страницы |
| U5 | Найти ранобэ | Оглавление, длинный текст, перевод/оригинал, сохранение позиции, отсутствие зависания на недоступной главе |
| U6 | Найти игру или программу | Версия, издание/coop, реальные варианты без ручного раскрытия, инструкции, нужный torrent/файл |
| U7 | Избранное, история и общая ссылка | Другая озвучка из истории, фильтры, preferred variant и другие источники на втором чистом профиле; приватные данные не передаются |
| U8 | Источник не отвечает или требует проверку | Остальные результаты доступны, нет служебного мусора; ошибка находится в «Источниках», исправляется и не остаётся ложной отметкой |
| U9 | Для выбранного видео нужен браузер | Ясная установка qcatch/вход, автоматическое продолжение точного действия, минимум ручных шагов, нет пачки новых Edge-окон |
| U10 | Установить только qplay | Локальное и сетевое видео, дорожки, перевод при настроенном AI, drag субтитров, fullscreen, прогресс и повторный запуск |
| U11 | Установить только qcatch, затем другие продукты | Самостоятельная загрузка; появление интеграции без переустановочного квеста; правильный tab/frame и отсутствие двойных очередей |
| U12 | Обновить и удалить продукты в разном порядке | Независимые продукты и текущие межпродуктовые контракты работают, payload и посторонние данные не затронуты; новая сборка не требует dev-runtime. Перенос старого профиля не является критерием |

Пройти путь основным агентом с фактическим вводом и визуальной сверкой. Затем дать свежему агенту без знаний внутренней реализации только публичную точку входа и задачу пользователя. Он должен самостоятельно найти путь, отметить каждый неочевидный шаг и несоответствие обещаний. Отдельному reviewer доступны brief, конечные исходники/diff и проверенные renders для поиска нарушений требований. Проверить находки самостоятельно, исправить и повторить затронутые пути. «Ревьюер не нашёл замечаний» не заменяет собственную проверку.

## 7. Дополнительные платформы qcatch: после основного пути

| Платформа | Требуемое действие | Статус |
| --- | --- | --- |
| TikTok | Скачать выбранный ролик одним действием | требует проверки покрытия |
| Reddit | Скачать GIF/видео с доступным звуком | требует проверки покрытия |
| Twitch Clips | Скачать клип | требует проверки покрытия |
| Twitch Live | Открыть текущий эфир в qplay | требует проверки покрытия |
| Instagram | Скачать выбранное поддерживаемое медиа | требует проверки покрытия |
| Pinterest | Скачать выбранное поддерживаемое медиа | требует проверки покрытия |
| SoundCloud | Скачать доступную аудиозапись | требует проверки покрытия |
| Яндекс Музыка | Проверить доступные способы работы с записью в пользовательской сессии | требует исследования |
| VK Музыка | Проверить доступные способы работы с записью в пользовательской сессии | требует исследования |

Сначала проверить текущий qcatch и закреплённые инструменты. [cobalt](https://github.com/imputnet/cobalt) и подходящие расширения — источники опыта, не доказательство поддержки у нас. Предпочитать уже используемый механизм; не зависеть от случайного публичного instance. Сохранить авторизацию, точный media ID, дорожки, длительность и корректный итоговый файл. DRM-обход не входит в план. Доступность каждой платформы подтверждать отдельно, оставляя реальные ограничения видимыми.

## 8. Форматы данных и удаление прежних слоёв

Спроектировать простые форматы новой системы. Разрешены несовместимые изменения схем, settings, ссылок и внутренних API. Миграция старых профилей/истории/настроек необязательна; не тратить этап на legacy readers, преобразователи и двойное хранение без конкретной пользы. Согласовать только реально используемые контракты текущих qbutt/qplay/qcatch. Сохранение между запусками новой версии и её корректность при сбое остаются обязательными. Breaking change не означает удаление torrent payload или посторонних пользовательских данных.

| Удаляемый слой | Условие удаления | Статус |
| --- | --- | --- |
| `qbutt-content.exe` и Bun product runtime | Все оставшиеся consumers переведены на Rust и текущие пользовательские сценарии работают | запланировано |
| `qbutt-search.exe`, Go indexer runtime, Cardigann loader/updater и массовый vendor | Отобранные пользовательские пути работают через собственные Rust-провайдеры; оставшихся Go consumers нет. Сохранение универсального каталога/автообновления definitions не требуется | запланировано |
| PowerShell private-pipe broker | Нативный Rust-владелец принят вместе с прежними consumers, isolation/cancel/shutdown | готово на `94597e886`; основной capture и integration проверены, evidence выше |
| PowerShell DPAPI | Защита настроек использует нативный API в новом владельце авторизации | запланировано |
| Внутренние stdio/JSON pending maps, watchdogs и launcher glue | Их единственные процессы-потребители удалены; необходимая отмена остаётся у Rust-операции | запланировано |
| Старые собственные Widgets-экраны и их QSS/handlers | Соответствующий QML-сценарий принят, новые и старые UI не живут параллельно без причины | запланировано |
| Дубли discovery/runtime/profile в трёх продуктах | У каждой ответственности один владелец и проверенная интеграция | запланировано |
| Устаревшие сборочные/QA drivers и generated payloads | Новые команды проверены, необходимые доказательства сохранены, файловые цели очистки проверены | запланировано |

После каждого среза провести code/UI/text/instruction ablation по правилам проекта. Удалять реально, затем повторять затронутые проверки. Не сохранять старую активную реализацию в fallback-ветке только из опасения удалить код; история Git сохраняет её для изучения.

## 9. Прежние функции форка и внешние условия

Проверенные контракты [архитектуры](qbutt-architecture.md) сохраняются при редизайне. Для изменённых владельцев повторить применимые сценарии из [tests/README.md](../tests/README.md):

- Обычные torrent add/resume/recheck, выбранные файлы, priorities, категории, очереди и проверенные hashes/sizes.
- Read-only repair без записи до согласия, exclusive ownership, корректные mappings и сохранение unknown/ignored файлов.
- Staging/journal/recovery, отсутствие writable hardlinks и скрытого in-place fallback; completion только после commit/verification.
- Один libtorrent session/writer, path generation, private scope и полный заявленный сетевой контур; не выводить Tunnels only из TCP smoke.
- Tray/Exit/startup, обновление, magnet associations, отдельный профиль, Desktop по умолчанию только для новых профилей.

Унаследованные закрытые проверки ядра сохраняются лишь для неизменённых владельцев. Публичные заявления о производительности опираются на сопоставимые измерения и verified bytes, а не wire/relay counters.

| Незакрытое внешнее условие | Статус | Следующее действие при доступности |
| --- | --- | --- |
| Реальные системные VPN off/on/restart, default-route/sleep/interface transitions на пользовательском хосте | внешнее условие: нужен независимый канал и разрешённое окно вмешательства | Выполнить оставшуюся B3/B5 матрицу прежней сетевой приёмки, сохранив физический packet evidence |
| Подтверждённая ISP CGNAT-топология и управляемый внешний peer | внешнее условие: топология не доказана | Проверить TCP/UDP inbound, endpoint identity и lease lifecycle; публичный IPv4/IPv6 test не выдавать за CGNAT |

Эти строки не исчезают при выпуске редизайна и не превращают его в заявление о полной сетевой приёмке. Они также не возвращают старую сетевую очередь на место текущего приоритета поиска и UX.

## 10. QA и готовность поставки

Основной desktop baseline — 1440×900, минимальный — 1280×720. Проверять длинные строки RU/EN, большие списки, relevant DPI, Light/Dark/System и live смену темы. Мелкие искусственные размеры окна не порождают дополнительные layout/state-ветки. Продуктовые снимки — тёмная тема; функциональные профили и режим appearance определены в AGENTS.md.

К каждому изменённому сценарию относятся loading/empty/error/cancel/retry/reopen, реальный pointer/keyboard ввод и возврат в исходное состояние. Инспекция кода, offscreen render или успешный build не заменяют установленное приложение. Плеер проверять на обычном легальном видео со звуком и субтитрами; цветовые таблицы использовать только для конкретной технической проверки, не вместо пользовательского пути.

Измерять время запуска/первого полезного результата, UI stalls, память/CPU на поиске и больших списках, ограничение кешей, дерево процессов и shutdown. Численные бюджеты задаются в P1 по исходным измерениям. Ни отсутствие зависания на одной машине, ни Rust сами по себе не доказывают улучшение.

Использовать интеграционные, end-to-end и fault-сценарии без unit-тестов. Повторять проверки после исправлений и при новой неопределённости; не гонять неизменный corpus вместо устранения пользовательской проблемы. Безопасность данных и корректность media/payload важнее скорости.

P9 готов, когда:

- Применимые критерии S/B/R/V/C/D и U1–U12 выполнены на итоговом составе; известные дефекты устранены либо честно выделены как незакрытая работа. Не называть промежуточную поставку полной приёмкой.
- Изменённые владельцы прошли регрессию; избыточные runtime и старые реализации из раздела 8 удалены. Сборка не содержит test/debug overrides или личных профилей.
- Install/upgrade/uninstall проверены в чистой VM, qplay/qcatch проверены без qbutt и вместе с ним. Обязательной миграции старых профилей нет; downloaded payload и посторонние файлы не повреждаются.
- Выполнены собственная визуальная сверка, независимый пользовательский проход, review и абляция. README/сайты/скриншоты соответствуют фактической поставке.
- Компоненты закреплены по проверенным ревизиям. Сборка локальная; workflows остаются ручными. Обычные assets содержат только установщик и при необходимости portable, обязательные сторонние исходники доступны отдельно.
- После публикации проверены реальные установщики и ссылки на GitHub/Pages. P10 и внешние условия раздела 9 сохраняют собственные статусы.

## 11. Технические и визуальные опоры

Проверять актуальность и совместимость перед закреплением зависимостей. Ссылки помогают выполнить этап, но не доказывают работу нашей сборки.

- [Qt Bridge for Rust 0.3: совместимость и статус](https://www.qt.io/blog/qt-bridge-for-rust-0.3-cxx-qt-compatibility-and-soundness), [CXX-Qt](https://kdab.github.io/cxx-qt/book/).
- [Qt Quick Controls](https://doc.qt.io/qt-6/qtquickcontrols-index.html), [Templates](https://doc.qt.io/qt-6/qtquick-templates-qmlmodule.html), [настройка контролов](https://doc.qt.io/qt-6/qtquickcontrols-customize.html).
- [Qt Labs StyleKit](https://doc.qt.io/qt-6/qtlabsstylekit-index.html), [QStyleKitStyle](https://doc.qt.io/qt-6/qstylekitstyle.html), [ограничения QQuickWidget](https://doc.qt.io/qt-6/qquickwidget.html).
- [Официальные Qt agent skills и Documentation MCP](https://github.com/TheQtCompanyRnD/agent-skills), [QML Live Preview MCP](https://www.qt.io/blog/introducing-the-qml-live-preview-mcp-tool-for-agentic-development-of-embedded-devices), [libmpv](https://github.com/mpv-player/mpv/blob/master/DOCS/client-api.md).
- [QuiTwin](https://github.com/AriesAlex/QuiTwin) и [edge-revanced](https://github.com/AriesAlex/edge-revanced): структура публичной страницы, языки, центрированный верх и скачивание.
- MangaLib: практический референс чтения. Telegram Desktop: ориентир качества нативного взаимодействия; не обязательный визуальный шаблон и не новый framework.
