// SPDX-License-Identifier: GPL-2.0-or-later
#include "content-library.h"

#include <array>
#include <functional>
#include <stdexcept>

#include <QCheckBox>
#include <QComboBox>
#include <QCoreApplication>
#include <QDir>
#include <QElapsedTimer>
#include <QEvent>
#include <QEventLoop>
#include <QFile>
#include <QFileInfo>
#include <QHash>
#include <QHeaderView>
#include <QIcon>
#include <QImage>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLabel>
#include <QLayout>
#include <QLineEdit>
#include <QListWidget>
#include <QProgressDialog>
#include <QProgressBar>
#include <QPushButton>
#include <QScrollArea>
#include <QScrollBar>
#include <QSet>
#include <QTabBar>
#include <QTableView>
#include <QTabWidget>
#include <QTextBrowser>
#include <QThread>
#include <QToolButton>

#include "base/global.h"
#include "base/content/client.h"
#include "base/media/player.h"
#include "base/preferences.h"
#include "gui/content/librarywidget.h"
#include "gui/content/readerwidget.h"
#include "gui/content/sourcesdialog.h"
#include "gui/mainwindow.h"
#include "gui/uithememanager.h"

namespace
{
    enum VariantColumns
    {
        VariantColumn,
        SourceColumn,
        SizeColumn,
        SeedsColumn
    };

    void require(const bool condition, const QString &message)
    {
        if (!condition)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const QString &label, const std::function<bool()> &ready)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!ready() && (elapsed.elapsed() < 10000))
        {
            QCoreApplication::processEvents(QEventLoop::AllEvents, 20);
            QThread::msleep(1);
        }
        require(ready(), u"Timed out: %1"_s.arg(label));
    }

    QJsonObject variantAt(const QTableView *table, const int row)
    {
        return table->model()->index(row, VariantColumn).data(Qt::UserRole).toJsonObject();
    }

    int variantRow(const QTableView *table, const QString &source, const bool needsInspection)
    {
        for (int row = 0; row < table->model()->rowCount(); ++row)
        {
            const QJsonObject variant = variantAt(table, row);
            if ((variant.value(u"sourceId"_s).toString() == source)
                && (variant.value(u"needsInspection"_s).toBool() == needsInspection))
            {
                return row;
            }
        }
        return -1;
    }

    QJsonArray fixtureEvents(const QString &path)
    {
        QFile file(path);
        if (!file.open(QIODevice::ReadOnly))
            return {};

        QJsonArray events;
        for (const QByteArray &line : file.readAll().split('\n'))
        {
            const QJsonDocument document = QJsonDocument::fromJson(line);
            if (document.isObject())
                events.append(document.object());
        }
        return events;
    }

    bool hasFixtureEvent(const QString &path, const QString &event, const QString &source)
    {
        for (const QJsonValue &value : fixtureEvents(path))
        {
            const QJsonObject object = value.toObject();
            if ((object.value(u"event"_s).toString() == event)
                && (object.value(u"sourceId"_s).toString() == source))
            {
                return true;
            }
        }
        return false;
    }

    void exerciseContentLocalization(MainWindow *window, const QJsonObject &spec, QJsonObject &evidence)
    {
        const QString locale = spec.value(u"locale"_s).toString();
        require(((locale == u"en") || (locale == u"ru")) && (Preferences::instance()->getLocale() == locale),
            u"Content localization did not use the requested profile language"_s);
        require((Content::LibraryWidget::tr("Favorites") == ((locale == u"ru") ? u"Избранное" : u"Favorites"))
            && (Content::LibraryWidget::tr("Add to favorites") == ((locale == u"ru") ? u"Добавить в избранное" : u"Add to favorites"))
            && (Content::LibraryWidget::tr("Remove from favorites") == ((locale == u"ru") ? u"Убрать из избранного" : u"Remove from favorites")),
            u"Favorites actions are not localized in the requested language"_s);
        struct Failure
        {
            QString code;
            QString english;
            QString russian;
        };
        const std::array failures {
            Failure {u"webview_unavailable"_s, u"The source check is unavailable. Reinstall qbutt."_s,
                u"Проверка источника недоступна. Переустановите qbutt."_s},
            Failure {u"webview_runtime_required"_s, u"Install Microsoft Edge WebView2 Runtime to check this source."_s,
                u"Для проверки источника установите Microsoft Edge WebView2 Runtime."_s},
            Failure {u"webview_timeout"_s, u"The source check took too long. Try again."_s,
                u"Проверка источника заняла слишком много времени. Повторите попытку."_s},
            Failure {u"webview_failed"_s, u"Could not check the source. Try again."_s,
                u"Не удалось проверить источник. Повторите попытку."_s},
            Failure {u"browser_check_required"_s, u"This source requires a browser check."_s,
                u"Источник требует проверки в браузере."_s},
            Failure {u"source_session_invalid"_s, u"Check the source session in Sources."_s,
                u"Проверьте сессию источника в разделе «Источники»."_s},
            Failure {u"fixture_unknown"_s, u"Не удалось завершить проверку исходного источника."_s,
                u"Не удалось завершить проверку исходного источника."_s}
        };
        QJsonArray checks = evidence.value(u"checks"_s).toArray();
        for (const Failure &failure : failures)
        {
            const QString message = (locale == u"ru") ? failure.russian : failure.english;
            const QJsonObject expected {
                {u"code"_s, failure.code}, {u"message"_s, message}, {u"status"_s, u"error"_s},
                {u"url"_s, u"https://recovery.example/original"_s},
                {u"checkedAt"_s, u"2026-10-11T00:00:00.000Z"_s}
            };
            QHash<QString, QJsonValue> responses;
            QObject receiver;
            const qint64 request = Content::Client::instance()->request(u"localizationFailure"_s, {{u"code"_s, failure.code}});
            QObject::connect(Content::Client::instance(), &Content::Client::eventReceived, &receiver,
                [&](const qint64 id, const QString &event, const QJsonValue &data)
            {
                if (id == request)
                    responses.insert(event, data);
            });
            waitFor(u"localized terminal failure"_s, [&] { return responses.contains(u"error"_s); });
            require((responses.value(u"error"_s).toObject() == expected)
                && (responses.value(u"job"_s).toObject() == expected)
                && (responses.value(u"jobs"_s).toArray() == QJsonArray {expected}),
                u"Failure localization changed metadata or left raw text in error/job/jobs: %1 (%2)"_s.arg(failure.code, locale));

            QJsonArray catalog;
            Content::SourcesDialog sources(window);
            QObject::connect(Content::Client::instance(), &Content::Client::eventReceived, &sources,
                [&](qint64, const QString &event, const QJsonValue &data)
            {
                if (event == u"sources")
                    catalog = data.toArray();
            });
            sources.show();
            auto *list = sources.findChild<QListWidget *>(u"contentSourceList"_s);
            auto *health = sources.findChild<QLabel *>(u"contentSourceHealth"_s);
            auto *check = sources.findChild<QPushButton *>(u"contentSourceCheck"_s);
            require(list && health && check, u"Source health controls are missing"_s);
            waitFor(u"localized source health"_s, [&]
            {
                return (catalog.size() == 3) && list->isEnabled() && (list->count() == 3) && check->isEnabled();
            });
            require(!catalog.first().toObject().contains(u"health"_s)
                && (catalog.at(2).toObject().value(u"health"_s).toObject() == expected),
                u"Source localization changed health metadata or added health to an unchecked source"_s);
            list->setCurrentRow(2);
            const bool challenge = failure.code == u"browser_check_required";
            const QString button = (locale == u"ru")
                ? (challenge ? u"Пройти проверку"_s : u"Проверить снова"_s)
                : (challenge ? u"Complete check"_s : u"Check again"_s);
            require(health->isVisible() && (health->text() == message) && (list->item(2)->toolTip() == message)
                && check->isVisible() && check->isEnabled() && (check->text() == button)
                && (check->toolTip() == expected.value(u"url"_s).toString()),
                u"Sources does not present localized health and the correct exact-source action: %1 (%2)"_s.arg(failure.code, locale));
            checks.append(QJsonObject {{u"name"_s, u"content-localization"_s}, {u"locale"_s, locale},
                {u"code"_s, failure.code}, {u"message"_s, message}, {u"checkButton"_s, button}});
        }
        evidence[u"checks"_s] = checks;
    }
}

