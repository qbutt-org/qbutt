// SPDX-License-Identifier: GPL-2.0-or-later
#include "weebcentral.h"

#include <functional>
#include <memory>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QApplication>
#include <QDebug>
#include <QDir>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QImageReader>
#include <QQuickItem>
#include <QScopeGuard>
#include <QTemporaryDir>
#include <QThreadPool>
#include <QTimer>

#include "gui/quick/contentview.h"
#include "qbutt_quick/src/controller.cxxqt.h"

using namespace Qt::Literals::StringLiterals;

namespace
{
    const QString publication = u"weebcentral:01J76XY7E9FNDZ1DBBM6PBJPFK"_s;
    const QString exact = publication + u":pages"_s;

    void require(const bool accepted, const QString &message)
    {
        if (!accepted)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const std::function<bool()> &ready, const QString &operation)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!ready() && (elapsed.elapsed() < 100'000))
        {
            QEventLoop loop;
            QTimer::singleShot(20, &loop, &QEventLoop::quit);
            loop.exec();
        }
        require(ready(), operation + u" timed out"_s);
    }

    QAbstractItemModel *model(QObject *controller, const char *name)
    {
        auto *result = qobject_cast<QAbstractItemModel *>(controller->property(name).value<QObject *>());
        require(result && (result->thread() == qApp->thread()), u"Missing GUI-thread Weeb Central model"_s);
        return result;
    }

    QString value(QAbstractItemModel *model, const int row, const QByteArray &role)
    {
        const int key = model->roleNames().key(role, -1);
        require(key >= 0, u"Missing Weeb Central model role"_s);
        return model->data(model->index(row, 0), key).toString();
    }

    void decoded(Content::ContentView *view, Qbutt::ContentController *controller)
    {
        waitFor([&] { return !controller->property("readerBusy").toBool(); }, u"Weeb Central decoded page"_s);
        require(controller->property("readerOpen").toBool()
            && controller->property("readerErrorText").toString().isEmpty(),
            u"Weeb Central reader failed: "_s + controller->property("readerErrorText").toString());
        const QUrl url = controller->property("readerPageUrl").toUrl();
        require(url.isLocalFile(), u"Weeb Central reader exposed a remote acquisition URL"_s);
        waitFor([&]
        {
            auto *page = view->rootObject()->findChild<QQuickItem *>(u"readerPageImage"_s);
            return page && (page->property("status").toInt() == 1)
                && (page->property("source").toUrl() == url);
        }, u"Weeb Central shown native page"_s);
        QImageReader image(url.toLocalFile());
        require(image.canRead() && !image.read().isNull(), u"Weeb Central native page did not decode"_s);
    }
}

