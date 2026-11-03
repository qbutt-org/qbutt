// SPDX-License-Identifier: GPL-2.0-or-later
#include "reader-manga-ahead.h"

#include <functional>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QDebug>
#include <QDir>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFile>
#include <QImage>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QScopeGuard>
#include <QSqlDatabase>
#include <QSqlError>
#include <QSqlQuery>
#include <QTimer>
#include <QUrl>

#include "peer-process.h"

#include "gui/quick/contentview.h"
#include "qbutt_quick/src/controller.cxxqt.h"

using namespace Qt::Literals::StringLiterals;

namespace
{
    void require(const bool condition, const QString &message)
    {
        if (!condition)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const std::function<bool()> &condition, const QString &operation)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        QEventLoop loop;
        QTimer timer;
        timer.setInterval(15);
        QObject::connect(&timer, &QTimer::timeout, &loop, [&]
        {
            if (condition() || elapsed.elapsed() >= 20'000)
                loop.quit();
        });
        if (!condition())
        {
            timer.start();
            loop.exec();
        }
        require(condition(), operation + u" timed out"_s);
    }

    void settle()
    {
        QEventLoop loop;
        QTimer::singleShot(200, &loop, &QEventLoop::quit);
        loop.exec();
    }

    QList<QJsonObject> turns(const QDir &native)
    {
        QList<QJsonObject> result;
        for (const auto &name : native.entryList({u"turn-*.json"_s}, QDir::Files, QDir::Name))
        {
            QFile file(native.filePath(name));
            require(file.open(QIODevice::ReadOnly), u"Peer trace unavailable"_s);
            const auto turn = QJsonDocument::fromJson(file.readAll()).object();
            if (!turn.isEmpty())
                result.append(turn);
        }
        return result;
    }

    void gate(const QDir &native, const int turn, const bool enabled, const QString &kind = u"hold"_s)
    {
        QFile file(native.filePath(kind + u'-' + QString::number(turn)));
        if (enabled)
            require(file.open(QIODevice::WriteOnly), u"Peer gate unavailable"_s);
        else
            require(file.remove(), u"Peer gate release failed"_s);
    }

    bool exited(const QJsonObject &turn)
    {
        const qint64 pid = turn[u"pid"_s].toInteger();
        const quint64 expectedCreation = turn[u"createdAt"_s].toString().toULongLong();
        require(pid && expectedCreation, u"Peer process identity missing"_s);
        const auto process = PeerProcess::state(pid);
        return !process.running || (process.createdAt != expectedCreation);
    }

    QVariant scalar(QSqlDatabase &database, const QString &sql)
    {
        QSqlQuery query(database);
        const bool executed = query.exec(sql);
        require(executed, u"SQLite %1: %2; %3"_s.arg(query.lastError().nativeErrorCode(), query.lastError().text(), sql));
        require(query.next(), u"Fixture query returned no row"_s);
        return query.value(0);
    }

    QImage pageImage(Qbutt::ContentController *controller)
    {
        const QUrl url(controller->property("readerPageUrl").toString());
        require(url.isLocalFile(), u"Reader image is not local"_s);
        const QImage image(url.toLocalFile());
        require(!image.isNull(), u"Reader image decode failed"_s);
        return image.convertToFormat(QImage::Format_ARGB32);
    }
}

