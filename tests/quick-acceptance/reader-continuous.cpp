// SPDX-License-Identifier: GPL-2.0-or-later
#include "reader-continuous.h"

#include <functional>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QApplication>
#include <QDebug>
#include <QDir>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFile>
#include <QFileInfo>
#include <QImageReader>
#include <QJsonDocument>
#include <QJsonObject>
#include <QKeyEvent>
#include <QQuickItem>
#include <QScopeGuard>
#include <QSqlDatabase>
#include <QSqlError>
#include <QSqlQuery>
#include <QTemporaryDir>
#include <QTimer>
#include <QWheelEvent>

#ifdef Q_OS_WIN
#include <qt_windows.h>
#endif

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

    void waitFor(const std::function<bool()> &condition, const QString &operation, const int timeout = 100'000)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!condition() && (elapsed.elapsed() < timeout))
        {
            QEventLoop loop;
            QTimer::singleShot(20, &loop, &QEventLoop::quit);
            loop.exec();
        }
        require(condition(), operation + u" timed out"_s);
    }

    QVariant value(QAbstractItemModel *model, const int row, const QByteArray &name)
    {
        const int role = model->roleNames().key(name, -1);
        require(role >= 0, u"Missing page role %1"_s.arg(QString::fromLatin1(name)));
        return model->data(model->index(row, 0), role);
    }

    QQuickItem *pageAt(QQuickItem *viewport, const int index)
    {
        QQuickItem *page = nullptr;
        require(QMetaObject::invokeMethod(viewport, "itemAtIndex", Q_RETURN_ARG(QQuickItem *, page), Q_ARG(int, index)),
            u"Production page lookup is missing"_s);
        return page;
    }

    QQuickItem *reader(Content::ContentView *view)
    {
        auto *loader = view->rootObject()->findChild<QObject *>(u"contentReaderLoader"_s);
        auto *result = loader ? loader->property("item").value<QQuickItem *>() : nullptr;
        require(result, u"Production reading page is missing"_s);
        return result;
    }

    void key(Content::ContentView *view, const Qt::Key code)
    {
        QKeyEvent pressed(QEvent::KeyPress, code, Qt::NoModifier);
        QCoreApplication::sendEvent(view, &pressed);
        QKeyEvent released(QEvent::KeyRelease, code, Qt::NoModifier);
        QCoreApplication::sendEvent(view, &released);
    }

    void execute(QSqlDatabase &database, const QString &sql)
    {
        QSqlQuery query(database);
        require(query.exec(sql), query.lastError().text());
    }

    QJsonObject metadata(QSqlDatabase &database, QString *reference = nullptr)
    {
        QSqlQuery query(database);
        require(query.exec(u"SELECT reference, entry FROM library WHERE json_extract(entry, '$.reader_progress') IS NOT NULL"_s)
            && query.next(), query.lastError().text());
        if (reference)
            *reference = query.value(0).toString();
        return QJsonDocument::fromJson(query.value(1).toString().toUtf8()).object();
    }

    void writeMetadata(QSqlDatabase &database, const QJsonObject &entry)
    {
        QSqlQuery query(database);
        require(query.prepare(u"UPDATE library SET entry = ? WHERE reference = ?"_s), query.lastError().text());
        query.addBindValue(QString::fromUtf8(QJsonDocument(entry).toJson(QJsonDocument::Compact)));
        QString reference;
        metadata(database, &reference);
        query.addBindValue(reference);
        require(query.exec() && (query.numRowsAffected() == 1), query.lastError().text());
    }

    void settled(Qbutt::ContentController *controller)
    {
        waitFor([&]
        {
            return !controller->property("libraryPending").toBool()
                && !controller->property("readerProgressPending").toBool()
                && (controller->property("readerSettingsState").value<Qbutt::Reader::ReaderSettingsState>()
                    != Qbutt::Reader::ReaderSettingsState::Saving);
        }, u"Reader FIFO completion"_s);
    }
}

