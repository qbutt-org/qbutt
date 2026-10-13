// SPDX-License-Identifier: GPL-2.0-or-later
#include "reader-text.h"

#include <functional>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QApplication>
#include <QCoreApplication>
#include <QDebug>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QImage>
#include <QJsonDocument>
#include <QJsonObject>
#include <QKeyEvent>
#include <QQuickItem>
#include <QScopeGuard>
#include <QSize>
#include <QSqlDatabase>
#include <QSqlError>
#include <QSqlQuery>
#include <QTimer>
#include <QUrl>

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

    void waitFor(const std::function<bool()> &condition, const QString &operation,
        const int timeoutMs = 100'000)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        QEventLoop loop;
        QTimer timer;
        timer.setInterval(20);
        QObject::connect(&timer, &QTimer::timeout, &loop, [&]
        {
            if (condition() || (elapsed.elapsed() >= timeoutMs))
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
        require(result && (result->thread() == qApp->thread()), u"Missing GUI-thread reader model"_s);
        return result;
    }

    QString value(QAbstractItemModel *model, const int row, const QByteArray &name)
    {
        const int role = model->roleNames().key(name, -1);
        require(role >= 0, u"Missing reader model role"_s);
        return model->data(model->index(row, 0), role).toString();
    }

    void readyText(Qbutt::ContentController *controller)
    {
        waitFor([&] { return !controller->property("readerBusy").toBool(); }, u"Original text chapter"_s);
        require(controller->property("readerOpen").toBool()
            && controller->property("readerIsText").toBool(), u"Original reader did not open"_s);
        require(controller->property("readerErrorText").toString().isEmpty(),
            controller->property("readerErrorText").toString());
        require(model(controller, "readerTextBlocks")->rowCount() > 0, u"Original chapter is empty"_s);
    }

    QQuickItem *inlineImage(QQuickItem *item, const QString &imageId)
    {
        const QUrl source = item->property("source").toUrl();
        if ((source.scheme() == u"image") && (source.host() == u"reader")
            && source.path().endsWith(u"/"_s + imageId))
        {
            return item;
        }
        for (auto *child : item->childItems())
        {
            if (auto *image = inlineImage(child, imageId))
                return image;
        }
        return nullptr;
    }

    QQuickItem *namedItem(QQuickItem *item, const QString &name)
    {
        if (item->objectName() == name)
            return item;
        for (auto *child : item->childItems())
        {
            if (auto *match = namedItem(child, name))
                return match;
        }
        return nullptr;
    }

    QQuickItem *visibleReaderItem(QObject *root, const QString &name)
    {
        for (auto *item : root->findChildren<QQuickItem *>(name))
        {
            if (item->isVisible() && item->height() > 0)
                return item;
        }
        return nullptr;
    }

    void pressReaderKey(Content::ContentView *view, const Qt::Key key)
    {
        QKeyEvent press(QEvent::KeyPress, key, Qt::NoModifier);
        QCoreApplication::sendEvent(view, &press);
        QKeyEvent release(QEvent::KeyRelease, key, Qt::NoModifier);
        QCoreApplication::sendEvent(view, &release);
    }

    void selectOnlySource(Qbutt::ContentController *controller, const QString &sourceId)
    {
        auto *sources = model(controller, "sources");
        for (int row = 0; row < sources->rowCount(); ++row)
        {
            const QString id = value(sources, row, "sourceId");
            require(controller->setSourceEnabled(id, id == sourceId), u"Source choice was rejected"_s);
        }
        waitFor([&]
        {
            for (int row = 0; row < sources->rowCount(); ++row)
            {
                const bool selected = value(sources, row, "sourceId") == sourceId;
                if ((value(sources, row, "pending") == u"true")
                    || (value(sources, row, "enabled") != (selected ? u"true" : u"false")))
                {
                    return false;
                }
            }
            return true;
        }, sourceId + u" source selection"_s);
    }

    void openTextWork(Content::ContentView *view, Qbutt::ContentController *controller,
        const QString &query, const QString &publication, const QString &variant)
    {
        controller->setListMode(Qbutt::Search::ListMode::Search);
        controller->search(query);
        waitFor([&] { return !controller->property("busy").toBool(); }, publication + u" search"_s);
        auto *works = model(controller, "works");
        bool found = false;
        for (int row = 0; row < works->rowCount(); ++row)
            found |= value(works, row, "workId") == publication;
        require(found, u"Exact original work is missing: "_s + publication);
        controller->selectWork(publication);
        waitFor([&] { return !controller->property("inspecting").toBool(); }, publication + u" inspection"_s);
        require(controller->property("errorText").toString().isEmpty(),
            controller->property("errorText").toString());
        controller->selectVariant(variant);
        require(controller->property("selectedVariantId").toString() == variant,
            u"Exact text variant was substituted"_s);
        auto *pane = view->rootObject()->findChild<QObject *>(u"contentWorkPane"_s);
        require(pane && QMetaObject::invokeMethod(pane, "openSelected"), u"QML Read action is missing"_s);
        readyText(controller);
    }

    struct PublicTextCase
    {
        QString source;
        QString query;
        QString publication;
        QString variant;
        QString firstChapter;
        QString secondChapter;
        QString ruby;
        int minimumSecondBlocks;
    };

    PublicTextCase publicTextCase(const QString &sourceId)
    {
        if (sourceId == u"kakuyomu"_s)
        {
            return {sourceId, u"桃太郎"_s, u"kakuyomu:11054822663254506405"_s,
                u"kakuyomu:11054822663254506405:original"_s,
                u"11054822663254506405/11054822663254569979"_s,
                u"11054822663254506405/11054822663260874365"_s,
                u"高槻(たかつき)"_s, 100};
        }
        require(sourceId == u"ranobehub"_s, u"Unknown public text source"_s);
        return {sourceId, u"Overlord"_s, u"ranobehub:206"_s,
            u"ranobehub:206:branch:main"_s, u"41634"_s, u"41635"_s, {}, 20};
    }

    struct SavedTextPosition
    {
        QString chapter;
        QString block;
        double offset;
    };

    SavedTextPosition savedTextPosition(const QString &profileDir)
    {
        const QString connectionName = u"public-text-reader-history"_s;
        auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, connectionName);
        const auto closeDatabase = qScopeGuard([&]
        {
            database.close();
            database = QSqlDatabase();
            QSqlDatabase::removeDatabase(connectionName);
        });
        database.setDatabaseName(profileDir + u"/content-library.sqlite"_s);
        require(database.open(), database.lastError().text());
        QSqlQuery query(database);
        require(query.exec(u"SELECT entry FROM library"_s) && query.next(),
            u"Confirmed text history is missing from SQLite"_s);
        const auto entry = QJsonDocument::fromJson(query.value(0).toString().toUtf8()).object();
        require(!query.next(), u"Reader profile contains unrelated works"_s);
        const auto progress = entry.value(u"reader_progress"_s).toObject();
        const auto position = progress.value(u"position"_s).toObject();
        require(position.value(u"kind"_s).toString() == u"text"_s,
            u"Reader did not persist a text position"_s);
        return {progress.value(u"chapter"_s).toObject().value(u"id"_s).toString(),
            position.value(u"block_id"_s).toString(), position.value(u"offset"_s).toDouble(-1)};
    }

    void saveRender(Content::ContentView *view, const QString &profileDir,
        const QString &name, const QSize &size)
    {
        view->resize(size);
        QEventLoop frame;
        QTimer::singleShot(150, &frame, &QEventLoop::quit);
        frame.exec();
        const QImage image = view->grabFramebuffer();
        require(image.size() == size && image.save(profileDir + u"/"_s + name),
            u"Native text reader render could not be saved"_s);
    }
}

