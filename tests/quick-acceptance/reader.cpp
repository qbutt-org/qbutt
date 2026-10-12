// SPDX-License-Identifier: GPL-2.0-or-later
#include "reader.h"

#include <atomic>
#include <cstdio>
#include <functional>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QApplication>
#include <QDebug>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFile>
#include <QFileInfo>
#include <QImageReader>
#include <QKeyEvent>
#include <QPointF>
#include <QPointer>
#include <QQuickItem>
#include <QScopeGuard>
#include <QTemporaryDir>
#include <QTimer>
#include <QUrl>

#include "gui/quick/contentview.h"
#include "qbutt_quick/src/controller.cxxqt.h"

using namespace Qt::Literals::StringLiterals;

namespace
{
    std::atomic_bool bindingLoopDetected = false;
    QtMessageHandler previousMessageHandler = nullptr;

    void observeBindingLoops(const QtMsgType type, const QMessageLogContext &context, const QString &message)
    {
        if (message.contains(u"Binding loop"_s))
            bindingLoopDetected.store(true);

        if (previousMessageHandler)
            previousMessageHandler(type, context, message);
        else
            std::fprintf(stderr, "%s\n", qPrintable(qFormatLogMessage(type, context, message)));
    }

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
        timer.setInterval(20);
        QObject::connect(&timer, &QTimer::timeout, &loop, [&]
        {
            if (condition() || (elapsed.elapsed() >= 100'000))
                loop.quit();
        });
        if (!condition())
        {
            timer.start();
            loop.exec();
        }
        require(condition(), operation + u" timed out"_s);
    }

    QAbstractItemModel *model(QObject *controller, const char *name)
    {
        auto *result = qobject_cast<QAbstractItemModel *>(controller->property(name).value<QObject *>());
        require(result && (result->thread() == qApp->thread()), u"Missing GUI-thread model"_s);
        return result;
    }

    QString value(QAbstractItemModel *model, const int row, const QByteArray &name)
    {
        const int role = model->roleNames().key(name, -1);
        require(role >= 0, u"Missing model role"_s);
        return model->data(model->index(row, 0), role).toString();
    }

    QString readyPage(Qbutt::ContentController *controller)
    {
        waitFor([&] { return !controller->property("readerBusy").toBool(); }, u"Reader page"_s);
        require(controller->property("readerOpen").toBool(), u"Reader unexpectedly closed"_s);
        require(controller->property("readerErrorText").toString().isEmpty(),
            controller->property("readerErrorText").toString());
        const QUrl url(controller->property("readerPageUrl").toString());
        require(url.isLocalFile(), u"Reader exposed a network acquisition URL"_s);
        QImageReader image(url.toLocalFile());
        require(image.canRead() && !image.read().isNull(), u"Prepared reader page could not be decoded"_s);
        require(QFileInfo(url.toLocalFile()).size() > 0, u"Prepared reader page is empty"_s);
        return url.toLocalFile();
    }
}

void exerciseReader(Content::ContentView *view)
{
    bindingLoopDetected.store(false);
    previousMessageHandler = qInstallMessageHandler(observeBindingLoops);
    const auto restoreMessageHandler = qScopeGuard([] { qInstallMessageHandler(previousMessageHandler); });
    require(view && (view->status() == QQuickWidget::Ready), u"Production reader view did not load"_s);
    QPointer<Qbutt::ContentController> controller = view->findChild<Qbutt::ContentController *>();
    require(controller, u"Production content controller is missing"_s);
    waitFor([&] { return controller->property("libraryState").value<Qbutt::Library::LibraryState>() == Qbutt::Library::LibraryState::Ready; },
        u"Isolated library"_s);

    const QString publication = u"mangadex:b0b721ff-c388-4486-aa0f-c2b0bb321512"_s;
    auto *results = view->rootObject()->findChild<QQuickItem *>(u"contentResults"_s);
    require(results, u"Production results view is missing"_s);
    QObject selectionObservation;
    const auto completionConnection = QObject::connect(controller, &Qbutt::ContentController::busyChanged,
        &selectionObservation, [&]
    {
        if (!controller->property("busy").toBool())
            controller->selectWork(publication);
    }, Qt::DirectConnection);
    controller->search(u"Frieren"_s);
    waitFor([&] { return !controller->property("busy").toBool(); }, u"Mixed source search"_s);
    QObject::disconnect(completionConnection);
    auto *works = model(controller, "works");
    bool found = false;
    bool animeFound = false;
    for (int row = 0; row < works->rowCount(); ++row)
    {
        found |= value(works, row, "workId") == publication;
        animeFound |= value(works, row, "source") == u"AniLiberty";
    }
    require(found && animeFound, u"Mixed search did not retain both source classes"_s);
    require((controller->property("selectedWorkId").toString() == publication)
        && (results->property("currentIndex").toInt() >= 0)
        && (value(works, results->property("currentIndex").toInt(), "workId") == publication)
        && !bindingLoopDetected.load(),
        u"Selection before the pending view layout caused a binding loop or lost its exact work"_s);
    results->forceActiveFocus();
    for (const Qt::Key key : {Qt::Key_Home, Qt::Key_Down, Qt::Key_Up, Qt::Key_End})
    {
        QKeyEvent press(QEvent::KeyPress, key, Qt::NoModifier);
        QCoreApplication::sendEvent(view, &press);
        QKeyEvent release(QEvent::KeyRelease, key, Qt::NoModifier);
        QCoreApplication::sendEvent(view, &release);
        int row = 0;
        if (key == Qt::Key_Down)
            row = 1;
        else if (key == Qt::Key_End)
            row = works->rowCount() - 1;
        require((results->property("currentIndex").toInt() == row)
            && (controller->property("selectedWorkId").toString() == value(works, row, "workId")),
            u"Keyboard work highlight and exact selection differ"_s);
    }
    controller->selectWork(publication);
    waitFor([&] { return !controller->property("inspecting").toBool(); }, u"Manga inspection"_s);
    require(controller->property("errorText").toString().isEmpty(), u"Manga inspection failed"_s);
    const QString exact = publication + u":translation:ru:9e383226-2af3-4af0-a69d-3d3b630212b3"_s;
    controller->selectVariant(exact);
    require(controller->property("selectedVariantId").toString() == exact, u"Exact manga translation is unavailable"_s);
    require(controller->property("selectedVariantKind").toString() == u"manga", u"Manga action has the wrong transport"_s);
    controller->setSelectedFavorite(true);
    waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Save manga favorite"_s);
    require(controller->property("selectedFavorite").toBool(), u"Manga favorite was not saved"_s);
    controller->setListMode(Qbutt::Search::ListMode::Favorites);
    require((results->property("currentIndex").toInt() == 0)
        && (controller->property("selectedVariantId").toString() == exact),
        u"Favorites switch lost the exact selected work or variant"_s);
    controller->setSelectedFavorite(false);
    waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Remove manga favorite"_s);
    waitFor([&] { return results->property("count").toInt() == 0; }, u"Favorite view removal"_s);
    require(results->property("currentIndex").toInt() == -1, u"Removed favorite remained the current work"_s);
    controller->setListMode(Qbutt::Search::ListMode::Search);
    require((results->property("currentIndex").toInt() >= 0)
        && (value(works, results->property("currentIndex").toInt(), "workId") == publication)
        && (controller->property("selectedVariantId").toString() == exact),
        u"Returning to search lost the exact selected work or variant"_s);
    require(!bindingLoopDetected.load(), u"Model change caused a work selection binding loop"_s);
    auto *chapters = model(controller, "readerChapters");
    QObject observations;
    int resetDepth = 0;
    int maximumResetDepth = 0;
    bool changedDuringReset = false;
    QObject::connect(chapters, &QAbstractItemModel::modelAboutToBeReset, &observations, [&]
    {
        ++resetDepth;
        maximumResetDepth = qMax(maximumResetDepth, resetDepth);
        if (!changedDuringReset)
        {
            changedDuringReset = true;
            controller->setChapterQuery(u"142"_s);
            controller->setChapterQuery(u"141"_s);
        }
    }, Qt::DirectConnection);
    QObject::connect(chapters, &QAbstractItemModel::modelReset, &observations, [&] { --resetDepth; }, Qt::DirectConnection);
    auto *pane = view->rootObject()->findChild<QObject *>(u"contentWorkPane"_s);
    require(pane && QMetaObject::invokeMethod(pane, "openSelected"), u"QML primary manga action is missing"_s);
    const QString firstPage = readyPage(controller);
    require(changedDuringReset && (maximumResetDepth == 1) && (resetDepth == 0),
        u"Chapter query caused a nested or unbalanced model reset"_s);
    require(controller->property("chapterQuery").toString() == u"141", u"Chapter reset overwrote the latest filter intent"_s);
    controller->setChapterQuery(u""_s);
    require(controller->property("readerPageIndex").toInt() == 0, u"New reading session did not start at its first page"_s);
    require(controller->property("readerPageCount").toInt() > 1, u"Live chapter needs a second page"_s);
    const int chapterCount = chapters->rowCount();
    require(chapterCount > 1, u"Live translation needs a second chapter"_s);
    const QString firstNumber = value(chapters, 0, "number");
    const QString secondNumber = value(chapters, 1, "number");
    require(!firstNumber.isEmpty() && !secondNumber.isEmpty() && (firstNumber != secondNumber),
        u"Reset fault needs two distinct chapter numbers"_s);
    {
        QObject endObservations;
        bool insideResetSignal = false;
        bool resetWithinSignal = false;
        bool changedAtEnd = false;
        QObject::connect(chapters, &QAbstractItemModel::modelAboutToBeReset, &endObservations,
            [&] { resetWithinSignal |= insideResetSignal; }, Qt::DirectConnection);
        QObject::connect(chapters, &QAbstractItemModel::modelReset, &endObservations, [&]
        {
            if (changedAtEnd)
                return;
            changedAtEnd = true;
            insideResetSignal = true;
            controller->setChapterQuery(secondNumber);
            insideResetSignal = false;
        }, Qt::DirectConnection);
        controller->setChapterQuery(firstNumber);
        require(changedAtEnd && !resetWithinSignal
            && (controller->property("chapterQuery").toString() == secondNumber)
            && (chapters->rowCount() > 0) && (value(chapters, 0, "number") == secondNumber),
            u"Latest chapter filter changed during modelReset was not applied"_s);
    }
    controller->setChapterQuery(u""_s);
    const QString firstChapter = controller->property("readerChapterId").toString();
    const QString lastChapter = value(chapters, chapterCount - 1, "chapterId");
    const QString lastNumber = value(chapters, chapterCount - 1, "number");
    controller->setChapterQuery(lastNumber);
    require(chapters->rowCount() > 0 && (chapters->rowCount() < chapterCount), u"Chapter filtering did not reduce the visible list"_s);
    controller->setChapterQuery(u""_s);
    require(chapters->rowCount() == chapterCount, u"Chapter filtering discarded the full list"_s);

    controller->turnReaderPage(1);
    const QString secondPage = readyPage(controller);
    require((controller->property("readerPageIndex").toInt() == 1) && (firstPage != secondPage),
        u"Page turn did not acquire the exact second page"_s);
    waitFor([&] { return !QFileInfo::exists(firstPage); }, u"Retired page cache cleanup"_s);
    auto *pageImage = view->rootObject()->findChild<QQuickItem *>(u"readerPageImage"_s);
    auto *viewport = view->rootObject()->findChild<QQuickItem *>(u"readerPageViewport"_s);
    auto *scrollBar = view->rootObject()->findChild<QQuickItem *>(u"readerPageScrollBar"_s);
    require(pageImage && viewport && scrollBar, u"Production reader image, viewport or scrollbar is missing"_s);
    const QSize decodedSize = QImageReader(secondPage).size();
    require(!decodedSize.isEmpty(), u"Decoded page size is missing"_s);
    const auto checkPageFit = [&](const Qbutt::Reader::ReaderFit fit)
    {
        waitFor([&] { return pageImage->property("status").toInt() == 1; }, u"Reader image decode"_s);
        require(controller->property("readerFit").value<Qbutt::Reader::ReaderFit>() == fit,
            u"Requested image fit did not reach the backend"_s);
        const qreal paintedWidth = pageImage->property("paintedWidth").toReal();
        const qreal paintedHeight = pageImage->property("paintedHeight").toReal();
        const QPointF scrollBarOrigin = scrollBar->mapToItem(viewport, QPointF());
        require((qAbs(scrollBar->height() - viewport->height()) < 1)
            && (qAbs(scrollBarOrigin.y()) < 1)
            && (qAbs(scrollBarOrigin.x() + scrollBar->width() - viewport->width()) < 1),
            u"Reader scrollbar does not cover the viewport's right edge"_s);
        if (fit == Qbutt::Reader::ReaderFit::Height)
        {
            require((qAbs(pageImage->height() - viewport->height()) < 1)
                && (paintedWidth <= viewport->width() + 1) && (paintedHeight <= viewport->height() + 1),
                u"Height fit exceeds the reader viewport"_s);
        }
        else
        {
            const qreal expectedHeight = viewport->width() * decodedSize.height() / decodedSize.width();
            require((qAbs(paintedWidth - viewport->width()) < 1)
                && (qAbs(paintedHeight - expectedHeight) < 2),
                u"Width fit lost the decoded page aspect or viewport width"_s);
        }
        require((paintedWidth > 0) && (paintedHeight > 0)
            && (qAbs(viewport->property("contentHeight").toReal() - qMax(viewport->height(), pageImage->height())) < 1)
            && !bindingLoopDetected.load(), u"Decoded image fit caused a binding loop or incorrect scroll bounds"_s);
    };
    checkPageFit(Qbutt::Reader::ReaderFit::Height);
    for (const QSize windowSize : {QSize(1280, 720), QSize(1440, 900)})
    {
        view->window()->resize(windowSize);
        checkPageFit(Qbutt::Reader::ReaderFit::Height);
        // Exercise both the first decode and the cached size transition.
        for (int pass = 0; pass < 2; ++pass)
        {
            controller->setReaderFit(Qbutt::Reader::ReaderFit::Width);
            checkPageFit(Qbutt::Reader::ReaderFit::Width);
            controller->setReaderFit(Qbutt::Reader::ReaderFit::Height);
            checkPageFit(Qbutt::Reader::ReaderFit::Height);
        }
    }

    auto *scroll = view->rootObject()->findChild<QQuickItem *>(u"readerPageScroll"_s);
    require(scroll && scroll->hasActiveFocus(), u"Reading did not focus its scrollable content"_s);
    for (const QString action : {u"readerHeightFit"_s, u"readerWidthFit"_s})
    {
        auto *button = view->rootObject()->findChild<QQuickItem *>(action);
        require(button, u"Reading fit action is missing"_s);
        button->forceActiveFocus(Qt::TabFocusReason);
        require(QMetaObject::invokeMethod(button, "clicked"), u"Reading fit action did not run"_s);
        require(scroll->hasActiveFocus(), u"Reading fit action did not return focus to content"_s);
    }
    checkPageFit(Qbutt::Reader::ReaderFit::Width);
    for (const Qt::Key key : {Qt::Key_Down, Qt::Key_Up})
    {
        const qreal previousOffset = viewport->property("contentY").toReal();
        QKeyEvent press(QEvent::KeyPress, key, Qt::NoModifier);
        QCoreApplication::sendEvent(view, &press);
        QKeyEvent release(QEvent::KeyRelease, key, Qt::NoModifier);
        QCoreApplication::sendEvent(view, &release);
        require(key == Qt::Key_Down ? viewport->property("contentY").toReal() > previousOffset
                                   : viewport->property("contentY").toReal() < previousOffset,
            u"Reading Up/Down did not scroll the page"_s);
        const qreal range = viewport->property("contentHeight").toReal() - viewport->height();
        require((range > 0) && (qAbs(controller->property("readerScrollOffset").toReal()
            - viewport->property("contentY").toReal() / range) < 0.000001),
            u"Keyboard reading position was not saved"_s);
    }
    auto *widthButton = view->rootObject()->findChild<QQuickItem *>(u"readerWidthFit"_s);
    widthButton->forceActiveFocus(Qt::TabFocusReason);
    QKeyEvent tab(QEvent::KeyPress, Qt::Key_Tab, Qt::NoModifier);
    QCoreApplication::sendEvent(view, &tab);
    require(scroll->hasActiveFocus(), u"Tab navigation skips the reading content"_s);

    controller->selectChapter(lastChapter);
    controller->selectChapter(firstChapter);
    readyPage(controller);
    require((controller->property("readerChapterId").toString() == firstChapter)
        && (controller->property("readerPageIndex").toInt() == 0), u"Stale chapter request changed the latest reading selection"_s);
    controller->selectReaderPage(-1);
    controller->selectReaderPage(controller->property("readerPageCount").toInt());
    require(controller->property("readerPageIndex").toInt() == 0, u"Invalid page index changed the reader"_s);

    controller->turnReaderPage(1);
    controller->setChapterQuery(u"no-chapter-matches-this-query"_s);
    require(chapters->rowCount() == 0, u"Reopen fault needs an empty filtered chapter list"_s);
    controller->closeReader();
    require(!controller->property("readerOpen").toBool() && controller->property("readerPageUrl").toString().isEmpty(),
        u"Back did not close and release the reading session"_s);
    const QString firstDirectory = QFileInfo(firstPage).absolutePath();
    waitFor([&] { return !QFileInfo::exists(firstDirectory); }, u"Closed chapter cache cleanup"_s);
    controller->openReader();
    const QString reopened = readyPage(controller);
    require((controller->property("selectedVariantId").toString() == exact) && !reopened.isEmpty(),
        u"Reopening substituted the selected translation"_s);
    require(controller->property("chapterQuery").toString().isEmpty() && (chapters->rowCount() == chapterCount),
        u"New reading session inherited a stale chapter query"_s);
    controller->closeReader();
    const QString reopenedDirectory = QFileInfo(reopened).absolutePath();
    waitFor([&] { return !QFileInfo::exists(reopenedDirectory); }, u"Reopened chapter cache cleanup"_s);

    QTemporaryDir fault;
    require(fault.isValid(), u"Could not create the isolated cache fault"_s);
    const QString blockedPath = fault.filePath(u"cache-is-a-file"_s);
    QFile blocked(blockedPath);
    require(blocked.open(QIODevice::WriteOnly), u"Could not prepare the cache fault"_s);
    blocked.close();
    const QByteArray originalTemp = qgetenv("TEMP");
    const QByteArray originalTmp = qgetenv("TMP");
    const auto restoreEnvironment = qScopeGuard([&]
    {
        qputenv("TEMP", originalTemp);
        qputenv("TMP", originalTmp);
    });
    qputenv("TEMP", blockedPath.toLocal8Bit());
    qputenv("TMP", blockedPath.toLocal8Bit());
    controller->openReader();
    waitFor([&] { return !controller->property("readerErrorText").toString().isEmpty(); }, u"Actual cache creation failure"_s);
    require(!controller->property("readerBusy").toBool() && (controller->property("readerPageCount").toInt() > 0),
        u"Cache creation failure did not reach a retryable reader state"_s);
    qputenv("TEMP", originalTemp);
    qputenv("TMP", originalTmp);
    controller->retryReader();
    const QString retried = readyPage(controller);
    require(controller->property("readerChapterId").toString() == firstChapter,
        u"Cache failure retry substituted another chapter"_s);
    controller->closeReader();
    const QString retriedDirectory = QFileInfo(retried).absolutePath();
    waitFor([&] { return !QFileInfo::exists(retriedDirectory); }, u"Retried chapter cache cleanup"_s);
    require(!bindingLoopDetected.load(), u"Reader lifecycle caused a binding loop"_s);
    qInfo().noquote() << u"PASS reader mixed_search=ok exact_translation=ok qml_action=ok decoded_pages=2 chapter_filter=ok reentrant_filter=ok fit=ok stale_chapter=ok bounds=ok cancel_reopen=ok cache_failure_retry=ok cache_cleanup=ok chapters=%1"_s.arg(chapterCount);
}
