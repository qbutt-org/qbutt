// SPDX-License-Identifier: GPL-2.0-or-later
#include "worklist.h"

#include <functional>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QAccessible>
#include <QApplication>
#include <QDebug>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QKeyEvent>
#include <QObject>
#include <QQuickItem>
#include <QTimer>

#include "gui/quick/contentview.h"
#include "gui/quick/qml/workfiltermodel.h"
#include "qbutt_quick/src/controller.cxxqt.h"
#include "qbutt_quick/src/models.cxxqt.h"

using namespace Qt::Literals::StringLiterals;

namespace
{
    void require(const bool accepted, const QString &message)
    {
        if (!accepted)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const std::function<bool()> &ready, const QString &operation)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!ready() && (elapsed.elapsed() < 60'000))
        {
            QEventLoop loop;
            QTimer::singleShot(20, &loop, &QEventLoop::quit);
            loop.exec();
        }
        require(ready(), operation + u" timed out"_s);
    }

    QString value(QAbstractItemModel *model, const int row, const QByteArray &name)
    {
        const int role = model->roleNames().key(name, -1);
        require(role >= 0, u"Missing work model role"_s);
        return model->data(model->index(row, 0), role).toString();
    }

    QQuickItem *itemAt(QQuickItem *results, const int row)
    {
        QQuickItem *item = nullptr;
        require(QMetaObject::invokeMethod(results, "itemAtIndex", Q_RETURN_ARG(QQuickItem *, item), Q_ARG(int, row)),
            u"Results delegate lookup failed"_s);
        return item;
    }

    Content::WorkFilterModel *visibleModel(QQuickItem *results, QAbstractItemModel *source)
    {
        auto *proxy = qobject_cast<Content::WorkFilterModel *>(results->property("model").value<QObject *>());
        require(proxy && (proxy->sourceModel() == source), u"Production work proxy uses another source model"_s);
        return proxy;
    }

    void verifyRow(QQuickItem *results, QAbstractItemModel *model, const int row, const QString &selected)
    {
        QQuickItem *item = itemAt(results, row);
        require(item, u"Visible work delegate is missing"_s);
        const QString expectedId = value(model, row, "workId");
        for (const QByteArray &role : {QByteArray("workId"), QByteArray("title"), QByteArray("source"),
            QByteArray("category"), QByteArray("year")})
        {
            const QString expected = value(model, row, role);
            const QString actual = item->property(role).toString();
            require(actual == expected, u"Visible row %1 role %2 differs: model=%3 delegate=%4"_s
                .arg(row).arg(QString::fromLatin1(role)).arg(expected).arg(actual));
        }
        const QString title = value(model, row, "title");
        bool titleRendered = false;
        for (QObject *child : item->findChildren<QObject *>())
            titleRendered |= child->property("text").toString() == title;
        require(titleRendered, u"Delegate label does not render the active title"_s);
        QAccessibleInterface *accessible = QAccessible::queryAccessibleInterface(item);
        require(accessible && (accessible->text(QAccessible::Name) == title)
            && (accessible->state().selected == (expectedId == selected)), u"Accessible work row differs from the active model"_s);
        require(item->property("highlighted").toBool() == (expectedId == selected), u"Work delegate highlight differs from selection"_s);
    }

    void settle(QQuickItem *results)
    {
        require(QMetaObject::invokeMethod(results, "forceLayout"), u"Results layout failed"_s);
        QEventLoop loop;
        QTimer::singleShot(150, &loop, &QEventLoop::quit);
        loop.exec();
    }