void exerciseWeebCentral()
{
    require(QGuiApplication::platformName() == u"offscreen"_s, u"Weeb Central fixture requires offscreen Qt"_s);
    QTemporaryDir profile;
    require(profile.isValid(), u"Owned Weeb Central profile failed"_s);
    const auto drain = qScopeGuard([]
    {
        if (!QThreadPool::globalInstance()->waitForDone(100'000))
            qFatal("Weeb Central owners did not drain");
    });
    std::unique_ptr<Content::ContentView> view;
    Qbutt::ContentController *controller = nullptr;
    const auto reopen = [&]
    {
        view = std::make_unique<Content::ContentView>(profile.path());
        view->resize(1440, 900);
        view->show();
        controller = view->findChild<Qbutt::ContentController *>();
        require(controller, u"Weeb Central production controller missing"_s);
        waitFor([&] { return controller->property("sourceSettingsState").value<Qbutt::Sources::SettingsState>() == Qbutt::Sources::SettingsState::Ready; }, u"Weeb Central source preferences"_s);
        waitFor([&] { return controller->property("libraryState").value<Qbutt::Library::LibraryState>() == Qbutt::Library::LibraryState::Ready; }, u"Weeb Central Library"_s);
    };
    const auto close = [&]
    {
        view.reset();
        controller = nullptr;
        require(QThreadPool::globalInstance()->waitForDone(100'000), u"Weeb Central owner did not drain"_s);
    };
    reopen();
    auto *sources = model(controller, "sources");
    for (int row = 0; row < sources->rowCount(); ++row)
    {
        const QString id = value(sources, row, "sourceId");
        require(controller->setSourceEnabled(id, id == u"weebcentral"_s), u"Weeb Central source choice rejected"_s);
    }
    controller->search(u"One Piece"_s);
    waitFor([&] { return !controller->property("busy").toBool(); }, u"Weeb Central live search"_s);
    controller->selectWork(publication);
    waitFor([&] { return !controller->property("inspecting").toBool(); }, u"Weeb Central live inspect"_s);
    controller->selectVariant(exact);
    require(controller->property("selectedVariantId").toString() == exact,
        u"Weeb Central exact English publication missing"_s);
    require(controller->property("selectedVariantKind").toString() == u"manga"_s,
        u"Weeb Central has the wrong reader transport"_s);
    require(controller->property("canShareSelected").toBool(), u"Weeb Central public reference cannot be shared"_s);
    require(model(controller, "history")->rowCount() == 0, u"Selection alone created History"_s);
    auto *primary = view->rootObject()->findChild<QQuickItem *>(u"contentDownload"_s);
    require(primary && primary->isEnabled(), u"Weeb Central Read action missing"_s);
    require(QMetaObject::invokeMethod(primary, "clicked"), u"Weeb Central QML Read action failed"_s);
    decoded(view.get(), controller);
    auto *chapters = model(controller, "readerChapters");
    require(chapters->rowCount() > 1000 && value(chapters, 1, "number") == u"2"_s,
        u"Weeb Central full chapter list missing"_s);
    const QString chapter = value(chapters, 1, "chapterId");
    controller->selectChapter(chapter);
    decoded(view.get(), controller);
    require(controller->property("readerChapterId").toString() == chapter,
        u"Weeb Central substituted the nonfirst chapter"_s);
    controller->selectReaderPage(1);
    decoded(view.get(), controller);
    require(controller->property("readerPageIndex").toInt() == 1, u"Weeb Central page turn lost exact position"_s);
    waitFor([&] { return model(controller, "history")->rowCount() == 1; }, u"Weeb Central shown-page History"_s);
    waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Weeb Central reader persistence"_s);

    const QString imageDirectory = qEnvironmentVariable("QBUTT_WEEBCENTRAL_QA_IMAGES");
    if (!imageDirectory.isEmpty())
    {
        require(QDir().mkpath(imageDirectory), u"Weeb Central render directory failed"_s);
        for (const QSize &size : {QSize(1440, 900), QSize(1280, 720)})
        {
            view->resize(size);
            QEventLoop frame;
            QTimer::singleShot(150, &frame, &QEventLoop::quit);
            frame.exec();
            require(view->grabFramebuffer().save(QDir(imageDirectory).filePath(u"weebcentral-%1x%2.png"_s.arg(size.width()).arg(size.height()))), u"Weeb Central render failed"_s);
        }
    }
    close();
    reopen();
    controller->setListMode(Qbutt::Search::ListMode::History);
    controller->selectWork(publication);
    waitFor([&] { return !controller->property("inspecting").toBool(); }, u"Weeb Central History inspect"_s);
    require(controller->property("selectedVariantId").toString() == exact,
        u"Weeb Central History changed the exact edition"_s);
    controller->openReader();
    decoded(view.get(), controller);
    require(controller->property("readerChapterId").toString() == chapter
        && controller->property("readerPageIndex").toInt() == 1,
        u"Weeb Central restart changed the exact chapter or page"_s);
    close();
    qInfo().noquote() << u"PASS weebcentral live_search=ok exact_edition=ok qml_read=ok nonfirst_chapter=ok native_decode=ok full_chapter_list=ok public_reference=ok history_restart=ok"_s;
}