void exerciseReaderContinuous(Content::ContentView *view, const QString &profileDir)
{
    auto *controller = view->findChild<Qbutt::ContentController *>();
    require(controller && controller->property("readerOpen").toBool(), u"Continuous fixture needs the live reading session"_s);
    settled(controller);
    auto *pages = qobject_cast<QAbstractItemModel *>(controller->property("readerPages").value<QObject *>());
    require(pages && (pages->thread() == qApp->thread())
        && (pages->rowCount() == controller->property("readerPageCount").toInt()) && (pages->rowCount() > 8),
        u"Continuous fixture needs a full native chapter manifest"_s);
    const quint64 generation = pages->property("generation").toULongLong();
    const int lastPage = pages->rowCount() - 1;
    const int initial = controller->property("readerPageIndex").toInt();
    const QUrl initialUrl = value(pages, initial, "pageUrl").toUrl();
    const QSize nativeSize = QImageReader(initialUrl.toLocalFile()).size();
    require((value(pages, initial, "pageIndex").toInt() == initial)
        && (value(pages, initial, "imageWidth").toInt() == nativeSize.width())
        && (value(pages, initial, "imageHeight").toInt() == nativeSize.height())
        && (value(pages, initial, "pageState").toInt() == static_cast<int>(Qbutt::Reader::ReaderPageState::PageReady)),
        u"Page metadata does not describe the actual validated image"_s);

    const QString connection = u"continuous-production-fixture"_s;
    auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, connection);
    const auto closeDatabase = qScopeGuard([&]
    {
        if (database.isOpen())
        {
            QSqlQuery cleanup(database);
            cleanup.exec(u"DROP TRIGGER IF EXISTS continuous_settings_abort"_s);
            database.close();
        }
        database = {};
        QSqlDatabase::removeDatabase(connection);
    });
    database.setDatabaseName(profileDir + u"/content-library.sqlite"_s);
    require(database.open(), database.lastError().text());
    const auto settings = [&]
    {
        QSqlQuery query(database);
        require(query.exec(u"SELECT fit, mode FROM reader_settings WHERE id = 1"_s) && query.next(), query.lastError().text());
        return QSize(query.value(0).toInt(), query.value(1).toInt());
    };
    require(settings() == QSize(1, 0), u"Continuous fault must start in committed Paged/Width"_s);
    execute(database, u"CREATE TRIGGER continuous_settings_abort BEFORE UPDATE ON reader_settings "
        "BEGIN SELECT RAISE(ABORT, 'generated atomic reading settings fault'); END"_s);
    auto *mode = reader(view)->findChild<QQuickItem *>(u"readerMode"_s);
    require(mode && mode->isEnabled(), u"Production reading-mode selector is missing"_s);
    mode->forceActiveFocus(Qt::TabFocusReason);
    key(view, Qt::Key_Space);
    auto *popup = mode->property("popup").value<QObject *>();
    require(popup, u"Reading-mode popup is missing"_s);
    waitFor([&] { return popup->property("visible").toBool(); }, u"Actual mode popup keyboard opening"_s);
    key(view, Qt::Key_Down);
    key(view, Qt::Key_Return);
    waitFor([&]
    {
        return !popup->property("visible").toBool()
            && (controller->property("readerMode").value<Qbutt::Reader::ReaderMode>() == Qbutt::Reader::ReaderMode::Continuous);
    }, u"Actual mode popup keyboard selection"_s);
    require(controller->setReaderFit(Qbutt::Reader::ReaderFit::Height), u"Applied Height intent was rejected"_s);
    waitFor([&]
    {
        return controller->property("readerSettingsState").value<Qbutt::Reader::ReaderSettingsState>()
            == Qbutt::Reader::ReaderSettingsState::Failed;
    }, u"Atomic mode/fit SQL abort"_s);
    require((settings() == QSize(1, 0))
        && (controller->property("readerMode").value<Qbutt::Reader::ReaderMode>() == Qbutt::Reader::ReaderMode::Continuous)
        && (controller->property("readerFit").value<Qbutt::Reader::ReaderFit>() == Qbutt::Reader::ReaderFit::Height),
        u"Failed atomic settings write invented a committed mode or lost its applied intent"_s);
    const bool favorite = controller->property("selectedFavorite").toBool();
    controller->setSelectedFavorite(!favorite);
    settled(controller);
    require(controller->property("readerSettingsState").value<Qbutt::Reader::ReaderSettingsState>()
        == Qbutt::Reader::ReaderSettingsState::Failed, u"Another library acknowledgement hid failed mode settings"_s);
    execute(database, u"DROP TRIGGER continuous_settings_abort"_s);
    controller->retryReaderSettings();
    settled(controller);
    require((settings() == QSize(0, 1))
        && controller->property("readerSettingsErrorText").toString().isEmpty(), u"Exact atomic settings retry failed"_s);

    auto *scroll = reader(view)->findChild<QQuickItem *>(u"readerContinuousScroll"_s);
    auto *viewport = reader(view)->findChild<QQuickItem *>(u"readerContinuousViewport"_s);
    require(scroll && viewport && scroll->hasActiveFocus(), u"Continuous did not focus its actual scrollable content"_s);
    waitFor([&]
    {
        auto *item = pageAt(viewport, initial);
        auto *image = item ? item->findChild<QQuickItem *>(u"readerStripImage%1"_s.arg(initial)) : nullptr;
        return image && image->isVisible() && (image->property("status").toInt() == 1);
    }, u"Actual continuous image decode"_s);
    require(!controller->setReaderViewport(generation + 1, 0, 0, 0, 0.2, true)
        && !controller->setReaderViewport(generation, -1, 0, 0, 0.2, true)
        && !controller->setReaderViewport(generation, 0, pages->rowCount(), 0, 0.2, true),
        u"Invalid or stale viewport changed the reader"_s);
    const double beforeKey = viewport->property("contentY").toDouble();
    key(view, Qt::Key_Down);
    waitFor([&] { return viewport->property("contentY").toDouble() > beforeKey; }, u"Continuous real Down key"_s);
    waitFor([&]
    {
        return controller->property("readerScrollOffset").toDouble() > 0
            && !controller->property("readerProgressPending").toBool();
    }, u"Decoded continuous user position"_s);
    settled(controller);
    const int anchor = controller->property("readerPageIndex").toInt();
    auto *anchorPage = pageAt(viewport, anchor);
    require(anchorPage && (qAbs(controller->property("readerScrollOffset").toDouble()
        - (viewport->property("contentY").toDouble() - anchorPage->y()) / anchorPage->height()) < 0.000001)
        && (metadata(database).value(u"reader_progress"_s).toObject().value(u"page"_s).toInt() == anchor),
        u"Continuous progress is not relative to the actual rendered anchor page"_s);
    const QPointF wheelPosition = viewport->mapToScene(QPointF(viewport->width() / 2, viewport->height() / 2));
    QWheelEvent wheel(wheelPosition, wheelPosition, QPoint(), QPoint(0, -120), Qt::NoButton, Qt::NoModifier,
        Qt::NoScrollPhase, false);
    const double beforeWheel = viewport->property("contentY").toDouble();
    QCoreApplication::sendEvent(view, &wheel);
    waitFor([&] { return viewport->property("contentY").toDouble() > beforeWheel; }, u"Continuous real wheel"_s);
    settled(controller);

    int delegates = 0;
    int decodedImages = 0;
    auto *content = viewport->property("contentItem").value<QQuickItem *>();
    require(content, u"Continuous visual content item is missing"_s);
    for (auto *item : content->childItems())
    {
        if (!item->objectName().startsWith(u"readerStripPage"_s))
            continue;
        ++delegates;
        auto *image = item->findChild<QQuickItem *>(u"readerStripImage%1"_s.arg(item->property("pageIndex").toInt()));
        if (image)
        {
            ++decodedImages;
            require(image->isVisible() && !image->property("cache").toBool(), u"An offscreen image retained a decoded texture"_s);
            const QSize requested = image->property("sourceSize").toSize();
            const QSize original = QImageReader(image->property("source").toUrl().toLocalFile()).size();
            require((requested.width() > 0) && (requested.height() > 0)
                && (requested.width() <= original.width()) && (requested.height() <= original.height()),
                u"Continuous decode is not bounded by native and rendered image dimensions"_s);
        }
    }
    require((delegates > 0) && (delegates < pages->rowCount()) && (decodedImages > 0),
        u"Continuous created the full chapter instead of virtualizing its pages"_s);

    // A legal saved position can become physically unreachable after fit/resize.
    // Reopen the real last page and verify that automatic clamping keeps it exact.
    settled(controller);
    controller->closeReader();
    auto savedEntry = metadata(database);
    auto desired = savedEntry.value(u"reader_progress"_s).toObject();
    require(pages->rowCount() == 0, u"Closed model retained page rows"_s);
    desired.insert(u"page"_s, lastPage);
    desired.insert(u"scroll_offset"_s, 0.37);
    savedEntry.insert(u"reader_progress"_s, desired);
    writeMetadata(database, savedEntry);
    controller->openReader();
    viewport = reader(view)->findChild<QQuickItem *>(u"readerContinuousViewport"_s);
    require(viewport, u"New continuous viewport is missing"_s);
    waitFor([&]
    {
        auto *item = pageAt(viewport, lastPage);
        auto *image = item ? item->findChild<QQuickItem *>(u"readerStripImage%1"_s.arg(lastPage)) : nullptr;
        return image && image->isVisible() && (image->property("status").toInt() == 1)
            && !controller->property("readerBusy").toBool();
    }, u"Saved exact last-page render"_s);
    settled(controller);
    viewport = reader(view)->findChild<QQuickItem *>(u"readerContinuousViewport"_s);
    require(viewport, u"Resumed continuous viewport is missing"_s);
    for (const QSize windowSize : {QSize(1280, 720), QSize(1440, 900)})
    {
        view->window()->resize(windowSize);
        waitFor([&]
        {
            auto *lastItem = pageAt(viewport, lastPage);
            return lastItem && (lastItem->height() <= viewport->height() + 1)
                && (lastItem->y() + 0.37 * lastItem->height() - viewport->property("contentY").toDouble() > 1);
        }, u"Physical short last-page clamp"_s);
        settled(controller);
        require((controller->property("readerPageIndex").toInt() == lastPage)
            && (controller->property("readerScrollOffset").toDouble() == 0.37)
            && (metadata(database).value(u"reader_progress"_s).toObject() == desired),
            u"Automatic Height/resize clamp replaced the exact saved anchor position"_s);
    }
    require(controller->setReaderFit(Qbutt::Reader::ReaderFit::Width), u"Continuous Width intent was rejected"_s);
    settled(controller);
    waitFor([&]
    {
        auto *item = pageAt(viewport, lastPage);
        auto *image = item ? item->findChild<QQuickItem *>(u"readerStripImage%1"_s.arg(lastPage)) : nullptr;
        return image && (image->property("status").toInt() == 1);
    }, u"Continuous Width re-decode"_s);
    require((controller->property("readerPageIndex").toInt() == lastPage)
        && (metadata(database).value(u"reader_progress"_s).toObject() == desired),
        u"Fit change altered page-relative saved progress"_s);

    const QUrl failedUrl = value(pages, lastPage, "pageUrl").toUrl();
    const QJsonObject beforeDecodeFault = metadata(database);
    {
        QFile corrupted(failedUrl.toLocalFile());
        require(corrupted.open(QIODevice::WriteOnly | QIODevice::Truncate)
            && (corrupted.write("generated image decode fault") > 0), u"Could not corrupt the isolated cached page"_s);
    }
    require(controller->setReaderFit(Qbutt::Reader::ReaderFit::Height), u"Decode fault re-layout was rejected"_s);
    waitFor([&]
    {
        return value(pages, lastPage, "pageState").toInt() == static_cast<int>(Qbutt::Reader::ReaderPageState::PageFailed);
    }, u"Actual QML Image.Error acknowledgement"_s);
    settled(controller);
    require(!controller->property("readerBusy").toBool()
        && value(pages, lastPage, "pageUrl").toUrl().isEmpty()
        && (metadata(database) == beforeDecodeFault), u"Failed decode created progress or left a permanent busy state"_s);
    auto *failedPage = pageAt(viewport, lastPage);
    auto *retry = failedPage ? failedPage->findChild<QQuickItem *>(u"readerStripRetry%1"_s.arg(lastPage)) : nullptr;
    require(retry && retry->isVisible() && retry->isEnabled() && QMetaObject::invokeMethod(retry, "clicked"),
        u"Failed continuous page has no exact retry action"_s);
    waitFor([&]
    {
        auto *item = pageAt(viewport, lastPage);
        auto *image = item ? item->findChild<QQuickItem *>(u"readerStripImage%1"_s.arg(lastPage)) : nullptr;
        return image && image->isVisible() && (image->property("status").toInt() == 1)
            && (value(pages, lastPage, "pageUrl").toUrl() != failedUrl);
    }, u"Exact manifest row acquisition retry"_s);
    settled(controller);
    require((controller->property("readerPageIndex").toInt() == lastPage)
        && (controller->property("readerScrollOffset").toDouble() == 0.37)
        && (metadata(database) == beforeDecodeFault), u"Retry substituted or re-confirmed an already shown position"_s);
    controller->readerPageShown(lastPage, failedUrl);
    controller->readerPageFailed(lastPage, failedUrl);
    require(!controller->property("readerProgressPending").toBool()
        && (value(pages, lastPage, "pageState").toInt() == static_cast<int>(Qbutt::Reader::ReaderPageState::PageReady)),
        u"Stale local URL changed the newly acquired exact page"_s);

    // Keep a retired file physically locked beyond the former cleanup timeout.
    // The live cache must release it after unlock without closing the chapter.