    void exerciseHistoryList(Content::ContentView *view, Qbutt::ContentController *controller, QQuickItem *results)
    {
        const QString publication = controller->property("selectedWorkId").toString();
        const QString exact = controller->property("selectedVariantId").toString();
        auto *history = qobject_cast<QAbstractItemModel *>(controller->property("history").value<QObject *>());
        auto *favorites = qobject_cast<QAbstractItemModel *>(controller->property("favorites").value<QObject *>());
        require(history && favorites && (history->rowCount() == 0) && (favorites->rowCount() == 1),
            u"History list fixture is not isolated"_s);
        auto *read = view->rootObject()->findChild<QQuickItem *>(u"contentDownload"_s);
        require(read && read->property("enabled").toBool() && QMetaObject::invokeMethod(read, "clicked"),
            u"Production reading action did not run"_s);
        waitFor([&]
        {
            return (history->rowCount() == 1) && !controller->property("libraryPending").toBool();
        }, u"Rendered page History acknowledgement"_s);
        auto *readerLoader = view->rootObject()->findChild<QQuickItem *>(u"contentReaderLoader"_s);
        require(readerLoader, u"Reader Loader is missing"_s);
        auto *reader = readerLoader->property("item").value<QQuickItem *>();
        require(reader, u"Current reader page is missing"_s);

        QObject confirmationObserver;
        int pageWrites = 0;
        QObject::connect(controller, &Qbutt::ContentController::readerChanged, &confirmationObserver, [&]
        {
            if (controller->property("readerProgressPending").toBool())
                ++pageWrites;
        });
        auto *next = reader->findChild<QQuickItem *>(u"readerNextPage"_s);
        require(next && next->property("enabled").toBool() && QMetaObject::invokeMethod(next, "clicked"),
            u"History fixture could not turn its actual page"_s);
        auto *page = reader->findChild<QQuickItem *>(u"readerPageImage"_s);
        waitFor([&]
        {
            return (pageWrites > 0) && page && page->isVisible() && !controller->property("readerBusy").toBool()
                && (controller->property("readerPageIndex").toInt() == 1)
                && !controller->property("libraryPending").toBool();
        }, u"Second rendered reading page"_s);
        auto *fit = reader->findChild<QQuickItem *>(u"readerWidthFit"_s);
        require(fit && QMetaObject::invokeMethod(fit, "clicked"), u"History reading fit action is missing"_s);
        auto *viewport = reader->findChild<QQuickItem *>(u"readerPageViewport"_s);
        waitFor([&] { return page->isVisible() && viewport && (viewport->property("contentHeight").toReal() > viewport->height()); },
            u"Scrollable rendered history page"_s);
        QKeyEvent down(QEvent::KeyPress, Qt::Key_Down, Qt::NoModifier);
        QCoreApplication::sendEvent(view, &down);
        QKeyEvent released(QEvent::KeyRelease, Qt::Key_Down, Qt::NoModifier);
        QCoreApplication::sendEvent(view, &released);
        waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Exact reading offset"_s);
        const double offset = controller->property("readerScrollOffset").toDouble();
        const QString chapter = controller->property("readerChapterId").toString();
        require((offset > 0) && controller->property("libraryErrorText").toString().isEmpty(),
            u"History reading position was not confirmed"_s);
        auto *back = reader->findChild<QQuickItem *>(u"readerBack"_s);
        require(back && QMetaObject::invokeMethod(back, "clicked"), u"History reading Back action is missing"_s);

        controller->setListMode(Qbutt::Search::ListMode::History);
        settle(results);
        auto *visible = visibleModel(results, history);
        require(results->property("count").toInt() == 1,
            u"Production results did not switch to History"_s);
        verifyRow(results, visible, 0, publication);
        require(QMetaObject::invokeMethod(itemAt(results, 0), "clicked"), u"History row action is missing"_s);
        waitFor([&] { return !controller->property("inspecting").toBool(); }, u"History publication inspection"_s);
        auto *variants = qobject_cast<QAbstractItemModel *>(controller->property("variants").value<QObject *>());
        require(variants && (variants->rowCount() > 1) && (controller->property("selectedVariantId").toString() == exact),
            u"History lost the exact choice or other variants"_s);
        const int writesBeforeReopen = pageWrites;
        require(read->property("enabled").toBool() && QMetaObject::invokeMethod(read, "clicked")
            && controller->property("readerOpen").toBool(), u"History reading action did not open its exact variant"_s);
        reader = readerLoader->property("item").value<QQuickItem *>();
        require(reader, u"Resumed current reader page is missing"_s);
        page = reader->findChild<QQuickItem *>(u"readerPageImage"_s);
        viewport = reader->findChild<QQuickItem *>(u"readerPageViewport"_s);
        auto *canvas = reader->findChild<QQuickItem *>(u"readerCanvas"_s);
        require(page && viewport && canvas, u"Resumed current reader canvas is incomplete"_s);
        waitFor([&]
        {
            return !canvas->property("errorText").toString().isEmpty()
                || (page->isVisible() && !controller->property("readerBusy").toBool());
        }, u"History reading resume"_s);
        require(canvas->property("errorText").toString().isEmpty(), canvas->property("errorText").toString());
        require((controller->property("readerChapterId").toString() == chapter)
            && (controller->property("readerPageIndex").toInt() == 1)
            && (qAbs(controller->property("readerScrollOffset").toDouble() - offset) < 0.000001),
            u"History reopened another position: chapter=%1 page=%2 offset=%3 expected_chapter=%4 expected_page=1 expected_offset=%5"_s
                .arg(controller->property("readerChapterId").toString()).arg(controller->property("readerPageIndex").toInt())
                .arg(controller->property("readerScrollOffset").toDouble()).arg(chapter).arg(offset));
        waitFor([&] { return (pageWrites > writesBeforeReopen) && !controller->property("libraryPending").toBool(); },
            u"Resumed visible page History acknowledgement"_s);
        const double range = viewport->property("contentHeight").toDouble() - viewport->height();
        require((range > 0) && (qAbs(viewport->property("contentY").toDouble() - qMin(offset * page->height(), range)) < 1),
            u"History did not restore the actual visible page position"_s);
        back = reader->findChild<QQuickItem *>(u"readerBack"_s);
        require(back && QMetaObject::invokeMethod(back, "clicked"), u"History resumed Back action is missing"_s);
        settle(results);
        auto *remove = view->rootObject()->findChild<QQuickItem *>(u"contentRemoveHistory"_s);
        require(remove && remove->isVisible() && remove->property("enabled").toBool()
            && QMetaObject::invokeMethod(remove, "clicked"), u"History removal action did not run"_s);
        waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Confirmed History removal"_s);
        settle(results);
        require((history->rowCount() == 0) && (results->property("currentIndex").toInt() == -1)
            && !remove->isVisible() && (favorites->rowCount() == 1) && controller->property("selectedFavorite").toBool(),
            u"History removal retained its action or damaged Favorites"_s);
        controller->setListMode(Qbutt::Search::ListMode::Favorites);
        settle(results);
        verifyRow(results, visibleModel(results, favorites), 0, publication);
    }
}

