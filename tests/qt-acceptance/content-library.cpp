// SPDX-License-Identifier: GPL-2.0-or-later
#include "content-library.h"

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
#include <QHeaderView>
#include <QImage>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLabel>
#include <QLineEdit>
#include <QListWidget>
#include <QProgressDialog>
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
#include "gui/content/librarywidget.h"
#include "gui/content/readerwidget.h"
#include "gui/content/sourcesdialog.h"
#include "gui/mainwindow.h"
#include "gui/uithememanager.h"

namespace
{
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
        return table->model()->index(row, 0).data(Qt::UserRole).toJsonObject();
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
}

void exerciseContentLibrary(MainWindow *window, const QJsonObject &spec, QJsonObject &evidence)
{
    auto *library = window->findChild<Content::LibraryWidget *>();
    require(library, u"Native content library is missing"_s);
    auto *query = library->findChild<QLineEdit *>(u"contentQuery"_s);
    auto *search = library->findChild<QPushButton *>(u"contentSearch"_s);
    auto *works = library->findChild<QTableView *>(u"contentResults"_s);
    auto *variants = library->findChild<QTableView *>(u"contentVariants"_s);
    auto *filter = library->findChild<QLineEdit *>(u"contentVariantFilter"_s);
    auto *episode = library->findChild<QComboBox *>(u"contentEpisode"_s);
    auto *voice = library->findChild<QComboBox *>(u"contentVoice"_s);
    auto *source = library->findChild<QComboBox *>(u"contentVariantSource"_s);
    auto *ads = library->findChild<QCheckBox *>(u"contentIncludeReportedAds"_s);
    auto *notes = library->findChild<QTextBrowser *>(u"contentNotes"_s);
    auto *open = library->findChild<QPushButton *>(u"contentOpen"_s);
    require(ads, u"Native reported-ad filter is missing"_s);
    require(notes, u"Native source comment view is missing"_s);
    require(query && search && works && variants && filter && open && episode && voice && source,
        u"Native content controls are missing"_s);

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
        search->click();
        waitFor(u"search %1"_s.arg(text), [&] { return finished; });
        QObject::disconnect(connection);
        require(!failed && (works->model()->rowCount() == 1), u"Fixture search failed: %1"_s.arg(text));
    };

    bool rankingFinished = false;
    const auto rankedSearch = QObject::connect(library, &Content::LibraryWidget::searchFinished,
        library, [&](bool) { rankingFinished = true; });
    query->setText(u"ranking"_s);
    search->click();
    waitFor(u"initial low relevance result"_s, [&] { return works->model()->rowCount() == 1; });
    works->selectRow(0);
    require((works->model()->columnCount() == 1) && works->horizontalHeader()->isHidden(),
        u"Search results still expose duplicated category and version columns"_s);
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
    waitFor(u"canonical result replacement"_s, [&]
    {
        return works->model()->index(0, 0).data(Qt::UserRole).toJsonObject().value(u"id"_s).toString()
            == u"canonical-promoted";
    });
    require(!promotionFinished && !open->isVisible() && (variants->model()->rowCount() == 0),
        u"Removed publication kept stale version actions while search continued"_s);
    waitFor(u"canonical promotion completion"_s, [&] { return promotionFinished && open->isEnabled(); });
    QObject::disconnect(promotedSearch);
    checks.append(u"canonical result replacement clears removed publication actions before search completes"_s);

    searchFor(u"games-recovery"_s);
    QLabel *inspectionStatus = nullptr;
    waitFor(u"failed initial publication inspection"_s, [&]
    {
        for (QLabel *label : library->findChildren<QLabel *>())
        {
            if (label->text() == u"Source A requires browser verification (HTTP 403).")
            {
                inspectionStatus = label;
                return true;
            }
        }
        return false;
    });
    require(inspectionStatus->isVisible(), u"Initial publication error was not shown"_s);
    const int workingPublication = variantRow(variants, u"fixture-b"_s, true);
    require(workingPublication >= 0, u"Working publication placeholder is missing"_s);
    variants->selectRow(workingPublication);
    waitFor(u"working publication after inspection error"_s, [&]
    {
        return (variantRow(variants, u"fixture-b"_s, false) >= 0) && open->isEnabled();
    });
    require(notes->toPlainText().contains(u"Install the game, then apply the cooperative patch."),
        u"Working publication instructions were not loaded"_s);
    require(inspectionStatus->text().isEmpty(), u"Successful publication retained the previous inspection error"_s);
    checks.append(u"successful publication inspection clears the prior source error and keeps working instructions"_s);

    // Change publication while the first inspection is in flight. The second
    // response deliberately predates a third streamed publication.
    bool gamesFinished = false;
    const QMetaObject::Connection gamesSearch = QObject::connect(library, &Content::LibraryWidget::searchFinished,
        library, [&](bool) { gamesFinished = true; });
    query->setText(u"games"_s);
    search->click();
    waitFor(u"initial publications"_s, [&] { return works->model()->rowCount() == 1; });
    works->selectRow(0);
    waitFor(u"first publication request"_s, [&] { return hasFixtureEvent(logPath, u"inspect"_s, u"fixture-a"_s); });
    const int second = variantRow(variants, u"fixture-b"_s, true);
    require(second >= 0, u"Second publication placeholder is missing"_s);
    variants->selectRow(second);
    require(!open->isEnabled(), u"Placeholder became downloadable during inspection"_s);
    waitFor(u"second publication expansion"_s, [&]
    {
        return (variantRow(variants, u"fixture-b"_s, false) >= 0)
            && (variantRow(variants, u"fixture-c"_s, true) >= 0) && open->isEnabled();
    });
    require(variantRow(variants, u"fixture-a"_s, true) >= 0,
        u"Cancelled inspection replaced a different selected publication"_s);
    require(variantAt(variants, variants->currentIndex().row()).value(u"sourceId"_s).toString() == u"fixture-b",
        u"Inspection changed the selected publication"_s);
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
    require(variants->isVisible() && open->isVisible(), u"Returning from details lost the version actions"_s);
    variants->selectRow(variantRow(variants, u"fixture-a"_s, true));
    waitFor(u"first publication expansion"_s, [&]
    {
        return (variantRow(variants, u"fixture-a"_s, false) >= 0) && open->isEnabled();
    });
    require((variantRow(variants, u"fixture-b"_s, false) >= 0)
        && (variantRow(variants, u"fixture-c"_s, true) >= 0),
        u"Source switching lost expanded or late streamed variants"_s);
    filter->setText(u"Source B patch"_s);
    require((variants->model()->rowCount() == 1)
        && (variantAt(variants, 0).value(u"sourceId"_s).toString() == u"fixture-b"),
        u"Version filter does not match words across source and release"_s);
    require(notes->toPlainText().contains(u"Author A") && notes->toPlainText().contains(u"Author B")
        && notes->toHtml().contains(u"https://fixture-a.example/game#comment-id-1")
        && notes->toHtml().contains(u"https://fixture-b.example/game#comment-id-1"),
        u"Switching an expanded publication lost work comments or their source attribution"_s);
    filter->clear();
    waitFor(u"game search completion"_s, [&] { return gamesFinished; });
    QObject::disconnect(gamesSearch);
    require(hasFixtureEvent(logPath, u"details"_s, u"fixture-a"_s), u"Stale inspection fault was not exercised"_s);
    checks.append(u"publication switch, stale inspection, streamed merge, source comments"_s);

    searchFor(u"game-downloads"_s);
    waitFor(u"distinct game download choices"_s, [&] { return (variants->model()->rowCount() == 5) && open->isEnabled(); });
    QSet<QString> downloadLabels;
    for (int row = 0; row < variants->model()->rowCount(); ++row)
        downloadLabels.insert(variants->model()->index(row, 1).data().toString());
    const QString visibleDownloads = QStringList(downloadLabels.cbegin(), downloadLabels.cend()).join(u'\n');
    require((downloadLabels.size() == 5) && visibleDownloads.contains(u".torrent")
        && visibleDownloads.contains(u".rar") && visibleDownloads.contains(u"Hosters")
        && visibleDownloads.contains(u"Drive") && visibleDownloads.contains(u"Fix"),
        u"Torrent, archive, hosting and fix choices are indistinguishable"_s);
    require(variants->horizontalHeader()->isHidden(), u"Text-only versions retain duplicated headers"_s);
    require(!episode->isVisible() && !voice->isVisible() && !source->isVisible(),
        u"Non-episodic game downloads expose irrelevant episode pickers"_s);
    checks.append(u"five game downloads remain visibly distinct: torrent, archive, Hosters, Drive, fix"_s);

    const QSize previousSize = window->size();
    const ColorScheme previousScheme = UIThemeManager::instance()->colorScheme();
    const QDir renderDirectory(QFileInfo(spec.value(u"evidencePath"_s).toString()).absolutePath());
    QJsonArray renders;
    const auto capture = [&](const QString &name, const QSize &size, const ColorScheme scheme)
    {
        UIThemeManager::instance()->previewColorScheme(scheme);
        window->resize(size);
        QCoreApplication::processEvents();
        require(variants->viewport()->height() >= variants->verticalHeader()->defaultSectionSize() * 2,
            u"Loaded layout leaves fewer than two version rows visible"_s);
        require((open->width() >= open->minimumSizeHint().width())
            && window->rect().contains(open->mapTo(window, open->rect().bottomRight())),
            u"Primary content action is clipped"_s);
        for (QComboBox *combo : {episode, voice, source})
        {
            if (combo->isVisible())
            {
                require((combo->width() >= combo->minimumSizeHint().width())
                    && window->rect().contains(combo->mapTo(window, combo->rect().bottomRight())),
                    u"Episode, voice or source picker is clipped"_s);
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

    searchFor(u"table-metadata"_s);
    require(variants->horizontalHeader()->isVisible() && !variants->isColumnHidden(2)
        && !variants->isColumnHidden(3) && (variants->model()->index(0, 3).data().toString() == u"0"),
        u"Torrent size or zero seeders lost their comparison headers"_s);

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
    const auto choose = [](QComboBox *combo, const QVariant &data)
    {
        const int index = combo->findData(data);
        require(index > 0, u"Declared variant filter choice is missing"_s);
        combo->setCurrentIndex(index);
    };
    choose(episode, QJsonObject {{u"episode"_s, u"2"_s}, {u"season"_s, u""_s}});
    choose(voice, u"Studio B"_s);
    choose(source, QJsonObject {{u"sourceId"_s, u"fixture-yani"_s}});
    require((variants->model()->rowCount() == 1) && !open->isEnabled()
        && variantAt(variants, 0).value(u"needsInspection"_s).toBool()
        && variants->model()->index(0, 1).data().toString().contains(u"Show episodes"),
        u"Episode and voice filters hid or automatically selected an unexpanded publication"_s);
    variants->selectRow(0);
    waitFor(u"filtered Yani publication expansion"_s, [&]
    {
        return (variants->model()->rowCount() == 4) && open->isEnabled()
            && !variantAt(variants, 0).value(u"needsInspection"_s).toBool();
    });
    require((episode->currentData().toJsonObject().value(u"episode"_s).toString() == u"2")
        && (voice->currentData().toString() == u"Studio B")
        && (source->currentData().toJsonObject() == QJsonObject {{u"sourceId"_s, u"fixture-yani"_s}}),
        u"Publication expansion reset the user's episode, voice or provider"_s);
    choose(source, QJsonObject {{u"sourceId"_s, u"fixture-yani"_s}, {u"player"_s, u"Kodik"_s}, {u"transport"_s, u"video"_s}});
    require((variants->model()->rowCount() == 1) && (source->currentText() == u"YummyAnime [Kodik]")
        && variantAt(variants, 0).value(u"episode"_s).toString() == u"2"
        && variantAt(variants, 0).value(u"translation"_s).toString() == u"Studio B"
        && variantAt(variants, 0).value(u"player"_s).toString() == u"Kodik"
        && variants->isColumnHidden(0) && variants->isColumnHidden(2) && variants->isColumnHidden(3)
        && variants->horizontalHeader()->isHidden(),
        u"Episode, studio and actual player did not narrow to the exact visible release"_s);
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
    require((variants->model()->rowCount() == 12) && variants->isColumnHidden(2) && variants->isColumnHidden(3),
        u"Text filtering is lost or empty torrent columns remain beside filtered video (rows=%1, size hidden=%2, seeds hidden=%3)"_s
            .arg(variants->model()->rowCount()).arg(variants->isColumnHidden(2)).arg(variants->isColumnHidden(3)));
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

    {
        Content::SourcesDialog sources(window);
        sources.show();
        auto *install = sources.findChild<QPushButton *>(u"contentBrowserInstall"_s);
        auto *instructions = sources.findChild<QLabel *>(u"contentBrowserSetup"_s);
        auto *directory = sources.findChild<QLineEdit *>(u"contentBrowserExtensionDirectory"_s);
        auto *status = sources.findChild<QLabel *>(u"contentSourcesStatus"_s);
        auto *sourceList = sources.findChild<QListWidget *>();
        require(install && instructions && directory && status && sourceList,
            u"Browser extension installation controls are missing"_s);
        waitFor(u"initial browser connection status"_s, [&] { return status->text() == u"Not connected"; });
        require(install->isVisible() && install->text().contains(u"Install browser extension"),
            u"Fresh user has no explicit extension installation action"_s);
        install->click();
        waitFor(u"browser extension setup"_s, [&] { return directory->isVisible() && install->isEnabled(); });
        require(directory->isReadOnly() && directory->isEnabled()
            && (directory->text() == u"C:\\Original qbutt\\browser-extension\\chromium")
            && instructions->wordWrap() && instructions->text().contains(u"Developer mode")
            && instructions->text().contains(u"Load unpacked")
            && instructions->text().contains(u"edge://extensions") && instructions->text().contains(u"Ctrl+L")
            && (status->text() == u"Not connected"),
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
        require(instructions->isVisible() && directory->isVisible() && (status->text() == u"Not connected"),
            u"Returning to setup erased the pending installation steps"_s);
        sources.activateWindow();
        QEvent activation(QEvent::ActivationChange);
        QCoreApplication::sendEvent(&sources, &activation);
        waitFor(u"actual connected browser name"_s, [&] { return status->text() == u"Connected to Microsoft Edge"; });
        require(!instructions->isVisible() && !directory->isVisible()
            && hasFixtureEvent(logPath, u"connectBrowser"_s, u"edge"_s),
            u"Connected browser setup did not finish after the real status response"_s);
        sources.close();
    }
    checks.append(u"extension installation has persistent manual steps and reports only authenticated browser connection"_s);

    // Decode a generated WAV with actual packaged mpv. This exercises the
    // service signal, Library association and stdio history write together.
    auto *player = Media::Player::instance();
    QObject observations;
    QStringList started;
    QStringList errors;
    QStringList preparationMessages;
    QObject::disconnect(player, &Media::Player::error, window, nullptr);
    QObject::connect(player, &Media::Player::playbackStarted, &observations,
        [&](const QString &identity) { started.append(identity); });
    QObject::connect(player, &Media::Player::error, &observations,
        [&](const QString &message) { errors.append(message); });
    QObject::connect(player, &Media::Player::preparationChanged, &observations,
        [&](const QString &message) { preparationMessages.append(message); });
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
                    entries = data.toArray();
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
    open->click();
    waitFor(u"Library playback history"_s, [&] { return hasFixtureEvent(logPath, u"remember"_s, u"media-start"_s); });
    require(errors.isEmpty() && (started == QStringList {u"media-start/accepted"_s}),
        u"Library history was not associated with the actual playback start"_s);

    started.clear();
    searchFor(u"media-reject"_s);
    waitFor(u"rejected voice details"_s, [&] { return open->isEnabled(); });
    open->click();
    waitFor(u"actual unavailable voice rejection"_s, [&] { return !errors.isEmpty(); });
    require(started.isEmpty() && !hasFixtureEvent(logPath, u"remember"_s, u"media-reject"_s),
        u"Unavailable voice wrote a false history item"_s);
    auto *views = library->findChild<QTabBar *>();
    require(views, u"Library history navigation is missing"_s);
    views->setCurrentIndex(2);
    waitFor(u"history rows"_s, [&]
    {
        return (works->model()->rowCount() == 1)
            && (works->model()->index(0, 0).data(Qt::UserRole).toJsonObject().value(u"id"_s).toString() == u"media-start");
    });
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
        require(images ? (saved.value(u"imageIndex"_s).toInt(-1) == 1) && !saved.contains(u"offset"_s)
            : saved.contains(u"offset"_s) && (saved.value(u"offset"_s).toDouble() == offset)
                && !saved.contains(u"imageIndex"_s),
            u"Library changed the actual image or text reading position"_s);
        bool sameReaderPosition = false;
        for (const QJsonValue &value : fixtureEvents(logPath))
        {
            const QJsonObject event = value.toObject();
            const QJsonObject params = event.value(u"params"_s).toObject();
            if ((event.value(u"event"_s).toString() == u"reader.position")
                && (event.value(u"sourceId"_s).toString() == workId)
                && (params.value(u"chapterId"_s) == saved.value(u"chapterId"_s)))
            {
                sameReaderPosition |= images ? (params.value(u"imageIndex"_s) == saved.value(u"imageIndex"_s))
                    && !params.contains(u"offset"_s)
                    : (params.value(u"offset"_s) == saved.value(u"offset"_s)) && !params.contains(u"imageIndex"_s);
            }
        }
        require(sameReaderPosition, u"Reader and Library persisted different reading positions"_s);
    }
    checks.append(u"actual online Library Reader forwards image-only and text-only positions without invented coordinates"_s);
    evidence[u"checks"_s] = checks;
    evidence[u"fixtureEvents"_s] = fixtureEvents(logPath);
}
