// SPDX-License-Identifier: GPL-2.0-or-later
#include "reader-text.h"

#include <functional>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QApplication>
#include <QDebug>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QQuickItem>
#include <QSize>
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
    auto *sources = model(controller, "sources");
    for (int row = 0; row < sources->rowCount(); ++row)
    {
        const QString id = value(sources, row, "sourceId");
        require(controller->setSourceEnabled(id, id == u"syosetu"), u"Source choice was rejected"_s);
    }
    waitFor([&]
    {
        for (int row = 0; row < sources->rowCount(); ++row)
        {
            const bool selected = value(sources, row, "sourceId") == u"syosetu";
            if ((value(sources, row, "pending") == u"true")
                || (value(sources, row, "enabled") != (selected ? u"true" : u"false")))
            {
                return false;
            }
        }
        return true;
    }, u"Syosetu source selection"_s);

    const auto openWork = [&](const QString &query, const QString &publication)
    {
        controller->setListMode(Qbutt::Search::ListMode::Search);
        controller->search(query);
        waitFor([&] { return !controller->property("busy").toBool(); }, u"Syosetu search"_s);
        auto *works = model(controller, "works");
        bool found = false;
        for (int row = 0; row < works->rowCount(); ++row)
            found |= value(works, row, "workId") == publication;
        require(found, u"Exact Syosetu work is missing: "_s + publication);
        controller->selectWork(publication);
        waitFor([&] { return !controller->property("inspecting").toBool(); }, u"Syosetu inspection"_s);
        require(controller->property("errorText").toString().isEmpty(),
            controller->property("errorText").toString());
        controller->selectVariant(publication + u":original"_s);
        require(controller->property("selectedVariantId").toString() == publication + u":original"_s,
            u"Exact original variant was substituted"_s);
        auto *pane = view->rootObject()->findChild<QObject *>(u"contentWorkPane"_s);
        require(pane && QMetaObject::invokeMethod(pane, "openSelected"), u"QML Read action is missing"_s);
        readyText(controller);
    };

    int chapterCount = 0;
    if (!imageOnly)
    {
        openWork(u"Ｒｅ：ゼロから始める異世界生活"_s, u"syosetu:n2267be"_s);
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

        openWork(u"本好きの下剋上"_s, u"syosetu:n4830bu"_s);
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
    openWork(u"ソーサリースフィア"_s, illustrated);
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