void exerciseWorkList(Content::ContentView *view)
{
    QAccessible::setActive(true);
    require(view && (view->status() == QQuickWidget::Ready), u"Production work list did not load"_s);
    auto *controller = view->findChild<Qbutt::ContentController *>();
    auto *results = view->rootObject()->findChild<QQuickItem *>(u"contentResults"_s);
    require(controller && results, u"Production work list controller is missing"_s);
    waitFor([&]
    {
        return controller->property("libraryState").value<Qbutt::Library::LibraryState>() == Qbutt::Library::LibraryState::Ready;
    }, u"Isolated library"_s);
    auto *works = qobject_cast<QAbstractItemModel *>(controller->property("works").value<QObject *>());
    auto *favorites = qobject_cast<QAbstractItemModel *>(controller->property("favorites").value<QObject *>());
    require(works && favorites && (favorites->rowCount() == 0), u"Fresh work list profile is not empty"_s);
    controller->search(u"Frieren"_s);
    waitFor([&] { return !controller->property("busy").toBool(); }, u"Live catalog metadata"_s);
    require(controller->property("errorText").toString().isEmpty() && (works->rowCount() > 5),
        u"Catalog did not produce a scrollable work list"_s);
    settle(results);
    auto *visible = visibleModel(results, works);
    verifyRow(results, visible, 0, controller->property("selectedWorkId").toString());
    results->forceActiveFocus();
    for (const Qt::Key key : {Qt::Key_End, Qt::Key_Home})
    {
        QKeyEvent press(QEvent::KeyPress, key, Qt::NoModifier);
        QCoreApplication::sendEvent(view, &press);
        QKeyEvent release(QEvent::KeyRelease, key, Qt::NoModifier);
        QCoreApplication::sendEvent(view, &release);
        settle(results);
        verifyRow(results, visible, key == Qt::Key_End ? visible->rowCount() - 1 : 0,
            controller->property("selectedWorkId").toString());
    }
    const QString publication = u"mangadex:b0b721ff-c388-4486-aa0f-c2b0bb321512"_s;
    const QString exact = publication + u":translation:ru:9e383226-2af3-4af0-a69d-3d3b630212b3"_s;
    controller->selectWork(publication);
    waitFor([&] { return !controller->property("inspecting").toBool(); }, u"Primary publication"_s);
    require(controller->property("errorText").toString().isEmpty(), u"Primary publication inspection failed"_s);
    controller->selectVariant(exact);
    require(controller->property("selectedVariantId").toString() == exact, u"Exact translation unavailable"_s);
    controller->setSelectedFavorite(true);
    waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Save isolated favorite"_s);
    require((favorites->rowCount() == 1) && (value(favorites, 0, "workId") == publication),
        u"Favorite model persisted another publication"_s);
    exerciseHistoryList(view, controller, results);
    controller->setListMode(Qbutt::Search::ListMode::Favorites);
    settle(results);
    visible = visibleModel(results, favorites);
    require(results->property("count").toInt() == 1, u"Results did not switch to Favorites"_s);
    verifyRow(results, visible, 0, publication);
    require(QMetaObject::invokeMethod(itemAt(results, 0), "clicked"), u"Favorite delegate action missing"_s);
    waitFor([&] { return !controller->property("inspecting").toBool(); }, u"Favorite row click"_s);
    require((controller->property("selectedWorkId").toString() == publication)
        && (controller->property("selectedVariantId").toString() == exact), u"Favorite row action changed exact selection"_s);
    controller->setSelectedFavorite(false);
    waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Remove isolated favorite"_s);
    settle(results);
    require((results->property("count").toInt() == 0) && (results->property("currentIndex").toInt() == -1)
        && !itemAt(results, 0), u"Removed favorite retained a visible delegate"_s);
    controller->setListMode(Qbutt::Search::ListMode::Search);
    settle(results);
    visible = visibleModel(results, works);
    const int selectedRow = results->property("currentIndex").toInt();
    require((selectedRow >= 0) && (value(visible, selectedRow, "workId") == publication),
        u"Search return lost selection"_s);
    verifyRow(results, visible, selectedRow, publication);

    for (int row = works->rowCount() - 1; row >= 0; --row)
    {
        controller->selectWork(value(works, row, "workId"));
        controller->setSelectedFavorite(true);
        waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Save reversed favorite"_s);
    }
    controller->selectWork(publication);
    waitFor([&] { return !controller->property("inspecting").toBool(); }, u"Restore exact publication"_s);
    require((controller->property("selectedVariantId").toString() == exact)
        && (favorites->rowCount() == works->rowCount()), u"Equal-count favorites fixture failed"_s);
    const auto *typedWorks = qobject_cast<Qbutt::WorkModel *>(works);
    const auto *typedFavorites = qobject_cast<Qbutt::WorkModel *>(favorites);
    require(typedWorks && typedFavorites
        && (typedWorks->indexOfId(publication) != typedFavorites->indexOfId(publication)),
        u"Equal-count model switch needs different selection indices"_s);
    for (int iteration = 0; iteration < 3; ++iteration)
    {
        for (const auto mode : {Qbutt::Search::ListMode::Favorites, Qbutt::Search::ListMode::Search})
        {
            controller->setListMode(mode);
            settle(results);
            const auto *active = mode == Qbutt::Search::ListMode::Favorites ? typedFavorites : typedWorks;
            auto *activeModel = mode == Qbutt::Search::ListMode::Favorites ? favorites : works;
            visible = visibleModel(results, activeModel);
            const int selectedIndex = visible->indexOfId(publication);
            require(selectedIndex == active->indexOfId(publication), u"All-category proxy changed source order"_s);
            require(results->property("currentIndex").toInt() == selectedIndex,
                u"Equal-count model switch lost the current selection index"_s);
            QQuickItem *selectedItem = itemAt(results, selectedIndex);
            require(selectedItem
                && selectedItem->mapRectToItem(results, selectedItem->boundingRect()).intersects(results->boundingRect()),
                u"Equal-count model switch left the selected work outside the view: mode=%1 iteration=%2 index=%3 id=%4 count=%5 contentY=%6 itemY=%7 viewportHeight=%8"_s
                    .arg(static_cast<int>(mode)).arg(iteration).arg(results->property("currentIndex").toInt())
                    .arg(visible->idAt(results->property("currentIndex").toInt())).arg(visible->rowCount())
                    .arg(results->property("contentY").toDouble()).arg(selectedItem ? selectedItem->y() : -1)
                    .arg(results->height()));
            for (int row = 0; row < activeModel->rowCount(); ++row)
            {
                if (itemAt(results, row))
                    verifyRow(results, visible, row, publication);
            }
        }
    }
    qInfo("PASS worklist visible_roles=ok accessible_rows=ok scroll=ok favorite_swap=ok history_swap=ok history_resume=ok history_remove=ok removal=ok equal_count=ok exact_selection=ok");
}