void exerciseContentLibrary(MainWindow *window, const QJsonObject &spec, QJsonObject &evidence)
{
    if (spec.value(u"localizationOnly"_s).toBool())
    {
        exerciseContentLocalization(window, spec, evidence);
        return;
    }
    auto *library = window->findChild<Content::LibraryWidget *>();
    require(library, u"Native content library is missing"_s);
    auto *query = library->findChild<QLineEdit *>(u"contentQuery"_s);
    auto *category = library->findChild<QComboBox *>(u"contentCategory"_s);
    auto *progress = library->findChild<QProgressBar *>(u"contentSearchProgress"_s);
    auto *emptyResults = library->findChild<QLabel *>(u"contentEmptyResults"_s);
    auto *search = library->findChild<QPushButton *>(u"contentSearch"_s);
    auto *works = library->findChild<QTableView *>(u"contentResults"_s);
    auto *variants = library->findChild<QTableView *>(u"contentVariants"_s);
    auto *filter = library->findChild<QLineEdit *>(u"contentVariantFilter"_s);
    auto *episode = library->findChild<QComboBox *>(u"contentEpisode"_s);
    auto *voice = library->findChild<QComboBox *>(u"contentVoice"_s);
    auto *source = library->findChild<QComboBox *>(u"contentVariantSource"_s);
    auto *language = library->findChild<QComboBox *>(u"contentLanguage"_s);
    auto *ads = library->findChild<QCheckBox *>(u"contentIncludeReportedAds"_s);
    auto *notes = library->findChild<QTextBrowser *>(u"contentNotes"_s);
    auto *open = library->findChild<QPushButton *>(u"contentOpen"_s);
    auto *favorite = library->findChild<QToolButton *>(u"contentSave"_s);
    auto *share = library->findChild<QToolButton *>(u"contentShare"_s);
    auto *title = library->findChild<QLabel *>(u"contentTitle"_s);
    require(ads, u"Native reported-ad filter is missing"_s);
    require(notes, u"Native source comment view is missing"_s);
    require(query && category && progress && emptyResults && search && works && variants && filter && open && favorite && share && title
        && episode && voice && source && language,
        u"Native content controls are missing"_s);

    const auto checkFavorite = [&]
    {
        const QString action = favorite->isChecked() ? Content::LibraryWidget::tr("Remove from favorites")
            : Content::LibraryWidget::tr("Add to favorites");
        require(favorite->isCheckable() && favorite->autoRaise() && favorite->text().isEmpty()
            && (favorite->toolButtonStyle() == Qt::ToolButtonIconOnly) && !favorite->icon().isNull()
            && (favorite->toolTip() == action) && (favorite->accessibleName() == action),
            u"Favorite bookmark has no accessible action or retains a text button"_s);
        const QImage icon = favorite->icon().pixmap(24, QIcon::Normal).toImage();
        bool red = false;
        for (int y = 0; y < icon.height(); ++y)
        {
            for (int x = 0; x < icon.width(); ++x)
            {
                const QColor color = icon.pixelColor(x, y);
                red |= (color.alpha() > 100) && (color.red() > color.green() * 2) && (color.red() > color.blue() * 2);
            }
        }
        require(red == favorite->isChecked(), u"Favorite bookmark does not use red only for the active state"_s);
    };

    const QString logPath = spec.value(u"fixtureLog"_s).toString();
    QJsonArray checks = evidence.value(u"checks"_s).toArray();
    const auto searchFor = [&](const QString &text)
    {
        bool finished = false;
        bool failed = false;
        const QMetaObject::Connection connection = QObject::connect(library, &Content::LibraryWidget::searchFinished,
            library, [&](const bool error)
        {
            failed = error;
            finished = true;
        });
        query->setText(text);
        category->setCurrentIndex(0);
        search->click();
        waitFor(u"search %1"_s.arg(text), [&] { return finished; });
        QObject::disconnect(connection);
        require(!failed && (works->model()->rowCount() == 1), u"Fixture search failed: %1"_s.arg(text));
    };
    const auto choose = [](QComboBox *combo, const QVariant &data)
    {
        const int index = combo->findData(data);
        require(index > 0, u"Declared variant filter choice is missing"_s);
        combo->setCurrentIndex(index);
    };
    const QHash<QString, QString> transportIcons {
        {u"torrent"_s, u"torrent-magnet"_s}, {u"video"_s, u"torrent-start"_s},
        {u"book"_s, u"help-contents"_s}, {u"manga"_s, u"help-contents"_s}, {u"file"_s, u"fileicon"_s}
    };
    const auto checkVariantIcons = [&]
    {
        require(works->currentIndex().siblingAtColumn(0).data(Qt::DecorationRole).value<QIcon>().isNull(),
            u"Aggregated work card carries a delivery icon"_s);
        require((variants->model()->columnCount() == 4) && !variants->isColumnHidden(VariantColumn)
            && (variants->horizontalHeader()->visualIndex(VariantColumn) == 0)
            && (variants->model()->headerData(VariantColumn, Qt::Horizontal).toString() == u"Variant"),
            u"Variant is not the visible first column"_s);
        for (int row = 0; row < variants->model()->rowCount(); ++row)
        {
            const QJsonObject variant = variantAt(variants, row);
            const QString iconName = transportIcons.value(variant.value(u"transport"_s).toString());
            const QModelIndex release = variants->model()->index(row, VariantColumn);
            const QIcon icon = release.data(Qt::DecorationRole).value<QIcon>();
            require(!variant.value(u"needsInspection"_s).toBool() && !iconName.isEmpty()
                && !icon.pixmap(16).isNull() && (icon.cacheKey() == UIThemeManager::instance()->getIcon(iconName).cacheKey()),
                u"Concrete version icon does not match its actual transport"_s);
            require(!release.data().toString().isEmpty()
                && (release.data(Qt::TextAlignmentRole).toInt() == (Qt::AlignLeft | Qt::AlignVCenter))
                && (variants->model()->index(row, SourceColumn).data(Qt::ToolTipRole).toString()
                    == variant.value(u"sourceName"_s).toString()),
                u"Variant text is not left-aligned or truncated source has no full tooltip"_s);
            for (int column : {SourceColumn, SizeColumn, SeedsColumn})
                require(variants->model()->index(row, column).data(Qt::DecorationRole).value<QIcon>().isNull(),
                    u"Concrete version repeats its delivery icon"_s);
        }
    };

    bool categoriesFinished = false;
    const auto categorySearch = QObject::connect(library, &Content::LibraryWidget::searchFinished,
        library, [&](bool) { categoriesFinished = true; });
    query->setText(u"categories"_s);
    search->click();
    waitFor(u"initial category result"_s, [&] { return works->model()->rowCount() == 1; });
    require(progress->isVisible(), u"Search progress is not visible near the query"_s);
    choose(category, u"anime"_s);
    require((works->model()->rowCount() == 0) && emptyResults->isVisible() && progress->isVisible(),
        u"Live category filtering restarted search or retained a hidden card"_s);
    waitFor(u"late matching category"_s, [&] { return works->model()->rowCount() == 1; });
    require(works->model()->index(0, 0).data(Qt::UserRole).toJsonObject().value(u"category"_s).toString() == u"anime",
        u"A later result bypassed the active category"_s);
    category->setCurrentIndex(0);
    require(works->model()->rowCount() == 2, u"Returning to All lost accumulated cards"_s);
    waitFor(u"category search completion"_s, [&] { return categoriesFinished; });
    QObject::disconnect(categorySearch);
    choose(category, u"games"_s);
    require((works->model()->rowCount() == 1) && !progress->isVisible(),
        u"Category filtering after completion does not use accumulated results"_s);
    category->setCurrentIndex(0);
    require(works->model()->rowCount() == 3, u"Completed category filtering discarded cards"_s);
    checks.append(u"category filters accumulated and later results live, restores All, and keeps native progress separate from source diagnostics"_s);

    bool rankingFinished = false;
    const auto rankedSearch = QObject::connect(library, &Content::LibraryWidget::searchFinished,
        library, [&](bool) { rankingFinished = true; });
    query->setText(u"ranking"_s);
    search->click();
    waitFor(u"initial low relevance result"_s, [&] { return works->model()->rowCount() == 1; });
    works->selectRow(0);
    require((works->model()->columnCount() == 1) && works->horizontalHeader()->isHidden(),
        u"Search results still expose duplicated category and version columns"_s);
    Content::Client::instance()->request(u"advanceFixture"_s, {{u"query"_s, u"ranking"_s}});
    waitFor(u"streamed relevance ordering"_s, [&] { return rankingFinished && open->isEnabled(); });
    QObject::disconnect(rankedSearch);
    const QStringList rankedIds {u"rank-exact"_s, u"rank-equal"_s, u"rank-selected"_s, u"rank-alias"_s, u"rank-unknown"_s};
    require(works->model()->rowCount() == rankedIds.size(), u"Ranked upsert duplicated or lost a title"_s);
    for (int row = 0; row < rankedIds.size(); ++row)
    {
        const QJsonObject work = works->model()->index(row, 0).data(Qt::UserRole).toJsonObject();
        require(work.value(u"id"_s).toString() == rankedIds[row], u"Relevance order or stable ties changed"_s);
        require(!work.contains(u"relevance"_s), u"Transient search rank leaked into saved work metadata"_s);
    }
    const QJsonObject selectedRanked = works->currentIndex().siblingAtColumn(0).data(Qt::UserRole).toJsonObject();
    require(selectedRanked.value(u"id"_s).toString() == u"rank-selected",
        u"Late exact result or score update changed the selected title"_s);
    require((selectedRanked.value(u"title"_s).toString() == u"Original film behind the scenes")
        && (selectedRanked.value(u"year"_s).toInt() == 1984)
        && (works->currentIndex().siblingAtColumn(0).data().toString() == u"Original film behind the scenes (1984)")
        && (title->text() == u"Original film behind the scenes (1984)"),
        u"Known film year is missing from the card or heading, or changed canonical metadata"_s);
    const QJsonArray rankedNotes = selectedRanked.value(u"notes"_s).toArray();
    require(!rankedNotes.isEmpty() && (rankedNotes.first().toObject().value(u"text"_s).toString() == u"Latest ranked result"),
        u"Repeated ID updated the wrong row after sorting"_s);
    checks.append(u"streamed relevance tiers sort stably; selection and repeated ID upsert survive promotion"_s);

    bool promotionFinished = false;
    const auto promotedSearch = QObject::connect(library, &Content::LibraryWidget::searchFinished,
        library, [&](bool) { promotionFinished = true; });
    query->setText(u"promoted"_s);
    search->click();
    waitFor(u"publication before canonical promotion"_s, [&] { return works->model()->rowCount() == 1; });
    works->selectRow(0);
    waitFor(u"selected publication details"_s, [&] { return open->isEnabled() && open->isVisible(); });
    filter->setText(u"clean"_s);
    Content::Client::instance()->request(u"advanceFixture"_s, {{u"query"_s, u"promoted"_s}});
    waitFor(u"canonical result replacement"_s, [&]
    {
        return works->currentIndex().siblingAtColumn(0).data(Qt::UserRole).toJsonObject().value(u"id"_s).toString()
            == u"canonical-promoted";
    });
    require(!promotionFinished && open->isVisible() && (variants->model()->rowCount() >= 1)
        && (variantAt(variants, variants->currentIndex().row()).value(u"id"_s).toString() == u"clean")
        && (filter->text() == u"clean"),
        u"Canonical promotion reset the selected exact publication or explicit filter"_s);
    waitFor(u"canonical promotion completion"_s, [&] { return promotionFinished && open->isEnabled(); });
    QObject::disconnect(promotedSearch);
    waitFor(u"automatic hydration added by canonical replacement"_s, [&]
    {
        return variantRow(variants, u"promotion-pending"_s, false) >= 0;
    });
    require((variants->model()->rowCount() == 2) && (filter->text() == u"clean")
        && (variantAt(variants, variants->currentIndex().row()).value(u"id"_s).toString() == u"clean")
        && (works->currentIndex().siblingAtColumn(0).data(Qt::UserRole).toJsonObject().value(u"id"_s).toString()
            == u"canonical-promoted"),
        u"Late replacement hydration reset the selected ready variant, filter or canonical work"_s);
    qint64 promotionCompleted = 0;
    qint64 promotionHydrated = 0;
    for (const QJsonValue &value : fixtureEvents(logPath))
    {
        const QJsonObject event = value.toObject();
        if (event.value(u"sourceId"_s).toString() != u"promotion-pending")
            continue;

        if (event.value(u"event"_s).toString() == u"promotionSearchComplete")
            promotionCompleted = event.value(u"time"_s).toInteger();
        else if (event.value(u"event"_s).toString() == u"details")
            promotionHydrated = event.value(u"time"_s).toInteger();
    }
    require((promotionCompleted > 0) && (promotionHydrated > promotionCompleted),
        u"Replacement hydration did not exercise a provider completing after search"_s);
    checks.append(u"canonical replacement hydrates its newly pending source after search completes, preserving the selected ready variant and explicit filter"_s);

    searchFor(u"games-recovery"_s);
    waitFor(u"automatic working publication after source failure"_s, [&]
    {
        return (variantRow(variants, u"fixture-b"_s, false) >= 0) && open->isEnabled();
    });
    require(hasFixtureEvent(logPath, u"inspectionError"_s, u"fixture-a"_s),
        u"The failed background publication was not exercised"_s);
    for (QLabel *label : library->findChildren<QLabel *>())
        require(!label->text().contains(u"HTTP 403"), u"Background source failure leaked into the search interface"_s);
    require(notes->toPlainText().contains(u"Install the game, then apply the cooperative patch."),
        u"Automatic hydration lost the working source instructions"_s);
    checks.append(u"selected card hydrates every pending provider and keeps background failures out of search"_s);

    bool gamesFinished = false;
    const QMetaObject::Connection gamesSearch = QObject::connect(library, &Content::LibraryWidget::searchFinished,
        library, [&](bool) { gamesFinished = true; });
    query->setText(u"games"_s);
    search->click();
    waitFor(u"automatic first game editions"_s, [&]
    {
        return (variantRow(variants, u"fixture-a"_s, false) >= 0) && open->isEnabled();
    });
    const QJsonObject selectedGame = variantAt(variants, variants->currentIndex().row());
    waitFor(u"all initial and late provider editions"_s, [&]
    {
        return gamesFinished && (variantRow(variants, u"fixture-b"_s, false) >= 0)
            && (variantRow(variants, u"fixture-c"_s, false) >= 0);
    });
    QObject::disconnect(gamesSearch);
    require(variantAt(variants, variants->currentIndex().row()).value(u"id"_s) == selectedGame.value(u"id"_s),
        u"Later hydrated providers replaced the selected edition"_s);
    for (int row = 0; row < variants->model()->rowCount(); ++row)
        require(!variantAt(variants, row).value(u"needsInspection"_s).toBool(), u"A manual expansion placeholder remains visible"_s);
    const int second = variantRow(variants, u"fixture-b"_s, false);
    variants->selectRow(second);
    require(notes->toPlainText().contains(u"Author B") && notes->toPlainText().contains(u"<literal>"),
        u"Source comments or escaped author text are missing"_s);
    require(notes->toHtml().contains(u"https://fixture-b.example/game#comment-id-1"),
        u"Comment attribution does not link to its source publication"_s);
    auto *detailPages = library->findChild<QTabWidget *>(u"contentDetailPages"_s);
    require(detailPages && (detailPages->currentIndex() == 0) && detailPages->isTabVisible(1),
        u"Versions are not the default detail view"_s);
    detailPages->setCurrentIndex(1);
    require(notes->isVisible() && (notes->maximumHeight() == QWIDGETSIZE_MAX),
        u"Description and comments retain a nested constrained scroll area"_s);
    detailPages->setCurrentIndex(0);
    filter->setText(u"Source B patch"_s);
    require((variants->model()->rowCount() == 1)
        && (variantAt(variants, 0).value(u"sourceId"_s).toString() == u"fixture-b"),
        u"Version filtering lost words across source and release"_s);
    filter->clear();
    checks.append(u"all source editions appear incrementally without expansion controls; exact selection and source comments survive late results"_s);
    searchFor(u"game-downloads"_s);
    waitFor(u"distinct game download choices"_s, [&] { return (variants->model()->rowCount() == 5) && open->isEnabled(); });
    checkVariantIcons();
    QSet<QString> downloadLabels;
    for (int row = 0; row < variants->model()->rowCount(); ++row)
        downloadLabels.insert(variants->model()->index(row, VariantColumn).data().toString());
    const QString visibleDownloads = QStringList(downloadLabels.cbegin(), downloadLabels.cend()).join(u'\n');
    require((downloadLabels.size() == 5) && visibleDownloads.contains(u".torrent")
        && visibleDownloads.contains(u".rar") && visibleDownloads.contains(u"Hosters")
        && visibleDownloads.contains(u"Drive") && visibleDownloads.contains(u"Fix"),
        u"Torrent, archive, hosting and fix choices are indistinguishable"_s);
    require(variants->horizontalHeader()->isHidden(), u"Text-only versions retain duplicated headers"_s);
    require(!episode->isVisible() && !voice->isVisible() && !source->isVisible(),
        u"Non-episodic game downloads expose irrelevant episode pickers"_s);
    checks.append(u"five game downloads remain visibly distinct: torrent, archive, Hosters, Drive, fix"_s);

    require(!share->menu(), u"Sharing still requires a choice between work and exact version"_s);
    const QJsonObject sharedWork = works->currentIndex().siblingAtColumn(0).data(Qt::UserRole).toJsonObject();
    const QJsonObject sharedVariant = variantAt(variants, variants->currentIndex().row());
    for (const bool selected : {true, false})
    {
        filter->setText(selected ? QString() : u"no-matching-share-version"_s);
        require(share->isEnabled(), u"Sharing a work requires a selected version"_s);
        const int previousEvents = fixtureEvents(logPath).size();
        share->click();
        QJsonObject sharedParams;
        waitFor(u"whole-work share request"_s, [&]
        {
            const QJsonArray events = fixtureEvents(logPath);
            for (qsizetype index = previousEvents; index < events.size(); ++index)
            {
                const QJsonObject event = events[index].toObject();
                if (event.value(u"event"_s).toString() == u"share")
                    sharedParams = event.value(u"params"_s).toObject();
            }
            return !sharedParams.isEmpty();
        });
        require((sharedParams.value(u"mode"_s).toString() == u"work")
            && (sharedParams.value(u"work"_s).toObject() == sharedWork)
            && (selected ? (sharedParams.value(u"variant"_s).toObject() == sharedVariant) : !sharedParams.contains(u"variant"_s)),
            u"Share lost known work variants or changed the optional exact selection"_s);
    }
    filter->clear();
    checks.append(u"single-click Share sends the whole known work, with an exact preference only when selected"_s);

    const QSize previousSize = window->size();
    const ColorScheme previousScheme = UIThemeManager::instance()->colorScheme();
    const QDir renderDirectory(QFileInfo(spec.value(u"evidencePath"_s).toString()).absolutePath());
    QJsonArray renders;
    const auto capture = [&](const QString &name, const QSize &size, const ColorScheme scheme)
    {
        UIThemeManager::instance()->previewColorScheme(scheme);
        window->resize(size);
        QCoreApplication::processEvents();
        checkFavorite();
        const QRect titleBounds(title->mapTo(library, QPoint()), title->size());
        const QRect favoriteBounds(favorite->mapTo(library, QPoint()), favorite->size());
        require((favoriteBounds.left() > titleBounds.right()) && (favoriteBounds.top() == titleBounds.top())
            && (title->width() <= title->sizeHint().width())
            && window->rect().contains(favorite->mapTo(window, favorite->rect().bottomRight())),
            u"Favorite bookmark is not beside the title or is clipped"_s);
        const QLayout *heading = title->parentWidget()->layout()->itemAt(0)->layout();
        const int availableTitleWidth = title->width() + heading->itemAt(heading->count() - 1)->geometry().width();
        const int singleLineWidth = title->fontMetrics().size(Qt::TextSingleLine, title->text()).width();
        require((singleLineWidth > availableTitleWidth) || (title->width() >= singleLineWidth),
            u"Work title wraps despite unused heading width"_s);
        require((variants->horizontalHeader()->sectionResizeMode(VariantColumn) == QHeaderView::Stretch)
            && (variants->horizontalHeader()->sectionResizeMode(SourceColumn) == QHeaderView::Interactive),
            u"Version table no longer gives its remaining width to the variant"_s);
        if (!variants->isColumnHidden(SourceColumn))
        {
            require((variants->columnWidth(SourceColumn) == 100)
                && (variants->columnWidth(VariantColumn) > variants->columnWidth(SourceColumn)),
                u"Source names consume the variant width at %1 pixels"_s.arg(size.width()));
        }
        require(variants->viewport()->height() >= variants->verticalHeader()->defaultSectionSize() * 2,
            u"Loaded layout leaves fewer than two version rows visible"_s);
        require((open->width() >= open->minimumSizeHint().width())
            && window->rect().contains(open->mapTo(window, open->rect().bottomRight())),
            u"Primary content action is clipped"_s);
        for (QComboBox *combo : {episode, voice, source, language})
        {
            if (combo->isVisible())
            {
                require((combo->width() >= combo->minimumSizeHint().width())
                    && window->rect().contains(combo->mapTo(window, combo->rect().bottomRight())),
                    u"Episode, voice, source or language picker is clipped"_s);
            }
        }
        const QString path = renderDirectory.filePath(name + u".png"_s);
        require(window->grab().save(path), u"Unable to retain native content render"_s);
        renders.append(path);
    };
    capture(u"library-modern-light-normal"_s, {1280, 800}, ColorScheme::Light);
    capture(u"library-modern-dark-normal"_s, {1280, 800}, ColorScheme::Dark);
    capture(u"library-modern-dark-minimum"_s, {800, 600}, ColorScheme::Dark);
    detailPages->setCurrentIndex(1);
    QCoreApplication::processEvents();
    const QString detailsRender = renderDirectory.filePath(u"library-modern-details-minimum.png"_s);
    require(window->grab().save(detailsRender), u"Unable to retain details render"_s);
    renders.append(detailsRender);
    detailPages->setCurrentIndex(0);
    UIThemeManager::instance()->previewColorScheme(previousScheme);
    window->resize(previousSize);
    evidence[u"renders"_s] = renders;
    checks.append(u"loaded native content keeps versions and primary action visible at normal and minimum sizes in Light and Dark"_s);

    for (const QString &headingQuery : {u"heading-frieren"_s, u"heading-ranobe"_s, u"heading-long"_s})
    {
        searchFor(headingQuery);
        capture(u"library-%1"_s.arg(headingQuery), headingQuery == u"heading-frieren" ? QSize(1440, 900) : QSize(1280, 720),
            headingQuery == u"heading-frieren" ? ColorScheme::Dark : ColorScheme::Light);
        if (title->text().startsWith(u"Очень длинное"))
        {
            require((title->width() < title->sizeHint().width())
                && (title->heightForWidth(title->width()) > title->fontMetrics().height()),
                u"Long work title no longer wraps within its available width"_s);
        }
    }
    UIThemeManager::instance()->previewColorScheme(previousScheme);
    window->resize(previousSize);
    evidence[u"renders"_s] = renders;
    checks.append(u"work headings use intrinsic one-line width, wrap only when exhausted and keep the bookmark beside the title"_s);

    searchFor(u"table-metadata"_s);
    require(variants->horizontalHeader()->isVisible() && !variants->isColumnHidden(SizeColumn)
        && !variants->isColumnHidden(SeedsColumn) && (variants->model()->index(0, SeedsColumn).data().toString() == u"0"),
        u"Torrent size or zero seeders lost their comparison headers"_s);

    searchFor(u"canonical-facets"_s);
    waitFor(u"canonical facet details"_s, [&] { return language->isVisible() && open->isEnabled(); });
    checkVariantIcons();
    capture(u"library-variant-icons-light-normal"_s, {1280, 800}, ColorScheme::Light);
    capture(u"library-variant-icons-dark-minimum"_s, {800, 600}, ColorScheme::Dark);
    require((episode->count() == 3) && (voice->count() == 4) && (language->count() == 4),
        u"Canonical episode, studio or language facets were duplicated"_s);
    choose(episode, QJsonObject {{u"episode"_s, u"2"_s}, {u"season"_s, u"1"_s}});
    choose(voice, u"OnWave"_s);
    choose(language, u"ru"_s);
    require(variants->model()->rowCount() == 2,
        u"Canonical facets did not match raw and prefixed studio names"_s);
    require(variants->model()->index(0, VariantColumn).data().toString().contains(u"1080p"),
        u"Known quality is missing before playback"_s);
    choose(voice, u"Kazoku Project"_s);
    require((variants->model()->rowCount() == 1)
        && variants->model()->index(0, VariantColumn).data().toString().contains(u"Дубляж Kazoku Project"),
        u"Russian language filtering lost the original dubbing type"_s);
    choose(language, u"ja"_s);
    require((variants->model()->rowCount() == 1)
        && variants->model()->index(0, VariantColumn).data().toString().contains(u"Субтитры Kazoku Project"),
        u"Studio normalization lost the original subtitle type"_s);
    episode->setCurrentIndex(0);
    language->setCurrentIndex(0);
    choose(voice, u"Original long studio name for minimum window width"_s);
    choose(source, QJsonObject {{u"sourceId"_s, u"fixture-other"_s}, {u"player"_s, u""_s}, {u"transport"_s, u"video"_s}});
    capture(u"library-canonical-facets-light-minimum"_s, {800, 600}, ColorScheme::Light);
    capture(u"library-canonical-facets-dark-minimum"_s, {800, 600}, ColorScheme::Dark);
    const QString sourceToolTip = works->model()->index(0, 0).data(Qt::ToolTipRole).toString();
    require((sourceToolTip.count(u"YummyAnime") == 1) && sourceToolTip.contains(u"Nyaa.si") && sourceToolTip.contains(u"Rutube"),
        u"Work metadata counts variants instead of distinct sources"_s);
    checks.append(u"canonical episode/studio facets, explicit language filter, raw dubbing/subtitle labels and known quality survive the compact four-facet layout"_s);

    bool refinementFinished = false;
    const auto refinementSearch = QObject::connect(library, &Content::LibraryWidget::searchFinished,
        library, [&](bool) { refinementFinished = true; });
    query->setText(u"facet-refinement"_s);
    search->click();
    waitFor(u"initial declared episode facet"_s, [&] { return open->isEnabled(); });
    choose(episode, QJsonObject {{u"episode"_s, u"2"_s}, {u"season"_s, u""_s}});
    choose(voice, u"OnWave"_s);
    choose(language, u"ru"_s);
    Content::Client::instance()->request(u"advanceFixture"_s, {{u"query"_s, u"facet-refinement"_s}});
    waitFor(u"authoritative canonical facet refinement"_s, [&] { return refinementFinished; });
    QObject::disconnect(refinementSearch);
    require((episode->currentData().toJsonObject().value(u"season"_s).toString() == u"2")
        && (voice->currentData().toString() == u"OnWave") && (language->currentData().toString() == u"ru")
        && (variants->model()->rowCount() == 1)
        && (variantAt(variants, variants->currentIndex().row()).value(u"id"_s).toString() == u"onwave-direct"),
        u"Canonical refinement lost the explicit facet or exact selected variant"_s);
    checks.append(u"canonical refinement remaps the selected variant's explicit facet without resetting unrelated choices"_s);

    searchFor(u"archive-parts"_s);
    require((variants->model()->rowCount() == 6) && (variants->contextMenuPolicy() == Qt::CustomContextMenu),
        u"Complete torrent presentation lost the source action or hid unrelated archive parts"_s);
    QSet<QString> visibleParts;
    for (int row = 0; row < variants->model()->rowCount(); ++row)
        visibleParts.insert(variantAt(variants, row).value(u"id"_s).toString());
    require(!visibleParts.contains(u"part-one"_s) && visibleParts.contains(u"part-two"_s)
        && visibleParts.contains(u"part-unknown"_s) && visibleParts.contains(u"part-other"_s)
        && visibleParts.contains(u"part-three"_s)
        && variants->model()->index(0, VariantColumn).data().toString().contains(u"1.2.3"),
        u"Archive parts were hidden without an exact complete magnet for the same publication and known version"_s);
    require(works->currentIndex().siblingAtColumn(0).data(Qt::UserRole).toJsonObject().value(u"variants"_s).toArray().size() == 7,
        u"Presentation filtering discarded archive parts from the underlying work"_s);
    checks.append(u"complete torrents hide only redundant parts of the exact known version; partial-only, unknown and other sources remain available"_s);

    bool episodesFinished = false;
    const auto episodesSearch = QObject::connect(library, &Content::LibraryWidget::searchFinished,
        library, [&](bool) { episodesFinished = true; });
    query->setText(u"episodes"_s);
    search->click();
    waitFor(u"mixed episode publications"_s, [&] { return works->model()->rowCount() == 1; });
    works->selectRow(0);
    waitFor(u"episode pickers"_s, [&] { return episode->isVisible() && open->isEnabled(); });
    require((episode->currentIndex() == 0) && (voice->currentIndex() == 0) && (source->currentIndex() == 0),
        u"A new work inherited episode filters"_s);
    require(episode->findData(QJsonObject {{u"episode"_s, u"2"_s}, {u"season"_s, u""_s}})
        < episode->findData(QJsonObject {{u"episode"_s, u"10"_s}, {u"season"_s, u""_s}}),
        u"Declared episode numbers are not in numeric order"_s);
    choose(episode, QJsonObject {{u"episode"_s, u"2"_s}, {u"season"_s, u""_s}});
    choose(voice, u"Studio B"_s);
    waitFor(u"automatic filtered Yani publication hydration"_s, [&]
    {
        return source->findData(QJsonObject {{u"sourceId"_s, u"fixture-yani"_s}, {u"player"_s, u"CVH"_s},
            {u"transport"_s, u"video"_s}}) >= 0;
    });
    require((episode->currentData().toJsonObject().value(u"episode"_s).toString() == u"2")
        && (voice->currentData().toString() == u"Studio B") && (source->currentIndex() == 0),
        u"Automatic hydration reset the user's episode, voice or provider filter"_s);
    choose(source, QJsonObject {{u"sourceId"_s, u"fixture-yani"_s}, {u"player"_s, u"Kodik"_s}, {u"transport"_s, u"video"_s}});
    require((variants->model()->rowCount() == 1) && (source->currentText() == u"YummyAnime [Kodik]")
        && variantAt(variants, 0).value(u"episode"_s).toString() == u"2"
        && variantAt(variants, 0).value(u"translation"_s).toString() == u"Studio B"
        && variantAt(variants, 0).value(u"player"_s).toString() == u"Kodik"
        && variants->isColumnHidden(SourceColumn) && variants->isColumnHidden(SizeColumn) && variants->isColumnHidden(SeedsColumn)
        && variants->horizontalHeader()->isHidden(),
        u"Episode, studio and actual player did not narrow to the exact visible release"_s);
    checkVariantIcons();
    capture(u"library-episodes-light-normal"_s, {1280, 800}, ColorScheme::Light);
    capture(u"library-episodes-dark-minimum"_s, {800, 600}, ColorScheme::Dark);
    choose(source, QJsonObject {{u"sourceId"_s, u"fixture-yani"_s}, {u"player"_s, u"CVH"_s}, {u"transport"_s, u"video"_s}});
    waitFor(u"late mixed publications"_s, [&] { return episodesFinished; });
    QObject::disconnect(episodesSearch);
    require((variantAt(variants, variants->currentIndex().row()).value(u"sourceId"_s).toString() == u"fixture-yani")
        && (variantAt(variants, variants->currentIndex().row()).value(u"id"_s).toString() == u"collision")
        && (source->currentText() == u"YummyAnime [CVH]"),
        u"Streamed results replaced the selected publication with another source's identical ID"_s);
    source->setCurrentIndex(0);
    voice->setCurrentIndex(0);
    episode->setCurrentIndex(0);
    require(variants->model()->rowCount() == 130, u"Expansion or late inspection lost a mixed publication"_s);
    capture(u"library-episodes-all-dark-normal"_s, {1280, 800}, ColorScheme::Dark);
    capture(u"library-episodes-all-dark-minimum"_s, {800, 600}, ColorScheme::Dark);
    choose(episode, QJsonObject {{u"episode"_s, u""_s}, {u"season"_s, u""_s}});
    require(variants->model()->rowCount() > 50, u"Unknown episodes were silently assigned to a numbered episode"_s);
    for (int row = 0; row < variants->model()->rowCount(); ++row)
        require(variantAt(variants, row).value(u"episode"_s).toString().isEmpty(), u"Unknown episode filter includes a numbered release"_s);
    choose(episode, QJsonObject {{u"episode"_s, u"1–12"_s}, {u"season"_s, u""_s}});
    require(variants->model()->rowCount() > 1, u"Literal episode batch is not explicitly selectable"_s);
    choose(episode, QJsonObject {{u"episode"_s, u"2"_s}, {u"season"_s, u"2"_s}});
    require((variants->model()->rowCount() == 1)
        && variantAt(variants, 0).value(u"season"_s).toString() == u"2",
        u"Explicit season was conflated with an unknown season"_s);
    episode->setCurrentIndex(0);
    choose(source, QJsonObject {{u"sourceId"_s, u"fixture-hybrid"_s}, {u"player"_s, u"Shared player"_s}, {u"transport"_s, u"torrent"_s}});
    require(source->currentText().contains(u"[Torrent]") && (variants->model()->rowCount() == 1)
        && variantAt(variants, 0).value(u"transport"_s).toString() == u"torrent",
        u"One provider's stream and torrent choices are ambiguous"_s);
    source->setCurrentIndex(0);
    filter->setText(u"YummyAnime Kodik"_s);
    require((variants->model()->rowCount() == 12) && variants->isColumnHidden(SizeColumn) && variants->isColumnHidden(SeedsColumn),
        u"Text filtering is lost or empty torrent columns remain beside filtered video (rows=%1, size hidden=%2, seeds hidden=%3)"_s
            .arg(variants->model()->rowCount()).arg(variants->isColumnHidden(SizeColumn)).arg(variants->isColumnHidden(SeedsColumn)));
    library->acceptSharedLink(QUrl(spec.value(u"episodeLink"_s).toString()));
    waitFor(u"exact shared episode and voice"_s, [&]
    {
        return (variants->model()->rowCount() == 1) && open->isEnabled()
            && (variantAt(variants, 0).value(u"sourceId"_s).toString() == u"fixture-yani");
    });
    require((variantAt(variants, variants->currentIndex().row()).value(u"id"_s).toString() == u"collision")
        && (voice->currentData().toString() == u"Studio B")
        && (episode->currentData().toJsonObject().value(u"episode"_s).toString() == u"2")
        && (source->currentText() == u"YummyAnime [CVH]") && filter->text().isEmpty(),
        u"Exact shared identity did not reveal its episode, voice and actual player"_s);
    evidence[u"renders"_s] = renders;
    UIThemeManager::instance()->previewColorScheme(previousScheme);
    window->resize(previousSize);
    checks.append(u"mixed 129 releases: episode/voice/player facets, deferred inspection, literal batches and unknown seasons, streamed source+ID identity, exact link and visible columns"_s);
    checks.append(u"Variant leads with a left-aligned delivery icon and text; source filtering keeps it visible, and long source names preserve variant width and their full tooltip"_s);

    searchFor(u"ads-mixed"_s);
    waitFor(u"mixed video details"_s, [&] { return open->isEnabled(); });
    require(!ads->isHidden() && !ads->isChecked() && (variants->model()->rowCount() == 2),
        u"Known reported video was not filtered by default"_s);
    for (int row = 0; row < variants->model()->rowCount(); ++row)
        require(variantAt(variants, row).value(u"adStatus"_s).toString() != u"reported", u"Reported video remained visible"_s);
    ads->click();
    require(variants->model()->rowCount() == 3, u"Explicit inclusion did not reveal the reported video"_s);
    checks.append(u"known reported excluded; clean and unknown retained; explicit inclusion works"_s);

    searchFor(u"ads-only"_s);
    require(!ads->isHidden() && !ads->isChecked() && (variants->model()->rowCount() == 0) && !open->isEnabled(),
        u"Reported-only result was automatically selected"_s);
    ads->click();
    require((variants->model()->rowCount() == 1) && open->isEnabled(), u"Reported-only result has no explicit choice"_s);

    searchFor(u"ads-discovered"_s);
    waitFor(u"newly reported details"_s, [&] { return hasFixtureEvent(logPath, u"details"_s, u"ads-discovered"_s); });
    waitFor(u"newly reported filter"_s, [&] { return !ads->isHidden(); });
    require(!ads->isChecked() && (variants->model()->rowCount() == 0) && !open->isEnabled(),
        u"Ordinary inspection silently enabled reported advertising"_s);
    checks.append(u"reported-only and newly reported require explicit inclusion"_s);

    library->acceptSharedLink(QUrl(spec.value(u"reportedLink"_s).toString()));
    waitFor(u"exact reported link"_s, [&]
    {
        return (variants->model()->rowCount() == 1) && ads->isChecked() && open->isEnabled();
    });
    require(variantAt(variants, variants->currentIndex().row()).value(u"id"_s).toString() == u"reported",
        u"Exact reported link substituted a different release"_s);

    library->acceptSharedLink(QUrl(spec.value(u"missingLink"_s).toString()));
    waitFor(u"missing exact release"_s, [&] { return variants->model()->rowCount() == 1; });
    require(!variants->currentIndex().isValid() && !open->isEnabled(),
        u"Missing exact release silently selected a replacement"_s);
    checks.append(u"exact reported choice retained; vanished exact release never substituted"_s);

    for (const QJsonValue &value : fixtureEvents(logPath))
    {
        const QJsonObject event = value.toObject();
        require(event.value(u"event"_s).toString() != u"resolve", u"Selection unexpectedly resolved or downloaded content"_s);
    }

    const auto setBrowserFixture = [&](const QJsonObject &state)
    {
        bool completed = false;
        const qint64 request = Content::Client::instance()->request(u"setBrowserFixture"_s, state);
        const auto connection = QObject::connect(Content::Client::instance(), &Content::Client::eventReceived,
            library, [&](const qint64 id, const QString &event, const QJsonValue &)
        {
            if ((id == request) && (event == u"complete"))
                completed = true;
        });
        waitFor(u"browser fixture state"_s, [&] { return completed; });
        QObject::disconnect(connection);
    };

    {
        Content::SourcesDialog sources(window);
        sources.show();
        auto *install = sources.findChild<QPushButton *>(u"contentBrowserInstall"_s);
        auto *instructions = sources.findChild<QLabel *>(u"contentBrowserSetup"_s);
        auto *directory = sources.findChild<QLineEdit *>(u"contentBrowserExtensionDirectory"_s);
        auto *status = sources.findChild<QLabel *>(u"contentBrowserStatus"_s);
        auto *sourceList = sources.findChild<QListWidget *>();
        require(install && instructions && directory && status && sourceList,
            u"Browser extension installation controls are missing"_s);
        waitFor(u"initial browser connection status"_s, [&] { return install->isEnabled(); });
        require(install->isVisible() && (install->text() == u"Install extension"),
            u"Fresh user has no explicit extension installation action"_s);
        install->click();
        waitFor(u"browser extension setup"_s, [&] { return directory->isVisible() && install->isEnabled(); });
        require(directory->isReadOnly() && directory->isEnabled()
            && (directory->text() == u"C:\\Original qcatch\\extension")
            && instructions->wordWrap() && instructions->text().contains(u"Developer mode")
            && instructions->text().contains(u"Load unpacked")
            && instructions->text().contains(u"edge://extensions")
            && (status->text() == u"Waiting for the extension…"),
            u"Browser setup lacks clear manual steps or falsely claims connection"_s);
        const QSize previousSourcesSize = sources.size();
        const ColorScheme previousSourcesScheme = UIThemeManager::instance()->colorScheme();
        UIThemeManager::instance()->previewColorScheme(ColorScheme::Dark);
        for (const QSize size : {previousSourcesSize, sources.minimumSize()})
        {
            sources.resize(size);
            QCoreApplication::processEvents();
            auto *scroll = sources.findChild<QScrollArea *>();
            require(scroll, u"Source settings have no scrollable page"_s);
            scroll->ensureWidgetVisible(directory);
            QCoreApplication::processEvents();
            const QString name = (size == previousSourcesSize)
                ? u"sources-browser-dark-normal.png"_s : u"sources-browser-dark-minimum.png"_s;
            const QString path = renderDirectory.filePath(name);
            require(sources.grab().save(path), u"Unable to retain browser setup render"_s);
            renders.append(path);
        }
        evidence[u"renders"_s] = renders;
        sources.resize(previousSourcesSize);
        UIThemeManager::instance()->previewColorScheme(previousSourcesScheme);
        sourceList->setCurrentRow(1);
        require(!instructions->isVisible() && !directory->isVisible(), u"Browser setup leaked into another source page"_s);
        bool statusCompleted = false;
        qint64 statusRequest = 0;
        const auto statusConnection = QObject::connect(Content::Client::instance(), &Content::Client::eventReceived,
            &sources, [&](const qint64 id, const QString &event, const QJsonValue &)
        {
            if (event == u"browser")
                statusRequest = id;
            else if ((id == statusRequest) && (event == u"complete"))
                statusCompleted = true;
        });
        sourceList->setCurrentRow(0);
        waitFor(u"browser setup status refresh"_s, [&] { return statusCompleted; });
        QObject::disconnect(statusConnection);
        require(!instructions->isVisible() && !directory->isVisible() && (install->text() == u"Connect extension"),
            u"Returning to setup retained stale browser instructions or offered a reinstall"_s);
        setBrowserFixture({{u"ready"_s, true}});
        waitFor(u"authenticated browser readiness"_s, [&] { return status->text() == u"Connected"; });
        require(!instructions->isVisible() && !directory->isVisible()
            && hasFixtureEvent(logPath, u"connectBrowser"_s, u"edge"_s),
            u"Connected browser setup did not finish after the real status response"_s);
        setBrowserFixture({{u"ready"_s, false}, {u"installed"_s, false}});
        waitFor(u"connection removed while Sources remains visible"_s, [&]
        {
            return install->isVisible() && (install->text() == u"Install extension");
        });
        auto *health = sources.findChild<QLabel *>(u"contentSourceHealth"_s);
        auto *check = sources.findChild<QPushButton *>(u"contentSourceCheck"_s);
        require(health && check, u"Source health controls are missing"_s);
        sourceList->setCurrentRow(2);
        require(health->isVisible() && health->text().contains(u"browser check") && check->isVisible()
            && (check->text() == u"Complete check") && (check->toolTip() == u"https://recovery.example/original")
            && !sourceList->currentItem()->icon().isNull(),
            u"Manual Sources view lost the failed source's reason, badge or retry"_s);
        require(!sources.findChild<QPushButton *>(u"contentSourceOpen"_s),
            u"Sources retained a competing OS-default browser action"_s);
        require(!hasFixtureEvent(logPath, u"checkSource"_s, u"fixture-error"_s),
            u"Opening Sources launched an interactive browser check"_s);
        auto *enabled = sources.findChild<QCheckBox *>();
        QLineEdit *sourceSetting = nullptr;
        for (auto *field : sources.findChildren<QLineEdit *>())
        {
            if (field->text() == u"Original saved address")
                sourceSetting = field;
        }
        require(enabled && sourceSetting && enabled->isEnabled() && sourceSetting->isEnabled(),
            u"Source fixture editors are missing or remain disabled before a check"_s);
        check->click();
        require(!check->isEnabled() && !sourceList->isEnabled() && !enabled->isEnabled()
            && !sourceSetting->isEnabled() && (check->text() == u"Checking…"),
            u"Interactive browser check has no loading state"_s);
        waitFor(u"unsolved browser challenge remains actionable"_s, [&]
        {
            return check->isEnabled() && (check->text() == u"Complete check");
        });
        require(health->isVisible() && sourceList->isEnabled() && !sourceList->currentItem()->icon().isNull(),
            u"Unsolved browser check cleared the source failure"_s);
        require(enabled->isEnabled(), u"Completed browser check left source enablement disabled"_s);
        for (auto *field : sources.findChildren<QLineEdit *>())
        {
            if (field->text() == u"Original saved address")
                require(field->isEnabled(), u"Completed browser check left its source setting disabled"_s);
        }
        check->click();
        waitFor(u"successful source recheck"_s, [&]
        {
            return !health->isVisible() && !check->isVisible()
                && sourceList->currentItem()->icon().isNull();
        });
        int interactiveRequests = 0;
        for (const QJsonValue &value : fixtureEvents(logPath))
        {
            const QJsonObject event = value.toObject();
            if (event.value(u"event"_s).toString() != u"checkSource")
                continue;

            const QJsonObject params = event.value(u"params"_s).toObject();
            require((params.value(u"id"_s).toString() == u"fixture-error")
                && params.value(u"interactive"_s).toBool() && !params.contains(u"url"_s),
                u"Manual check lost the persisted failed-operation owner"_s);
            ++interactiveRequests;
        }
        require(interactiveRequests == 2, u"Interactive browser check repeated without a user action"_s);
        sources.close();
    }
    checks.append(u"extension installation has persistent manual steps and reports only authenticated browser connection"_s);
    checks.append(u"source errors are shown only in manually opened Sources; successful recheck removes the badge"_s);
    checks.append(u"Complete check explicitly retries the persisted source operation; an unsolved challenge keeps its action and failure mark"_s);

    searchFor(u"source-recovery"_s);
    waitFor(u"failed background publication before manual recovery"_s, [&]
    {
        return hasFixtureEvent(logPath, u"inspectionError"_s, u"fixture-error"_s) && open->isEnabled();
    });
    filter->setText(u"clean"_s);
    require((variants->model()->rowCount() == 1)
        && (variantAt(variants, variants->currentIndex().row()).value(u"id"_s).toString() == u"ready-clean"),
        u"Ready publication was lost before manual source recovery"_s);
    auto *sourcesButton = library->findChild<QPushButton *>(u"contentSources"_s);
    require(sourcesButton, u"Library Sources action is missing"_s);
    sourcesButton->click();
    auto *recoveryDialog = library->findChild<Content::SourcesDialog *>();
    require(recoveryDialog, u"Library did not open its connected Sources dialog"_s);
    auto *recoverySources = recoveryDialog->findChild<QListWidget *>(u"contentSourceList"_s);
    auto *recoveryHealth = recoveryDialog->findChild<QLabel *>(u"contentSourceHealth"_s);
    auto *recoveryCheck = recoveryDialog->findChild<QPushButton *>(u"contentSourceCheck"_s);
    require(recoverySources && recoveryHealth && recoveryCheck, u"Manual source recovery controls are missing"_s);
    waitFor(u"recovery source list"_s, [&] { return (recoverySources->count() == 3) && recoveryCheck->isEnabled(); });
    recoverySources->setCurrentRow(2);
    require(recoveryHealth->isVisible() && recoveryCheck->isVisible(), u"Failed source has no manual recovery action"_s);
    recoveryCheck->click();
    waitFor(u"current card updated by manual source retry"_s, [&]
    {
        return (variantRow(variants, u"fixture-error"_s, false) >= 0) && !recoveryHealth->isVisible();
    });
    const QJsonObject recoveredWork = works->currentIndex().siblingAtColumn(0).data(Qt::UserRole).toJsonObject();
    require((variants->model()->rowCount() == 2) && (filter->text() == u"clean")
        && (variantAt(variants, variants->currentIndex().row()).value(u"id"_s).toString() == u"ready-clean")
        && (recoveredWork.value(u"id"_s).toString() == u"source-recovery")
        && (recoveredWork.value(u"title"_s).toString() == u"Original source recovery"),
        u"Manual retry reset the current work, exact variant or filter, or accepted an unrelated publication"_s);
    recoveryDialog->close();
    checks.append(u"manual source retry updates the currently selected card without reselecting; exact selection and filters survive, unrelated publication updates are ignored"_s);

    setBrowserFixture({{u"installed"_s, false}, {u"ready"_s, false}, {u"failNextConnect"_s, true}});
    searchFor(u"browser-prerequisite"_s);
    choose(episode, QJsonObject {{u"episode"_s, u"2"_s}, {u"season"_s, u""_s}});
    choose(voice, u"Studio B"_s);
    choose(language, u"ru"_s);
    filter->setText(u"Studio B"_s);
    require(variants->model()->rowCount() == 1, u"Browser prerequisite selection was not filtered"_s);
    const QJsonObject browserVariant = variantAt(variants, variants->currentIndex().row());
    const QJsonObject browserWork = works->currentIndex().siblingAtColumn(0).data(Qt::UserRole).toJsonObject();
    const auto countBrowserEvents = [&](const QString &name, const QString &sourceId)
    {
        int count = 0;
        for (const QJsonValue &value : fixtureEvents(logPath))
        {
            const QJsonObject event = value.toObject();
            if ((event.value(u"event"_s).toString() == name)
                && (event.value(u"sourceId"_s).toString() == sourceId))
            {
                ++count;
            }
        }
        return count;
    };
    const int originalConnects = countBrowserEvents(u"connectBrowser"_s, u"edge"_s);
    open->click();
    QWidget *prerequisite = nullptr;
    waitFor(u"content-area browser prerequisite"_s, [&]
    {
        prerequisite = library->findChild<QWidget *>(u"contentBrowserPrerequisite"_s);
        return prerequisite && prerequisite->isVisible();
    });
    auto *browserInstall = prerequisite->findChild<QPushButton *>(u"contentBrowserInstall"_s);
    auto *browserBack = prerequisite->findChild<QPushButton *>(u"contentBrowserBack"_s);
    auto *browserStatus = prerequisite->findChild<QLabel *>(u"contentBrowserStatus"_s);
    auto *browserDirectory = prerequisite->findChild<QLineEdit *>(u"contentBrowserExtensionDirectory"_s);
    require(browserInstall && browserBack && browserStatus && browserDirectory,
        u"Browser prerequisite controls are missing"_s);
    waitFor(u"fresh prerequisite installation action"_s, [&]
    {
        return browserInstall->isEnabled() && (browserInstall->text() == u"Install extension");
    });
    const auto capturePrerequisite = [&](const QString &state)
    {
        const QSize originalSize = window->size();
        const ColorScheme originalScheme = UIThemeManager::instance()->colorScheme();
        const QString locale = (browserBack->text() == u"Назад") ? u"ru"_s : u"en"_s;
        for (const ColorScheme scheme : {ColorScheme::Light, ColorScheme::Dark})
        {
            UIThemeManager::instance()->previewColorScheme(scheme);
            for (const QSize size : {QSize(1280, 800), QSize(800, 600).expandedTo(window->minimumSize())})
            {
                window->resize(size);
                QCoreApplication::processEvents();
                auto *body = prerequisite->findChild<QWidget *>(u"contentBrowserPrerequisiteBody"_s);
                require(body && body->parentWidget()->layout(), u"Browser prerequisite body has no parent layout"_s);
                const auto headings = body->findChildren<QLabel *>(QString(), Qt::FindDirectChildrenOnly);
                require((headings.size() == 1) && headings.first()->font().bold()
                    && headings.first()->text().contains(u"VK Video") && headings.first()->wordWrap(),
                    u"Browser prerequisite must have one source-specific heading"_s);
                const QMargins margins = body->parentWidget()->layout()->contentsMargins();
                const int availableWidth = body->parentWidget()->width() - margins.left() - margins.right();
                require(body->width() == qMin(body->maximumWidth(), availableWidth),
                    u"Browser prerequisite body shrank to its size hint instead of the available width"_s);
                require(body->height() >= body->heightForWidth(body->width()),
                    u"Browser prerequisite body compressed its wrapped content"_s);
                for (auto *button : prerequisite->findChildren<QPushButton *>())
                {
                    if (button->isVisible())
                    {
                        require((button->height() >= button->sizeHint().height())
                            && (button->width() >= button->sizeHint().width()),
                            u"Browser prerequisite clipped a button below its style size hint"_s);
                    }
                }
                for (auto *label : prerequisite->findChildren<QLabel *>())
                {
                    if (label->isVisible() && !label->text().isEmpty() && label->wordWrap())
                    {
                        require(label->height() >= label->heightForWidth(label->width()),
                            u"Browser prerequisite clipped wrapped text"_s);
                    }
                }
                const QString theme = (scheme == ColorScheme::Dark) ? u"dark"_s : u"light"_s;
                const QString width = (size.width() == 1280) ? u"normal"_s : u"minimum"_s;
                const QString path = renderDirectory.filePath(u"browser-prerequisite-%1-%2-%3-%4.png"_s.arg(locale, state, theme, width));
                require(window->grab().save(path), u"Unable to retain browser prerequisite render"_s);
                renders.append(path);
            }
        }
        UIThemeManager::instance()->previewColorScheme(originalScheme);
        window->resize(originalSize);
        QCoreApplication::processEvents();
        evidence[u"renders"_s] = renders;
    };
    capturePrerequisite(u"install"_s);
    require(!query->isVisible() && !open->isVisible()
        && (countBrowserEvents(u"connectBrowser"_s, u"edge"_s) == originalConnects),
        u"Prerequisite left footer actions visible or opened the browser without a user action"_s);
    browserBack->click();
    require(!prerequisite->isVisible() && query->isVisible()
        && (variantAt(variants, variants->currentIndex().row()) == browserVariant)
        && (works->currentIndex().siblingAtColumn(0).data(Qt::UserRole).toJsonObject() == browserWork)
        && (filter->text() == u"Studio B") && (voice->currentData().toString() == u"Studio B")
        && (language->currentData().toString() == u"ru")
        && (episode->currentData().toJsonObject().value(u"episode"_s).toString() == u"2"),
        u"Back from prerequisite changed the work, exact variant or filters"_s);

    open->click();
    waitFor(u"prerequisite re-entry"_s, [&] { return prerequisite->isVisible() && browserInstall->isEnabled(); });
    browserInstall->click();
    waitFor(u"inline installation failure"_s, [&]
    {
        return (browserInstall->text() == u"Try again") && browserInstall->isEnabled()
            && browserStatus->text().contains(u"Original setup failure");
    });
    require(prerequisite->isVisible() && !query->isVisible(), u"Setup failure escaped the prerequisite page"_s);
    capturePrerequisite(u"failure"_s);
    browserBack->click();
    setBrowserFixture({{u"installed"_s, true}});
    open->click();
    waitFor(u"reuse installed qcatch"_s, [&]
    {
        return prerequisite->isVisible() && browserInstall->isEnabled()
            && (browserInstall->text() == u"Connect extension");
    });
    browserInstall->click();
    waitFor(u"manual extension connection prerequisite"_s, [&]
    {
        return browserDirectory->isVisible() && browserInstall->isEnabled();
    });
    require((browserStatus->text() == u"Waiting for the extension…")
        && (countBrowserEvents(u"resolve"_s, u"fixture-browser"_s) == 3),
        u"Installer completion falsely resumed playback before browser readiness"_s);
    capturePrerequisite(u"waiting"_s);
    browserBack->click();
    setBrowserFixture({{u"ready"_s, true}});
    QElapsedTimer hiddenPoll;
    hiddenPoll.start();
    waitFor(u"late browser readiness after Back"_s, [&] { return hiddenPoll.elapsed() >= 2200; });
    require(!prerequisite->isVisible() && (countBrowserEvents(u"resolve"_s, u"fixture-browser"_s) == 3),
        u"Hidden prerequisite resumed the abandoned action"_s);

    setBrowserFixture({{u"ready"_s, false}});
    open->click();
    waitFor(u"fresh connection after Back"_s, [&]
    {
        return prerequisite->isVisible() && browserInstall->isEnabled()
            && (browserInstall->text() == u"Connect extension") && !browserDirectory->isVisible();
    });
    browserInstall->click();
    waitFor(u"waiting for verified browser status"_s, [&] { return browserDirectory->isVisible(); });
    setBrowserFixture({{u"ready"_s, true}});
    waitFor(u"exact action resumed after verified browser readiness"_s, [&]
    {
        return !prerequisite->isVisible() && (countBrowserEvents(u"resolve"_s, u"fixture-browser"_s) == 5);
    });
    for (const QJsonValue &value : fixtureEvents(logPath))
    {
        const QJsonObject event = value.toObject();
        if ((event.value(u"event"_s).toString() != u"resolve")
            || (event.value(u"sourceId"_s).toString() != u"fixture-browser"))
        {
            continue;
        }
        const QJsonObject params = event.value(u"params"_s).toObject();
        require((params.value(u"work"_s).toObject() == browserWork)
            && (params.value(u"variant"_s).toObject() == browserVariant),
            u"Browser connection resumed a different work or exact publication"_s);
    }
    require((countBrowserEvents(u"connectBrowser"_s, u"edge"_s) == originalConnects + 3)
        && (variantAt(variants, variants->currentIndex().row()) == browserVariant)
        && (filter->text() == u"Studio B"), u"Connection retry repeated setup or reset exact selection"_s);
    checks.append(u"missing qcatch replaces the content area; Back retains exact work, episode, voice, language and text filter"_s);
    checks.append(u"setup failure stays inline; installed qcatch offers connection; setup exit alone never resumes playback"_s);
    checks.append(u"late readiness after Back is ignored; verified readiness resumes exactly the original publication once"_s);
    checks.append(u"browser prerequisite uses available width and preserves button and wrapped-text heights at normal and minimum sizes in Light and Dark"_s);

    // Decode a generated WAV with actual packaged mpv. This exercises the
    // service signal, Library association and stdio history write together.
    auto *player = Media::Player::instance();
    QObject observations;
    QStringList started;
    QStringList errors;
    QStringList preparationMessages;
    QElapsedTimer playbackElapsed;
    playbackElapsed.start();
    const auto recordPlayback = [&](const QString &event, const QJsonValue &data)
    {
        QJsonArray playbackEvents = evidence.value(u"playbackEvents"_s).toArray();
        playbackEvents.append(QJsonObject {{u"event"_s, event}, {u"elapsedMs"_s, playbackElapsed.elapsed()}, {u"data"_s, data}});
        evidence[u"playbackEvents"_s] = playbackEvents;
    };
    QObject::disconnect(player, &Media::Player::error, window, nullptr);
    QObject::connect(player, &Media::Player::playbackStarted, &observations,
        [&](const QString &identity)
    {
        started.append(identity);
        recordPlayback(u"started"_s, identity);
    });
    QObject::connect(player, &Media::Player::error, &observations,
        [&](const QString &message)
    {
        errors.append(message);
        recordPlayback(u"error"_s, message);
    });
    QObject::connect(player, &Media::Player::preparationChanged, &observations,
        [&](const QString &message)
    {
        preparationMessages.append(message);
        recordPlayback(u"preparation"_s, message);
    });
    QObject::connect(player, &Media::Player::tracksChanged, &observations,
        [&](const QJsonArray &tracks) { recordPlayback(u"tracks"_s, tracks); });
    const auto contentBarrier = [&]
    {
        bool completed = false;
        QJsonArray entries;
        const qint64 request = Content::Client::instance()->request(u"library"_s);
        const auto connection = QObject::connect(Content::Client::instance(), &Content::Client::eventReceived,
            &observations, [&](const qint64 id, const QString &event, const QJsonValue &data)
        {
            if (id == request)
            {
                if (event == u"library")
                {
                    for (const QJsonValue &entry : data.toArray())
                        entries.append(entry);
                }
                else if (event == u"complete")
                    completed = true;
            }
        });
        waitFor(u"content history barrier"_s, [&] { return completed; });
        QObject::disconnect(connection);
        return entries;
    };

    Media::PlaybackOptions cancelled;
    cancelled.identity = u"cancelled-preparation"_s;
    require(player->playUrl(QUrl::fromLocalFile(spec.value(u"mediaPath"_s).toString()), u"Cancelled preparation"_s, cancelled),
        u"Player rejected the preparation fixture"_s);
    waitFor(u"native bundled runtime verification"_s, [&]
    {
        const auto *dialog = window->findChild<QProgressDialog *>(u"playerPreparationDialog"_s);
        return dialog && dialog->isVisible() && !preparationMessages.isEmpty()
            && !preparationMessages.first().isEmpty() && (dialog->labelText() == preparationMessages.first())
            && hasFixtureEvent(logPath, u"mediaRuntime"_s, u""_s);
    });
    auto *preparation = window->findChild<QProgressDialog *>(u"playerPreparationDialog"_s);
    require((preparation->windowModality() == Qt::NonModal)
        && (preparation->minimum() == 0) && (preparation->maximum() == 0),
        u"Bundled runtime verification is blocking or reports fabricated percentage progress"_s);
    auto *cancel = preparation->findChild<QPushButton *>();
    require(cancel && cancel->isEnabled(), u"Preparation cancellation is unavailable"_s);
    qint64 preparationId = 0;
    for (const QJsonValue &value : fixtureEvents(logPath))
    {
        const QJsonObject event = value.toObject();
        if (event.value(u"event"_s).toString() == u"mediaRuntime")
            preparationId = event.value(u"id"_s).toInteger();
    }
    require(preparationId > 0, u"Preparation did not use Content Client IPC"_s);
    cancel->click();
    waitFor(u"preparation cancelled"_s, [&]
    {
        for (const QJsonValue &value : fixtureEvents(logPath))
        {
            const QJsonObject event = value.toObject();
            if ((event.value(u"event"_s).toString() == u"cancel")
                && (event.value(u"id"_s).toInteger() == preparationId))
            {
                const auto *dialog = window->findChild<QProgressDialog *>(u"playerPreparationDialog"_s);
                return !dialog || !dialog->isVisible();
            }
        }
        return false;
    });
    waitFor(u"late preparation reply"_s, [&] { return hasFixtureEvent(logPath, u"runtimeReady"_s, u""_s); });
    contentBarrier();
    require((preparationMessages.size() == 2) && preparationMessages.last().isEmpty()
        && started.isEmpty() && errors.isEmpty() && !hasFixtureEvent(logPath, u"remember"_s, u""_s),
        u"Cancelled preparation started playback or wrote history"_s);
    checks.append(u"modeless bundled runtime verification Cancel clears request and ignores late readiness"_s);

    recordPlayback(u"request"_s, u"Local file"_s);
    require(player->playUrl(QUrl::fromLocalFile(spec.value(u"mediaPath"_s).toString()), u"Local file"_s),
        u"Actual player rejected the generated local audio"_s);
    waitFor(u"local file decoding"_s, [&] { return !started.isEmpty() || !errors.isEmpty(); });
    require(errors.isEmpty() && (started == QStringList {QString()}), u"Local file did not emit its empty playback identity"_s);
    contentBarrier();
    require(!hasFixtureEvent(logPath, u"remember"_s, u""_s), u"Local file wrote an empty Library item"_s);
    player->stop();
    checks.append(u"actual local Player start never writes an empty Library history item"_s);

    started.clear();
    searchFor(u"media-start"_s);
    waitFor(u"media version details"_s, [&] { return open->isEnabled(); });
    require(variants->model()->rowCount() == 3, u"Playback fixture lost its complete work context"_s);
    auto *views = library->findChild<QTabBar *>();
    require(views, u"Library history navigation is missing"_s);
    const QJsonObject snapshotWork = works->currentIndex().siblingAtColumn(0).data(Qt::UserRole).toJsonObject();
    Content::Client::instance()->request(u"remember"_s,
        {{u"work"_s, snapshotWork}, {u"variant"_s, variantAt(variants, variants->currentIndex().row())}, {u"favorite"_s, false}});
    contentBarrier();
    Content::Client::instance()->request(u"holdLibrarySnapshot"_s, {{u"workId"_s, u"media-start"_s}});
    views->setCurrentIndex(1);
    waitFor(u"older library snapshot held"_s, [&] { return hasFixtureEvent(logPath, u"librarySnapshotHeld"_s, u"media-start"_s); });
    views->setCurrentIndex(0);
    require(favorite->isEnabled() && !favorite->isChecked(), u"Held library snapshot changed the search card"_s);
    favorite->click();
    contentBarrier();
    require(favorite->isChecked(), u"Favorite completion did not update the current search card"_s);
    Content::Client::instance()->request(u"advanceFixture"_s, {{u"query"_s, u"library-snapshot"_s}});
    waitFor(u"older library snapshot released"_s, [&] { return hasFixtureEvent(logPath, u"librarySnapshotReleased"_s, u"media-start"_s); });
    contentBarrier();
    require(favorite->isChecked()
        && (works->currentIndex().siblingAtColumn(0).data(Qt::UserRole).toJsonObject().value(u"id"_s).toString() == u"media-start"),
        u"An older library snapshot overwrote the newer favorite or current search card"_s);
    favorite->click();
    contentBarrier();
    require(!favorite->isChecked(), u"Favorite snapshot fixture did not restore its initial state"_s);
    checks.append(u"newer favorite remains authoritative after a deliberately late older library snapshot"_s);
    choose(episode, QJsonObject {{u"episode"_s, u"2"_s}, {u"season"_s, u""_s}});
    choose(voice, u"Studio B"_s);
    choose(language, u"ru"_s);
    require((source->currentIndex() == 0) && (variants->model()->rowCount() == 2),
        u"History fixture must exercise an explicit All sources choice and colliding publication IDs"_s);
    variants->selectRow(variantRow(variants, u"media"_s, false));
    open->click();
    waitFor(u"Library playback history"_s, [&]
    {
        for (const QJsonValue &value : fixtureEvents(logPath))
        {
            const QJsonObject event = value.toObject();
            if ((event.value(u"event"_s).toString() == u"remember")
                && (event.value(u"sourceId"_s).toString() == u"media-start")
                && event.value(u"params"_s).toObject().value(u"opened"_s).toBool())
            {
                return true;
            }
        }
        return false;
    });
    require(errors.isEmpty() && (started == QStringList {u"video:media:accepted"_s}),
        u"Library history was not associated with the actual playback start"_s);

    started.clear();
    searchFor(u"media-reject"_s);
    waitFor(u"rejected voice details"_s, [&] { return open->isEnabled(); });
    open->click();
    waitFor(u"actual unavailable voice rejection"_s, [&] { return !errors.isEmpty(); });
    require(started.isEmpty() && !hasFixtureEvent(logPath, u"remember"_s, u"media-reject"_s),
        u"Unavailable voice wrote a false history item"_s);
    views->setCurrentIndex(2);
    waitFor(u"history rows"_s, [&]
    {
        return (works->model()->rowCount() == 1)
            && (works->model()->index(0, 0).data(Qt::UserRole).toJsonObject().value(u"id"_s).toString() == u"media-start");
    });
    require((variants->model()->rowCount() == 2)
        && (variantAt(variants, variants->currentIndex().row()).value(u"sourceId"_s).toString() == u"media")
        && (variantAt(variants, variants->currentIndex().row()).value(u"id"_s).toString() == u"accepted")
        && (episode->currentData().toJsonObject().value(u"episode"_s).toString() == u"2")
        && (voice->currentData().toString() == u"Studio B") && (language->currentData().toString() == u"ru")
        && (source->currentIndex() == 0)
        && (works->model()->index(0, 0).data().toString() == u"Original complete series"),
        u"History did not restore canonical work, exact source publication and saved dropdown choices"_s);
    const auto searchesBeforeHistoryChange = fixtureEvents(logPath);
    episode->setCurrentIndex(0);
    voice->setCurrentIndex(0);
    language->setCurrentIndex(0);
    require(variants->model()->rowCount() == 3, u"History froze the work to the last played edition"_s);
    choose(episode, QJsonObject {{u"episode"_s, u"1"_s}, {u"season"_s, u""_s}});
    choose(voice, u"Studio A"_s);
    choose(language, u"en"_s);
    choose(source, QJsonObject {{u"sourceId"_s, u"media"_s}, {u"player"_s, u""_s}, {u"transport"_s, u"video"_s}});
    require((variants->model()->rowCount() == 1) && open->isEnabled()
        && (variantAt(variants, 0).value(u"id"_s).toString() == u"alternate"),
        u"History cannot select another episode, voice and language from the known work"_s);
    require(!favorite->isChecked(), u"History favorite fixture is already saved"_s);
    int favoriteSelectionChanges = 0;
    QJsonArray savedHistory;
    {
        QObject receiver;
        QObject::connect(works->selectionModel(), &QItemSelectionModel::currentRowChanged, &receiver,
            [&] { ++favoriteSelectionChanges; });
        favorite->click();
        savedHistory = contentBarrier();
    }
    QJsonObject savedEntry;
    for (const QJsonValue &value : savedHistory)
    {
        const QJsonObject entry = value.toObject();
        if (entry.value(u"work"_s).toObject().value(u"id"_s).toString() == u"media-start")
        {
            require(savedEntry.isEmpty(), u"History contains duplicate entries for the played work"_s);
            savedEntry = entry;
        }
    }
    const QJsonObject savedVariant = savedEntry.value(u"variant"_s).toObject();
    const QJsonObject expectedFilters {
        {u"episode"_s, QJsonObject {{u"episode"_s, u"2"_s}, {u"season"_s, u""_s}}},
        {u"translation"_s, u"Studio B"_s},
        {u"language"_s, u"ru"_s}
    };
    require(savedEntry.value(u"favorite"_s).toBool() && !savedEntry.value(u"lastOpened"_s).toString().isEmpty()
        && (savedVariant.value(u"sourceId"_s).toString() == u"media")
        && (savedVariant.value(u"id"_s).toString() == u"accepted")
        && (savedEntry.value(u"variantFilters"_s).toObject() == expectedFilters)
        && (savedEntry.value(u"work"_s).toObject().value(u"variants"_s).toArray().size() == 3),
        u"Favorite changed the exact last played variant or playback dropdown defaults: %1"_s
            .arg(QString::fromUtf8(QJsonDocument(savedHistory).toJson(QJsonDocument::Compact))));
    require(favorite->isChecked(), u"Favorite bookmark did not reflect the stored entry"_s);
    require((favoriteSelectionChanges == 0)
        && (works->currentIndex().siblingAtColumn(0).data(Qt::UserRole).toJsonObject().value(u"id"_s).toString() == u"media-start")
        && (variants->model()->rowCount() == 1)
        && (variantAt(variants, variants->currentIndex().row()).value(u"sourceId"_s).toString() == u"media")
        && (variantAt(variants, variants->currentIndex().row()).value(u"id"_s).toString() == u"alternate")
        && (episode->currentData().toJsonObject().value(u"episode"_s).toString() == u"1")
        && (voice->currentData().toString() == u"Studio A") && (language->currentData().toString() == u"en")
        && (source->currentData().toJsonObject().value(u"sourceId"_s).toString() == u"media"),
        u"Saving a favorite reset the current work, exact browsed edition or dropdown choices"_s);
    checkFavorite();
    for (const QJsonValue &value : fixtureEvents(logPath))
    {
        if (value.toObject().value(u"event"_s).toString() == u"search")
            require(searchesBeforeHistoryChange.contains(value), u"Changing History dropdowns started a global search"_s);
    }
    views->setCurrentIndex(1);
    views->setCurrentIndex(2);
    waitFor(u"History restored after browsing other editions"_s, [&]
    {
        return (variants->model()->rowCount() == 2) && (source->currentIndex() == 0)
            && (voice->currentData().toString() == u"Studio B") && (language->currentData().toString() == u"ru")
            && (variantAt(variants, variants->currentIndex().row()).value(u"sourceId"_s).toString() == u"media")
            && (variantAt(variants, variants->currentIndex().row()).value(u"id"_s).toString() == u"accepted");
    });
    capture(u"library-history-full-work-light-minimum"_s, {800, 600}, ColorScheme::Light);
    capture(u"library-favorite-active-dark-minimum"_s, {800, 600}, ColorScheme::Dark);
    require(views->tabText(1) == Content::LibraryWidget::tr("Favorites"),
        u"The library tab is not named Favorites"_s);
    favorite->click();
    contentBarrier();
    require(!favorite->isChecked(), u"Favorite bookmark did not remove the stored favorite"_s);
    checkFavorite();
    views->setCurrentIndex(1);
    contentBarrier();
    require(works->model()->rowCount() == 0, u"Removed favorite remains in the Favorites view"_s);
    views->setCurrentIndex(2);
    waitFor(u"removing favorite preserves played work in History"_s, [&]
    {
        return (works->model()->rowCount() == 1)
            && (variantAt(variants, variants->currentIndex().row()).value(u"id"_s).toString() == u"accepted")
            && (voice->currentData().toString() == u"Studio B");
    });
    evidence[u"renders"_s] = renders;
    checks.append(u"History restores full canonical work, exact played source and durable dropdowns; other editions require no search"_s);
    checks.append(u"actual Library Player start writes history; unavailable voice leaves history unchanged"_s);
    player->stop();

    QImage original(64, 96, QImage::Format_RGB32);
    original.fill(Qt::white);
    require(original.save(renderDirectory.filePath(u"reader-original.png"_s)), u"Unable to create legal Reader image"_s);
    for (const QString &workId : {u"reader-images"_s, u"reader-text"_s})
    {
        const bool images = workId == u"reader-images";
        searchFor(workId);
        waitFor(u"online Reader version details"_s, [&] { return open->isEnabled(); });
        checkVariantIcons();
        open->click();
        auto *reader = library->findChild<Content::ReaderWidget *>();
        waitFor(u"online Reader page"_s, [&]
        {
            reader = library->findChild<Content::ReaderWidget *>();
            if (!reader || !library->isReading())
                return false;
            const auto *next = reader->findChild<QToolButton *>(u"readerNext"_s);
            const auto *text = reader->findChild<QTextBrowser *>(u"readerText"_s);
            return images ? next && next->isEnabled()
                : text && text->isVisible() && (text->verticalScrollBar()->maximum() > 0);
        });
        contentBarrier();
        double offset = 0;
        if (images)
        {
            reader->findChild<QToolButton *>(u"readerNext"_s)->click();
            const auto *number = reader->findChild<QLabel *>(u"readerPageNumber"_s);
            require(number && (number->text() == u"2 / 3"), u"Online comic did not turn to its second image"_s);
        }
        else
        {
            auto *scroll = reader->findChild<QTextBrowser *>(u"readerText"_s)->verticalScrollBar();
            scroll->setValue(scroll->maximum() / 2);
            offset = double(scroll->value()) / scroll->maximum();
            require(offset > 0.4, u"Text fixture did not reach a meaningful reading position"_s);
        }
        reader->findChild<QToolButton *>(u"readerBack"_s)->click();
        require(!library->isReading(), u"Reader Back did not return to the Library"_s);
        const QJsonArray entries = contentBarrier();
        QJsonObject saved;
        for (const QJsonValue &value : entries)
        {
            const QJsonObject entry = value.toObject();
            if (entry.value(u"work"_s).toObject().value(u"id"_s).toString() == workId)
                saved = entry.value(u"progress"_s).toObject();
        }
        require((saved.value(u"chapterId"_s).toString() == workId + u":chapter"_s)
            && !saved.contains(u"documentId"_s) && !saved.contains(u"workId"_s),
            u"Reader position lost its chapter or persisted transport identities"_s);
        require(saved.contains(u"offset"_s) && (saved.value(u"offset"_s).toDouble() == offset)
            && (images ? saved.value(u"imageIndex"_s).toInt(-1) == 1 : !saved.contains(u"imageIndex"_s)),
            u"Library changed the actual image or text reading position: expected offset %1, saved %2"_s
                .arg(offset).arg(QString::fromUtf8(QJsonDocument(saved).toJson(QJsonDocument::Compact))));
        bool sameReaderPosition = false;
        for (const QJsonValue &value : fixtureEvents(logPath))
        {
            const QJsonObject event = value.toObject();
            QJsonObject params = event.value(u"params"_s).toObject();
            if ((event.value(u"event"_s).toString() == u"reader.position")
                && (event.value(u"sourceId"_s).toString() == workId)
                && (params.value(u"chapterId"_s) == saved.value(u"chapterId"_s)))
            {
                params.remove(u"documentId"_s);
                params.remove(u"workId"_s);
                sameReaderPosition |= params == saved;
            }
        }
        require(sameReaderPosition, u"Reader and Library persisted different reading positions"_s);
    }
    checks.append(u"Online Library Reader preserves the exact chapter and offset, with an image index only for manga"_s);
    evidence[u"checks"_s] = checks;
    evidence[u"fixtureEvents"_s] = fixtureEvents(logPath);
}