void exerciseReaderMangaAhead(Content::ContentView *view, const QString &profileDir)
{
    const QByteArray root = qgetenv("QBUTT_MANGA_PEER_ROOT");
    const QDir native(QString::fromUtf8(qgetenv("QBUTT_MANGA_PEER_DIRECTORY")));
#ifdef Q_OS_WIN
    const QString peerName = u"codex.exe"_s;
#else
    const QString peerName = u"codex"_s;
#endif
    require(!root.isEmpty() && QFile::exists(native.filePath(peerName)), u"Controlled manga peer missing"_s);
    const QByteArray previousProgramFiles = qgetenv("ProgramFiles");
    const QByteArray previousLocalAppData = qgetenv("LOCALAPPDATA");
    const QByteArray previousCodexHome = qgetenv("CODEX_HOME");
    const QByteArray previousPath = qgetenv("PATH");
    const auto restoreEnvironment = qScopeGuard([&]
    {
        qputenv("ProgramFiles", previousProgramFiles);
        qputenv("LOCALAPPDATA", previousLocalAppData);
        qputenv("CODEX_HOME", previousCodexHome);
        qputenv("PATH", previousPath);
    });
    const auto locate = [&](const QByteArray &directory)
    {
        qputenv("ProgramFiles", directory);
        qputenv("LOCALAPPDATA", directory);
        qputenv("CODEX_HOME", directory);
#ifndef Q_OS_WIN
        qputenv("PATH", directory);
#endif
    };
    locate(root);

    auto *controller = view->findChild<Qbutt::ContentController *>();
    require(controller, u"Manga controller missing"_s);
    waitFor([&] { return controller->property("libraryState").value<Qbutt::Library::LibraryState>()
        == Qbutt::Library::LibraryState::Ready; }, u"Library"_s);
    auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, u"reader-manga-ahead"_s);
    const auto closeDatabase = qScopeGuard([&]
    {
        database.close();
        database = {};
        QSqlDatabase::removeDatabase(u"reader-manga-ahead"_s);
    });
    database.setDatabaseName(profileDir + u"/content-library.sqlite"_s);
    require(database.open(), database.lastError().text());
    const auto cached = [&] { return scalar(database, u"SELECT count(*) FROM translated_images"_s).toInt(); };
    const auto import = [&](const QString &name)
    {
        controller->importLocalBook(QUrl::fromLocalFile(profileDir + u'/' + name + u".cbz"_s));
        waitFor([&] { return controller->property("readerOpen").toBool()
            && !controller->property("readerBusy").toBool()
            && controller->property("readerPageCount").toInt() == 5; }, u"Authored CBZ"_s);
        settle();
    };
    const auto translated = [&]
    {
        waitFor([&] { return !controller->property("readerBusy").toBool()
            && !controller->property("readerTranslationBusy").toBool()
            && controller->property("readerTranslationRequested").toBool()
            && !controller->property("readerTranslationError").toBool(); }, u"Foreground translation"_s);
    };
    const auto nextTurn = [&](const int count)
    {
        waitFor([&] { return turns(native).size() == count; }, u"Controlled manga turn "_s + QString::number(count));
        const auto recorded = turns(native);
        // A canceled job must release/reap its peer before another job starts.
        for (int index = 0; index < recorded.size() - 1; ++index)
            require(exited(recorded[index]), u"Peer turn %1 PID %2 overlapped turn %3"_s
                .arg(recorded[index][u"turn"_s].toInt())
                .arg(recorded[index][u"pid"_s].toInteger()).arg(count));
        return recorded.last();
    };

    import(u"first"_s);
    require(turns(native).isEmpty(), u"Original reading started AI"_s);
    const QImage original = pageImage(controller);
    auto *pages = qobject_cast<QAbstractItemModel *>(controller->property("readerPages").value<QObject *>());
    require(pages, u"Reader page model missing"_s);
    const int pageUrlRole = pages->roleNames().key("pageUrl", -1);
    require(pageUrlRole >= 0, u"Reader page URL role missing"_s);
    const auto rowTranslated = [&](const int index)
    {
        const QUrl url = pages->data(pages->index(index, 0), pageUrlRole).toUrl();
        if (!url.isLocalFile())
            return false;
        const QImage image(url.toLocalFile());
        return !image.isNull() && image.convertToFormat(QImage::Format_ARGB32) != original;
    };
    gate(native, 2, true);
    controller->setReaderMangaTranslation(true);
    translated();
    const auto second = nextTurn(2);
    const auto context = second[u"prompt"_s].toObject()[u"workContext"_s].toObject();
    require(context[u"revision"_s].toInt() == 1
        && context[u"terms"_s].toArray().size() == 1 && context[u"facts"_s].toArray().size() == 1,
        u"Read ahead lost accepted work context"_s);
    const auto term = context[u"terms"_s].toArray()[0].toObject();
    const auto fact = context[u"facts"_s].toArray()[0].toObject();
    require(term[u"source"_s].toString() == u"Ren"_s
        && term[u"target"_s].toString() == u"Рен"_s
        && fact[u"subject"_s].toString() == u"Ren"_s
        && fact[u"value"_s].toString() == u"awaited"_s,
        u"Read ahead corrupted accepted names or facts"_s);
    const QImage first = pageImage(controller);
    require(first != original, u"Translation did not change the original page"_s);
    const auto history = scalar(database, u"SELECT entry FROM library LIMIT 1"_s);
    gate(native, 2, false);
    waitFor([&] { return rowTranslated(1); }, u"One following page publication"_s);
    require(cached() == 2, u"Following page publication preceded cache commit"_s);
    settle();
    require(turns(native).size() == 2 && first == pageImage(controller)
        && controller->property("readerPageIndex").toInt() == 0
        && scalar(database, u"SELECT entry FROM library LIMIT 1"_s) == history,
        u"Background changed page/history or exceeded one next page"_s);
    qInfo("PASS reader_manga_ahead bounded context history original_opt_in");

    gate(native, 3, true);
    controller->selectReaderPage(1);
    translated();
    nextTurn(3);
    controller->selectReaderPage(2);
    waitFor([&] { return !controller->property("readerBusy").toBool(); }, u"Promoted original"_s);
    require(controller->property("readerTranslationBusy").toBool() && turns(native).size() == 3,
        u"Promotion lost or duplicated its job"_s);
    gate(native, 3, false);
    translated();
    nextTurn(4);
    waitFor([&] { return rowTranslated(3); }, u"Promoted and next page publication"_s);
    require(cached() == 4, u"Next page publication preceded cache commit"_s);
    controller->selectReaderPage(4);
    translated();
    nextTurn(5);
    require(cached() == 5, u"Last page publication preceded cache commit"_s);
    require(scalar(database, u"SELECT revision FROM translation_context"_s).toInt() == 5,
        u"Promotion wrote duplicate work context"_s);
    const QImage last = pageImage(controller);
    controller->setReaderMangaTranslation(false);
    require(pageImage(controller) == original, u"Original pixels were not restored"_s);
    controller->closeReader();
    locate(profileDir.toUtf8());
    controller->openReader();
    waitFor([&] { return controller->property("readerOpen").toBool()
        && !controller->property("readerBusy").toBool(); }, u"Offline reopen"_s);
    controller->setReaderMangaTranslation(true);
    translated();
    require(pageImage(controller) == last, u"Offline PNG differs from accepted cache"_s);
    controller->selectReaderPage(0);
    translated();
    settle();
    require(turns(native).size() == 5 && pageImage(controller) == first,
        u"Offline current/next cache repeated discovery or AI"_s);
    locate(root);
    qInfo("PASS reader_manga_ahead promotion exact_original offline_cache");

    gate(native, 7, true);
    controller->setReaderTranslationLanguage(u"en"_s);
    translated();
    const auto english = nextTurn(7);
    controller->setReaderTranslationLanguage(u"ru"_s);
    translated();
    waitFor([&] { return exited(english); }, u"Language cancellation"_s);
    gate(native, 8, true);
    controller->setReaderTranslationLanguage(u"en"_s);
    translated();
    const auto superseded = nextTurn(8);
    controller->selectReaderPage(2);
    translated();
    const auto distantAhead = nextTurn(10);
    require(exited(superseded) && scalar(database,
        u"SELECT count(*) FROM translated_images "
        u"WHERE json_extract(key, '$.target_language') = 'en' AND json_extract(key, '$.page') = 1"_s).toInt() == 0,
        u"Canceled next page wrote stale PNG"_s);
    controller->selectReaderPage(0);
    translated();
    settle();
    waitFor([&] { return exited(distantAhead); }, u"Distant prefetch cancellation"_s);
    require(turns(native).size() == 10,
        u"A later prefetch allowed canceled background work to repeat"_s);
    controller->setReaderTranslationLanguage(u"ru"_s);
    translated();
    controller->selectReaderPage(0);
    translated();
    require(controller->loadReaderTranslationOptions(), u"Translation options"_s);
    waitFor([&] { return !controller->property("readerTranslationOptionsLoading").toBool(); }, u"Model catalog"_s);
    gate(native, 12, true);
    require(controller->setReaderTranslationModel(u"fixture-alt"_s), u"Model selection"_s);
    translated();
    const auto model = nextTurn(12);
    gate(native, 14, true);
    require(controller->setReaderTranslationEffort(u"medium"_s), u"Effort selection"_s);
    translated();
    const auto effort = nextTurn(14);
    require(exited(model), u"Model change did not drain old job"_s);
    controller->setReaderMangaTranslation(false);
    require(pageImage(controller) == original, u"Original did not restore during cancellation"_s);
    gate(native, 15, true);
    // Re-enable in the same Qt turn, before the retired job's callback can
    // release its slot. The successor must wait for drain without getting stuck.
    controller->setReaderMangaTranslation(true);
    translated();
    const auto closing = nextTurn(15);
    require(exited(effort), u"Original/Translated did not drain the retired job"_s);
    controller->closeReader();
    waitFor([&] { return exited(closing); }, u"Close cancellation"_s);
    import(u"first"_s);
    gate(native, 16, true);
    controller->setReaderMangaTranslation(true);
    translated();
    const auto replacing = nextTurn(16);
    import(u"second"_s);
    waitFor([&] { return exited(replacing); }, u"Book cancellation"_s);
    require(!controller->property("readerTranslationRequested").toBool(), u"New book inherited translation"_s);
    require(scalar(database,
        u"SELECT count(*) FROM translation_context, json_each(context, '$.facts') "
        u"WHERE json_extract(value, '$.attribute') IN "
        u"('observation-7','observation-8','observation-12','observation-14','observation-15','observation-16')"_s).toInt() == 0,
        u"Canceled work wrote stale context"_s);
    require(scalar(database,
        u"SELECT count(*) FROM translated_images "
        u"WHERE json_extract(key, '$.page') = 1 AND json_extract(key, '$.model') = 'fixture-alt'"_s).toInt() == 0,
        u"Canceled selection wrote stale PNG"_s);
    qInfo("PASS reader_manga_ahead current_priority no_automatic_retry language model effort original_toggle_drain close book_cancel");

    gate(native, 18, true);
    controller->setReaderMangaTranslation(true);
    translated();
    const auto stale = nextTurn(18);
    const int beforeStale = cached();
    QSqlQuery revision(database);
    require(revision.exec(u"UPDATE translation_context SET context = json_set(context, '$.revision', revision + 1), revision = revision + 1"_s),
        revision.lastError().text());
    gate(native, 18, false);
    waitFor([&] { return exited(stale); }, u"Stale context result"_s);
    settle();
    require(cached() == beforeStale && turns(native).size() == 18
        && !controller->property("readerTranslationError").toBool()
        && controller->property("readerTranslationRequested").toBool()
        && controller->property("readerPageIndex").toInt() == 0,
        u"Stale read ahead wrote cache, retried or disturbed foreground"_s);
    qInfo("PASS reader_manga_ahead stale_revision background_error no_paid_retry");

    gate(native, 19, true);
    controller->retryReader();
    translated();
    const auto retiredChapter = nextTurn(19);
    gate(native, 20, true);
    controller->retryReader();
    translated();
    nextTurn(20);
    require(exited(retiredChapter) && cached() == beforeStale,
        u"Chapter replacement accepted stale PNG or repeated the current translation"_s);
    gate(native, 20, false);
    waitFor([&] { return rowTranslated(1); }, u"Replaced chapter read ahead publication"_s);
    require(cached() == beforeStale + 1, u"Replaced chapter publication preceded cache commit"_s);
    settle();
    require(turns(native).size() == 20, u"Chapter reload exceeded one next page"_s);
    gate(native, 21, true, u"fail"_s);
    controller->selectReaderPage(1);
    translated();
    const auto failedAhead = nextTurn(21);
    waitFor([&] { return exited(failedAhead); }, u"Rejected background proposal"_s);
    settle();
    require(cached() == beforeStale + 1 && turns(native).size() == 21
        && controller->property("readerTranslationRequested").toBool()
        && !controller->property("readerTranslationError").toBool()
        && controller->property("readerPageIndex").toInt() == 1,
        u"Background failure changed current reading or retried"_s);
    gate(native, 23, true);
    controller->selectReaderPage(2);
    translated();
    nextTurn(23);
    require(cached() == beforeStale + 2, u"Foreground did not retry the failed next page once"_s);
    const QImage acceptedPage = pageImage(controller);
    const int beforeDecline = cached();
    gate(native, 24, true, u"decline-cleanup"_s);
    controller->selectReaderPage(4);
    nextTurn(24);
    waitFor([&] { return !controller->property("readerTranslationBusy").toBool()
        && controller->property("readerTranslationError").toBool(); }, u"Local sound-effect cleanup decline"_s);
    require(controller->property("readerTranslationRequested").toBool()
        && pageImage(controller) == original && cached() == beforeDecline,
        u"One declined page disabled chapter translation or cached a failure"_s);
    settle();
    require(turns(native).size() == 24, u"Declined current page retried automatically"_s);

    locate(profileDir.toUtf8());
    controller->setReaderMangaTranslation(true);
    waitFor([&] { return !controller->property("readerTranslationBusy").toBool()
        && controller->property("readerTranslationNeedsCodex").toBool(); }, u"Typed unavailable Codex"_s);
    locate(root);
    controller->selectReaderPage(2);
    waitFor([&] { return !controller->property("readerBusy").toBool()
        && !controller->property("readerTranslationBusy").toBool(); }, u"Cached page during service failure"_s);
    require(controller->property("readerTranslationRequested").toBool()
        && pageImage(controller) == acceptedPage,
        u"A service failure hid an accepted translated page"_s);
    controller->selectReaderPage(4);
    waitFor([&] { return !controller->property("readerBusy").toBool()
        && !controller->property("readerTranslationBusy").toBool(); }, u"Paused failed page"_s);
    settle();
    require(turns(native).size() == 24 && cached() == beforeDecline,
        u"A global service failure retried or changed the cache"_s);
    controller->selectReaderPage(2);
    waitFor([&] { return !controller->property("readerBusy").toBool()
        && !controller->property("readerTranslationBusy").toBool(); }, u"Paused cached page before chapter reload"_s);
    const quint64 pausedChapter = pages->property("generation").toULongLong();
    controller->retryReader();
    waitFor([&] { return pages->property("generation").toULongLong() != pausedChapter
        && !controller->property("readerBusy").toBool()
        && !controller->property("readerTranslationBusy").toBool(); }, u"Paused chapter reload"_s);
    require(controller->property("readerTranslationNeedsCodex").toBool()
        && controller->property("readerTranslationRequested").toBool()
        && pageImage(controller) == acceptedPage && turns(native).size() == 24,
        u"Chapter reload resumed a paused service or hid its accepted cache"_s);
    controller->selectReaderPage(4);
    waitFor([&] { return !controller->property("readerBusy").toBool()
        && !controller->property("readerTranslationBusy").toBool(); }, u"Paused chapter cache miss"_s);
    require(controller->property("readerTranslationNeedsCodex").toBool()
        && turns(native).size() == 24 && cached() == beforeDecline,
        u"Chapter cache miss resumed inference without explicit Retry"_s);
    controller->setReaderMangaTranslation(true);
    translated();
    nextTurn(25);
    require(cached() == beforeDecline + 1, u"Explicit retry did not cache the recovered page"_s);
    const QImage recoveredPage = pageImage(controller);
    // Page 1's next page is cached, so this check has no fresh inference target.
    controller->selectReaderPage(1);
    translated();
    require(pageImage(controller) == acceptedPage && turns(native).size() == 25,
        u"Local decline discarded an already translated page"_s);
    controller->selectReaderPage(4);
    translated();
    controller->setReaderMangaTranslation(false);
    require(pageImage(controller) == original, u"Original did not restore after a local decline"_s);
    controller->closeReader();
    locate(profileDir.toUtf8());
    controller->openReader();
    waitFor([&] { return controller->property("readerOpen").toBool()
        && !controller->property("readerBusy").toBool(); }, u"Recovered page reopen"_s);
    controller->setReaderMangaTranslation(true);
    translated();
    require(pageImage(controller) == recoveredPage && turns(native).size() == 25,
        u"Recovered page did not reopen from its exact offline cache"_s);
    qInfo("PASS reader_manga_ahead foreground_decline service_pause retained_pages explicit_retry original offline_cache chapter_pause");
    require(view->grabFramebuffer().save(profileDir + u"/reader-manga-ahead.png"_s), u"Qt render capture"_s);
    controller->closeReader();
    for (const auto &turn : turns(native))
        waitFor([&] { return exited(turn); }, u"Owned peer reaping"_s);
    qInfo("PASS reader_manga_ahead chapter_cancel background_failure foreground_retry cleanup");
}