void exerciseSyosetuReader(Content::ContentView *view, const bool imageOnly)
{
    require(view && (view->status() == QQuickWidget::Ready), u"Production reader view did not load"_s);
    auto *controller = view->findChild<Qbutt::ContentController *>();
    require(controller, u"Production content controller is missing"_s);
    waitFor([&]
    {
        return (controller->getLibraryState() == Qbutt::Library::LibraryState::Ready)
            && (controller->property("sourceSettingsState").value<Qbutt::Sources::SettingsState>()
                == Qbutt::Sources::SettingsState::Ready);
    }, u"Isolated reader library"_s);
    selectOnlySource(controller, u"syosetu"_s);

    int chapterCount = 0;
    if (!imageOnly)
    {
        openTextWork(view, controller, u"Ｒｅ：ゼロから始める異世界生活"_s,
            u"syosetu:n2267be"_s, u"syosetu:n2267be:original"_s);
        auto *chapters = model(controller, "readerChapters");
        chapterCount = chapters->rowCount();
        require(chapterCount > 700 && value(chapters, 1, "chapterId") == u"2",
            u"Native table of contents is incomplete or reordered"_s);
        controller->selectChapter(u"2"_s);
        readyText(controller);
        require(controller->property("readerChapterId").toString() == u"2", u"Nonfirst chapter was substituted"_s);
        auto *blocks = model(controller, "readerTextBlocks");
        bool ruby = false;
        for (int row = 0; row < blocks->rowCount(); ++row)
            ruby |= value(blocks, row, "plainText").contains(u"菜月(なつき)"_s);
        require(ruby, u"Original ruby did not reach the native text model"_s);
        controller->setChapterQuery(u"ギザ十"_s);
        require((chapters->rowCount() == 1) && (value(chapters, 0, "chapterId") == u"2"),
            u"Native chapter filtering lost the exact nonfirst chapter"_s);
        controller->setChapterQuery(u""_s);
        require(chapters->rowCount() == chapterCount, u"Chapter filtering discarded the complete table of contents"_s);
        controller->closeReader();

        openTextWork(view, controller, u"本好きの下剋上"_s,
            u"syosetu:n4830bu"_s, u"syosetu:n4830bu:original"_s);
        blocks = model(controller, "readerTextBlocks");
        bool notes = false;
        for (int row = 0; row < blocks->rowCount(); ++row)
            notes |= value(blocks, row, "blockKind") == u"2";
        require(notes, u"Author notes did not reach the native reader"_s);
        controller->closeReader();
    }

    qsizetype imageBytes = 0;
    QString imageId;
    QObject observation;
    QObject::connect(controller, &Qbutt::ContentController::readerAsset, &observation,
        [&](const quint64, const QString &id, const QByteArray &bytes)
    {
        imageId = id;
        imageBytes = bytes.size();
    });
    const QString illustrated = u"syosetu:n7653fr"_s;
    openTextWork(view, controller, u"ソーサリースフィア"_s, illustrated,
        illustrated + u":original"_s);
    controller->selectChapter(u"39"_s);
    readyText(controller);
    require(controller->property("readerChapterId").toString() == u"39", u"Illustrated chapter was substituted"_s);
    require(imageBytes == 143'156 && !imageId.isEmpty(), u"Exact illustration bytes did not reach Qt"_s);
    auto *blocks = model(controller, "readerTextBlocks");
    QString imageBlock;
    for (int row = 0; row < blocks->rowCount(); ++row)
    {
        if (value(blocks, row, "imageId") == imageId)
            imageBlock = value(blocks, row, "blockId");
    }
    require(!imageBlock.isEmpty(), u"Native illustration block is missing"_s);
    const auto generation = controller->property("readerTextGeneration").toULongLong();
    require(controller->setReaderTextViewport(generation, imageBlock, 0, true),
        u"Illustration viewport was rejected"_s);
    controller->readerTextPositionRequested(generation, imageBlock, 0);
    QQuickItem *image = nullptr;
    waitFor([&]
    {
        image = inlineImage(view->rootObject(), imageId);
        return image && image->isVisible() && (image->property("status").toInt() == 1)
            && (image->property("paintedWidth").toReal() > 0)
            && (image->property("paintedHeight").toReal() > 0);
    }, u"QML inline illustration decode"_s);
    const QSize paintedSize(image->property("paintedWidth").toInt(),
        image->property("paintedHeight").toInt());
    require(controller->setReaderTextTranslation(false)
        && !controller->property("readerTranslationRequested").toBool(), u"Original text action failed"_s);
    auto *history = model(controller, "history");
    waitFor([&]
    {
        if (controller->property("readerProgressPending").toBool())
            return false;
        for (int row = 0; row < history->rowCount(); ++row)
        {
            if (value(history, row, "workId") == illustrated)
                return true;
        }
        return false;
    }, u"Confirmed text history"_s);
    require(controller->property("readerProgressErrorText").toString().isEmpty(), u"Reader progress write failed"_s);
    controller->closeReader();
    controller->setListMode(Qbutt::Search::ListMode::History);
    controller->selectWork(illustrated);
    waitFor([&] { return !controller->property("inspecting").toBool(); }, u"History publication refresh"_s);
    controller->openReader();
    readyText(controller);
    require(controller->property("readerChapterId").toString() == u"39"
        && controller->property("selectedVariantId").toString() == illustrated + u":original"_s,
        u"History reopening lost the exact illustrated chapter"_s);
    controller->closeReader();
    if (imageOnly)
    {
        qInfo().noquote() << u"PASS syosetu_image chapter=39 image_bytes=%1 image_ready=%2x%3 original=ok history_reopen=39"_s
            .arg(imageBytes).arg(paintedSize.width()).arg(paintedSize.height());
    }
    else
    {
        qInfo().noquote() << u"PASS syosetu_reader toc=%1 nonfirst=2 ruby=ok notes=ok image_bytes=%2 image_ready=%3x%4 original=ok history_reopen=39"_s
            .arg(chapterCount).arg(imageBytes).arg(paintedSize.width()).arg(paintedSize.height());
    }
}

void exerciseEstarReader(Content::ContentView *view)
{
    require(view && (view->status() == QQuickWidget::Ready), u"Production reader view did not load"_s);
    auto *controller = view->findChild<Qbutt::ContentController *>();
    require(controller, u"Production content controller is missing"_s);
    waitFor([&]
    {
        return (controller->getLibraryState() == Qbutt::Library::LibraryState::Ready)
            && (controller->property("sourceSettingsState").value<Qbutt::Sources::SettingsState>()
                == Qbutt::Sources::SettingsState::Ready);
    }, u"Isolated Estar reader library"_s);
    selectOnlySource(controller, u"estar"_s);

    const QString publication = u"estar:26510966"_s;
    const QString chapter = u"26510966/6"_s;
    openTextWork(view, controller, u"下の階の消防士"_s, publication,
        publication + u":original"_s);
    auto *chapters = model(controller, "readerChapters");
    require(chapters->rowCount() == 31 && value(chapters, 1, "chapterId") == chapter,
        u"Native Estar table of contents is incomplete or reordered"_s);
    controller->selectChapter(chapter);
    readyText(controller);
    require(controller->property("readerChapterId").toString() == chapter,
        u"Nonfirst Estar chapter was substituted"_s);
    auto *blocks = model(controller, "readerTextBlocks");
    require(blocks->rowCount() >= 3, u"Nonfirst Estar chapter is incomplete"_s);
    const int blockCount = blocks->rowCount();
    QString plainText;
    for (int row = 0; row < blocks->rowCount(); ++row)
        plainText += value(blocks, row, "plainText");
    require(plainText.size() > 2'000, u"Nonfirst Estar chapter lost its text"_s);

    const QString firstBlock = value(blocks, 0, "blockId");
    auto *viewport = view->rootObject()->findChild<QQuickItem *>(u"readerTextViewport"_s);
    require(viewport && viewport->isVisible(), u"Native text viewport is missing"_s);
    waitFor([&]
    {
        auto *item = namedItem(viewport, u"readerTextBlock_%1"_s.arg(firstBlock));
        return item && item->isVisible() && item->height() > 0;
    }, u"Estar text rendered in QML"_s);

    const auto generation = controller->property("readerTextGeneration").toULongLong();
    require(controller->setReaderTextViewport(generation, firstBlock, 0, true),
        u"Estar viewport was rejected"_s);
    controller->readerTextPositionRequested(generation, firstBlock, 0);
    auto *history = model(controller, "history");
    waitFor([&]
    {
        if (controller->property("readerProgressPending").toBool())
            return false;
        for (int row = 0; row < history->rowCount(); ++row)
        {
            if (value(history, row, "workId") == publication)
                return true;
        }
        return false;
    }, u"Confirmed Estar text history"_s);
    require(controller->property("readerProgressErrorText").toString().isEmpty(),
        u"Estar reader progress write failed"_s);
    controller->closeReader();
    controller->setListMode(Qbutt::Search::ListMode::History);
    controller->selectWork(publication);
    waitFor([&] { return !controller->property("inspecting").toBool(); }, u"Estar history refresh"_s);
    controller->openReader();
    readyText(controller);
    require(controller->property("readerChapterId").toString() == chapter
        && controller->property("selectedVariantId").toString() == publication + u":original"_s,
        u"Estar history reopening lost the exact original chapter"_s);
    controller->closeReader();
    qInfo().noquote() << u"PASS estar_reader toc=31 nonfirst=%1 blocks=%2 text=%3 qml=ok history_reopen=%1"_s
        .arg(chapter).arg(blockCount).arg(plainText.size());
}

void exercisePublicTextReader(Content::ContentView *view, const QString &profileDir,
    const QString &sourceId, const bool reopenOnly)
{
    require(view && view->status() == QQuickWidget::Ready,
        u"Production text reader view did not load"_s);
    auto *controller = view->findChild<Qbutt::ContentController *>();
    require(controller, u"Production content controller is missing"_s);
    waitFor([&]
    {
        return controller->getLibraryState() == Qbutt::Library::LibraryState::Ready
            && controller->property("sourceSettingsState").value<Qbutt::Sources::SettingsState>()
                == Qbutt::Sources::SettingsState::Ready;
    }, u"Isolated public text reader library"_s);

    const PublicTextCase selected = publicTextCase(sourceId);
    if (!reopenOnly)
    {
        selectOnlySource(controller, selected.source);
        openTextWork(view, controller, selected.query, selected.publication, selected.variant);
        auto *chapters = model(controller, "readerChapters");
        require(chapters->rowCount() >= 2
            && value(chapters, 0, "chapterId") == selected.firstChapter
            && value(chapters, 1, "chapterId") == selected.secondChapter,
            u"Exact public text table of contents is missing or reordered"_s);

        if (sourceId == u"ranobehub"_s)
        {
            auto *blocks = model(controller, "readerTextBlocks");
            require(controller->property("readerChapterId").toString() == selected.firstChapter
                && blocks->rowCount() == 5,
                u"Image-only RanobeHub first chapter did not open"_s);
            for (int row = 0; row < blocks->rowCount(); ++row)
                require(!value(blocks, row, "imageId").isEmpty(),
                    u"RanobeHub image was lost from the native text model"_s);
            const QString firstImage = value(blocks, 0, "imageId");
            waitFor([&]
            {
                auto *image = inlineImage(view->rootObject(), firstImage);
                return image && image->isVisible() && image->property("status").toInt() == 1
                    && image->property("paintedWidth").toReal() > 0;
            }, u"RanobeHub first illustration decoded in QML"_s, 10'000);
            saveRender(view, profileDir, u"ranobehub-images-1280x720.png"_s, QSize(1280, 720));
        }

        if (!selected.ruby.isEmpty())
        {
            require(controller->property("readerChapterId").toString() == selected.firstChapter,
                u"First Kakuyomu chapter was substituted"_s);
            auto *blocks = model(controller, "readerTextBlocks");
            QString rubyBlock;
            for (int row = 0; row < blocks->rowCount(); ++row)
            {
                if (value(blocks, row, "plainText").contains(selected.ruby))
                    rubyBlock = value(blocks, row, "blockId");
            }
            require(!rubyBlock.isEmpty(), u"Live Kakuyomu ruby did not reach the Qt text model"_s);
            view->resize(QSize(1280, 720));
            QEventLoop layout;
            QTimer::singleShot(150, &layout, &QEventLoop::quit);
            layout.exec();
            auto *viewport = view->rootObject()->findChild<QQuickItem *>(u"readerTextViewport"_s);
            require(viewport, u"Native text viewport is missing"_s);
            auto *scroll = view->rootObject()->findChild<QQuickItem *>(u"readerTextScroll"_s);
            require(scroll && scroll->isVisible(), u"Native ruby scroll area is missing"_s);
            scroll->forceActiveFocus(Qt::TabFocusReason);
            require(scroll->hasActiveFocus(), u"Ruby chapter did not focus its scroll area"_s);
            pressReaderKey(view, Qt::Key_End);
            waitFor([&] { return viewport->property("contentY").toReal() > 0; },
                u"Kakuyomu keyboard End scroll"_s, 10'000);
            for (int step = 0; step < 12; ++step)
            {
                auto *item = namedItem(viewport, u"readerTextBlock_%1"_s.arg(rubyBlock));
                if (item && item->isVisible() && item->height() > 0)
                {
                    const qreal top = item->mapToItem(viewport, QPointF()).y();
                    if (top < viewport->height() && top + item->height() > 0)
                        break;
                }
                pressReaderKey(view, Qt::Key_PageUp);
                QEventLoop frame;
                QTimer::singleShot(100, &frame, &QEventLoop::quit);
                frame.exec();
            }
            waitFor([&]
            {
                auto *item = namedItem(viewport, u"readerTextBlock_%1"_s.arg(rubyBlock));
                if (!item || !item->isVisible() || item->height() <= 0)
                    return false;
                const qreal top = item->mapToItem(viewport, QPointF()).y();
                return top < viewport->height() && top + item->height() > 0;
            }, u"Live Kakuyomu ruby block rendered in QML"_s, 10'000);
            saveRender(view, profileDir, u"kakuyomu-ruby-1280x720.png"_s, QSize(1280, 720));
        }

        controller->selectChapter(selected.secondChapter);
        readyText(controller);
        require(controller->property("readerChapterId").toString() == selected.secondChapter,
            u"Nonfirst public text chapter was substituted"_s);
        auto *blocks = model(controller, "readerTextBlocks");
        require(blocks->rowCount() >= selected.minimumSecondBlocks,
            u"Nonfirst public text chapter is incomplete"_s);

        saveRender(view, profileDir, sourceId + u"-chapter-1440x900.png"_s, QSize(1440, 900));
        auto *scroll = view->rootObject()->findChild<QQuickItem *>(u"readerTextScroll"_s);
        auto *viewport = view->rootObject()->findChild<QQuickItem *>(u"readerTextViewport"_s);
        require(scroll && viewport && scroll->isVisible() && viewport->isVisible(),
            u"Native text scroll area is missing"_s);
        waitFor([&]
        {
            return viewport->property("contentHeight").toReal() > viewport->height()
                && !controller->property("readerTextAnchor").toString().isEmpty();
        }, u"Scrollable public chapter layout"_s, 10'000);
        const QString initialBlock = controller->property("readerTextAnchor").toString();
        const double initialOffset = controller->property("readerTextOffset").toDouble();
        const qreal initialY = viewport->property("contentY").toReal();
        scroll->forceActiveFocus(Qt::TabFocusReason);
        require(scroll->hasActiveFocus(), u"Public text did not focus its scroll area"_s);
        for (int step = 0; step < 4; ++step)
        {
            pressReaderKey(view, Qt::Key_PageDown);
            QEventLoop frame;
            QTimer::singleShot(100, &frame, &QEventLoop::quit);
            frame.exec();
        }
        waitFor([&]
        {
            return viewport->property("contentY").toReal() > initialY
                && (controller->property("readerTextAnchor").toString() != initialBlock
                    || qAbs(controller->property("readerTextOffset").toDouble() - initialOffset) > 0.05)
                && !controller->property("readerProgressPending").toBool();
        }, u"Keyboard scroll anchor persistence"_s, 10'000);
        require(controller->property("readerProgressErrorText").toString().isEmpty(),
            u"Public text reader progress write failed"_s);
        saveRender(view, profileDir, sourceId + u"-scrolled-1440x900.png"_s, QSize(1440, 900));
        controller->closeReader();
    }

    const SavedTextPosition saved = savedTextPosition(profileDir);
    require(saved.chapter == selected.secondChapter && !saved.block.isEmpty()
        && saved.offset >= 0 && saved.offset <= 1,
        u"SQLite lost the exact nonfirst text chapter or scroll anchor"_s);
    controller->setListMode(Qbutt::Search::ListMode::History);
    auto *history = model(controller, "history");
    waitFor([&]
    {
        for (int row = 0; row < history->rowCount(); ++row)
        {
            if (value(history, row, "workId") == selected.publication)
                return true;
        }
        return false;
    }, u"Confirmed public text history"_s);
    controller->selectWork(selected.publication);
    waitFor([&] { return !controller->property("inspecting").toBool(); },
        u"Public text history inspection"_s);
    require(controller->property("errorText").toString().isEmpty(),
        controller->property("errorText").toString());
    controller->selectVariant(selected.variant);
    require(controller->property("selectedVariantId").toString() == selected.variant,
        u"History substituted the exact text variant"_s);
    auto *pane = view->rootObject()->findChild<QObject *>(u"contentWorkPane"_s);
    require(pane && QMetaObject::invokeMethod(pane, "openSelected"),
        u"History QML Read action is missing"_s);
    readyText(controller);
    require(controller->property("readerChapterId").toString() == saved.chapter
        && controller->property("readerTextAnchor").toString() == saved.block
        && qAbs(controller->property("readerTextOffset").toDouble() - saved.offset) < 0.001,
        u"History reopening lost the chapter or exact text anchor"_s);
    QQuickItem *viewport = nullptr;
    waitFor([&]
    {
        viewport = visibleReaderItem(view->rootObject(), u"readerTextViewport"_s);
        return viewport && viewport->property("count").toInt() > 0;
    }, u"Reopened visible text viewport"_s, 10'000);
    waitFor([&]
    {
        auto *item = namedItem(viewport, u"readerTextBlock_%1"_s.arg(saved.block));
        if (!item || !item->isVisible() || item->height() <= 0)
            return false;
        const qreal top = item->mapToItem(viewport, QPointF()).y();
        return top < viewport->height() && top + item->height() > 0;
    }, u"Saved text anchor rendered after History reopen"_s, 10'000);
    saveRender(view, profileDir,
        sourceId + (reopenOnly ? u"-restart-1440x900.png"_s : u"-history-1440x900.png"_s),
        QSize(1440, 900));
    const int chapterCount = model(controller, "readerChapters")->rowCount();
    const int blockCount = model(controller, "readerTextBlocks")->rowCount();
    controller->closeReader();
    qInfo().noquote() << u"PASS %1_reader phase=%2 chapters=%3 nonfirst=%4 blocks=%5 anchor=ok history=ok qml=ok"_s
        .arg(sourceId, reopenOnly ? u"restart"_s : u"live"_s)
        .arg(chapterCount).arg(selected.secondChapter).arg(blockCount);
}