#ifdef Q_OS_WIN
    const QUrl retiringUrl = value(pages, lastPage, "pageUrl").toUrl();
    const QString retiringPath = retiringUrl.toLocalFile();
    const HANDLE held = CreateFileW(reinterpret_cast<LPCWSTR>(retiringPath.utf16()), GENERIC_READ,
        FILE_SHARE_READ, nullptr, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
    require(held != INVALID_HANDLE_VALUE, u"Could not hold the isolated reader cache file"_s);
    bool closed = false;
    const auto closeHeld = qScopeGuard([&] { if (!closed) CloseHandle(held); });
    controller->selectReaderPage(0);
    waitFor([&] { return !controller->property("readerBusy").toBool(); }, u"Anchor priority after leaving the last page"_s);
    QElapsedTimer holdTime;
    holdTime.start();
    waitFor([&] { return holdTime.elapsed() > 2'500; }, u"Physical retired-cache lock interval"_s, 5'000);
    require(QFileInfo::exists(retiringPath), u"A locked physical cache file unexpectedly disappeared"_s);
    CloseHandle(held);
    closed = true;
    waitFor([&] { return !QFileInfo::exists(retiringPath); }, u"Retired byte lease physical unlink after unlock"_s, 5'000);
#endif
    qint64 cachedBytes = 0;
    const QString directory = QFileInfo(value(pages, 0, "pageUrl").toUrl().toLocalFile()).absolutePath();
    for (const QFileInfo &file : QDir(directory).entryInfoList(QDir::Files))
        cachedBytes += file.size();
    require((cachedBytes > 0) && (cachedBytes <= (120LL * 1024 * 1024)), u"Encoded cache exceeded its bounded owner"_s);
    require(controller->setReaderMode(Qbutt::Reader::ReaderMode::Paged)
        && controller->setReaderFit(Qbutt::Reader::ReaderFit::Width), u"Paged/Width restoration was rejected"_s);
    controller->setSelectedFavorite(favorite);
    settled(controller);
    require((settings() == QSize(1, 0)) && (controller->property("selectedFavorite").toBool() == favorite),
        u"Continuous acceptance did not restore its surrounding fixture settings"_s);
    const QJsonObject beforeCacheFault = metadata(database);
    require(directory.startsWith(QDir::tempPath() + u"/qbutt-reader-"_s, Qt::CaseInsensitive),
        u"Fault target is outside the private reader cache: %1 temp=%2"_s.arg(directory, QDir::tempPath()));
    waitFor([&] { return QDir(directory).removeRecursively(); }, u"Prepare owned cache-directory fault"_s, 5'000);
    QFile blockedDirectory(directory);
    require(blockedDirectory.open(QIODevice::WriteOnly), u"Could not block the exact reader directory"_s);
    blockedDirectory.close();
    bool directoryRecovered = false;
    const auto restoreDirectory = qScopeGuard([&]
    {
        if (!directoryRecovered)
        {
            blockedDirectory.remove();
            QDir().mkpath(directory);
        }
    });
    require(controller->setReaderFit(Qbutt::Reader::ReaderFit::Height), u"Cache fault decode transition failed"_s);
    waitFor([&]
    {
        return value(pages, 0, "pageState").toInt() == static_cast<int>(Qbutt::Reader::ReaderPageState::PageFailed);
    }, u"Missing physical page decode"_s);
    controller->retryReaderPage(pages->property("generation").toULongLong(), 0);
    waitFor([&]
    {
        return value(pages, 0, "pageState").toInt() == static_cast<int>(Qbutt::Reader::ReaderPageState::PageFailed);
    }, u"Repeated actual acquisition failure"_s);
    require(!controller->property("readerBusy").toBool() && (metadata(database) == beforeCacheFault),
        u"Failed acquisition retry invented reading or remained busy"_s);
    require(controller->setReaderMode(Qbutt::Reader::ReaderMode::Continuous)
        && controller->setReaderMode(Qbutt::Reader::ReaderMode::Paged), u"Failed-page mode transition was rejected"_s);
    require(blockedDirectory.remove() && QDir().mkpath(directory), u"Could not recover the same reader directory"_s);
    directoryRecovered = true;
    controller->retryReaderPage(pages->property("generation").toULongLong(), 0);
    waitFor([&]
    {
        return !controller->property("readerBusy").toBool()
            && (value(pages, 0, "pageState").toInt() == static_cast<int>(Qbutt::Reader::ReaderPageState::PageReady));
    }, u"Retry identity after failure and mode ABA"_s, 10'000);
    const QUrl recoveredUrl = value(pages, 0, "pageUrl").toUrl();
    require(!QImageReader(recoveredUrl.toLocalFile()).read().isNull(), u"Recovered native page did not decode"_s);
    {
        QFile corrupted(recoveredUrl.toLocalFile());
        require(corrupted.open(QIODevice::WriteOnly | QIODevice::Truncate)
            && (corrupted.write("generated reentrant retry fault") > 0), u"Could not prepare the retry notification fault"_s);
    }
    require(controller->setReaderFit(Qbutt::Reader::ReaderFit::Width), u"Reentrant decode transition failed"_s);
    waitFor([&]
    {
        return value(pages, 0, "pageState").toInt() == static_cast<int>(Qbutt::Reader::ReaderPageState::PageFailed);
    }, u"Reentrant retry failed row"_s);
    QObject retryObserver;
    bool navigatedDuringRetry = false;
    QObject::connect(pages, &QAbstractItemModel::dataChanged, &retryObserver, [&]
    {
        if (!navigatedDuringRetry && (value(pages, 0, "pageState").toInt()
            == static_cast<int>(Qbutt::Reader::ReaderPageState::PageLoading)))
        {
            navigatedDuringRetry = true;
            controller->selectReaderPage(3);
        }
    }, Qt::DirectConnection);
    controller->retryReaderPage(pages->property("generation").toULongLong(), 0);
    waitFor([&]
    {
        return !controller->property("readerBusy").toBool()
            && !controller->property("readerProgressPending").toBool()
            && (metadata(database).value(u"reader_progress"_s).toObject().value(u"page"_s).toInt() == 3);
    }, u"Reentrant retry preserves newer decoded navigation"_s, 10'000);
    require(navigatedDuringRetry && (controller->property("readerPageIndex").toInt() == 3)
        && controller->property("readerErrorText").toString().isEmpty(), u"Retry completion replaced newer navigation"_s);
    QObject reentrantObserver;
    int resetDepth = 0;
    int maximumResetDepth = 0;
    bool reopenedDuringReset = false;
    QObject::connect(pages, &QAbstractItemModel::modelAboutToBeReset, &reentrantObserver, [&]
    {
        ++resetDepth;
        maximumResetDepth = qMax(maximumResetDepth, resetDepth);
        if (!reopenedDuringReset && controller->property("readerOpen").toBool())
        {
            reopenedDuringReset = true;
            controller->closeReader();
            controller->openReader();
        }
    }, Qt::DirectConnection);
    QObject::connect(pages, &QAbstractItemModel::modelReset, &reentrantObserver, [&] { --resetDepth; }, Qt::DirectConnection);
    controller->closeReader();
    controller->openReader();
    waitFor([&] { return !controller->property("readerBusy").toBool(); }, u"Reentrant page-model close/reopen"_s);
    require(reopenedDuringReset && (maximumResetDepth == 1) && (resetDepth == 0)
        && (controller->property("readerPageIndex").toInt() == 3)
        && controller->property("readerErrorText").toString().isEmpty(),
        u"Page model allowed nested reset or a stale session to replace the latest reader"_s);
    settled(controller);
    qInfo().noquote() << u"PASS reader_continuous model=ok decoded_anchor=ok key=ok wheel=ok virtualized=ok atomic_retry=ok short_last_clamp=ok resize=ok decode_retry=ok stale_url=ok retired_lock=ok retry_aba=ok reentrant_navigation=ok reentrant_reset=ok cached_bytes=%1 delegates=%2 images=%3 pages=%4"_s
        .arg(cachedBytes).arg(delegates).arg(decodedImages).arg(pages->rowCount());
}
