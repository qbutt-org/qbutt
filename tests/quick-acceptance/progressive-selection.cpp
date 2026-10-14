// SPDX-License-Identifier: GPL-2.0-or-later
#include "progressive-selection.h"

#include <functional>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QApplication>
#include <QDebug>
#include <QDir>
#include <QElapsedTimer>
#include <QEventLoop>
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
    void require(const bool condition, const QString &message)
    {
        if (!condition)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const std::function<bool()> &ready, const QString &operation)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!ready() && (elapsed.elapsed() < 10'000))
        {
            QEventLoop loop;
            QTimer::singleShot(10, &loop, &QEventLoop::quit);
            loop.exec();
        }
        require(ready(), operation + u" timed out"_s);
    }

    QAbstractItemModel *model(Qbutt::ContentController &controller, const char *name)
    {
        auto *result = qobject_cast<QAbstractItemModel *>(controller.property(name).value<QObject *>());
        require(result, u"Production model is missing"_s);
        return result;
    }

    QString value(QAbstractItemModel *model, const int row, const QByteArray &role)
    {
        const int id = model->roleNames().key(role, -1);
        require(id >= 0, u"Production role is missing"_s);
        return model->data(model->index(row, 0), id).toString();
    }
}

void exerciseProgressiveSelection()
{
    const bool failure = qEnvironmentVariableIntValue("QBUTT_PROGRESSIVE_FAILURE") == 1;
    const bool initial = qEnvironmentVariableIntValue("QBUTT_PROGRESSIVE_INITIAL") == 1;
    const bool missing = qEnvironmentVariableIntValue("QBUTT_PROGRESSIVE_MISSING") == 1;
    const bool pointer = qEnvironmentVariableIntValue("QBUTT_PROGRESSIVE_POINTER") == 1;
    QTemporaryDir profile;
    require(profile.isValid(), u"Progressive profile failed"_s);
    const auto drainOwners = qScopeGuard([]
    {
        if (!QThreadPool::globalInstance()->waitForDone(10'000))
            qFatal("Progressive library owner did not drain");
    });
    Content::ContentView view(profile.path());
    const QString renderDirectory = qEnvironmentVariable("QBUTT_PROGRESSIVE_RENDER_DIR");
    if (!renderDirectory.isEmpty())
        require(QDir().mkpath(renderDirectory), u"Render directory is unavailable"_s);
    if (pointer || !renderDirectory.isEmpty())
    {
        view.setWindowTitle(u"qbutt — progressive variants"_s);
        view.resize(1440, 900);
        view.show();
    }
    auto *controller = view.findChild<Qbutt::ContentController *>(u"contentController"_s);
    require(controller, u"Production controller is missing"_s);
    waitFor([&]
    {
        return controller->property("sourceSettingsState").value<Qbutt::Sources::SettingsState>()
            == Qbutt::Sources::SettingsState::Ready;
    }, u"Source settings"_s);

    auto *sources = model(*controller, "sources");
    for (int row = 0; row < sources->rowCount(); ++row)
    {
        const QString id = value(sources, row, "sourceId");
        if ((id != u"aniliberty"_s) && (id != u"yani"_s))
            require(controller->setSourceEnabled(id, false), u"Source isolation failed"_s);
    }
    waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Source writes"_s);

    auto *works = model(*controller, "works");
    auto *variants = model(*controller, "variants");
    controller->search(u"Generated progressive"_s);
    waitFor([&] { return works->rowCount() == 1; }, u"Fast search publication"_s);
    if (initial)
        waitFor([&] { return !controller->property("busy").toBool(); }, u"Both search publications"_s);
    const QString work = value(works, 0, "workId");
    controller->selectWork(work);
    waitFor([&]
    {
        return (controller->property("inspecting").toBool() == initial) && (variants->rowCount() == 2);
    }, u"Fast publication variants"_s);
    require(controller->property("selectedVariantId").toString() == value(variants, 0, "variantId"),
        u"First ready variant was not selected"_s);
    const QString selected = value(variants, 1, "variantId");
    controller->selectVariant(selected);
    qInfo("FIXTURE release_late_search");
    waitFor([&] { return controller->property("inspecting").toBool(); }, u"Late inspection started"_s);
    require(controller->property("selectedVariantState").value<Qbutt::Search::VariantSelectionState>()
        == Qbutt::Search::VariantSelectionState::Available,
        u"A pending source blocked the ready variant"_s);
    auto *download = view.rootObject()->findChild<QQuickItem *>(u"contentDownload"_s);
    require(download && download->isEnabled(), u"Ready variant action is disabled"_s);
    if (!renderDirectory.isEmpty())
    {
        QEventLoop frame;
        QTimer::singleShot(200, &frame, &QEventLoop::quit);
        frame.exec();
        require(view.grabFramebuffer().save(QDir(renderDirectory).filePath(u"ready-during-inspection.png"_s)),
            u"Progressive product render failed"_s);
    }

    int handoffs = 0;
    QString magnet;
    const QMetaObject::Connection capture = QObject::connect(controller, &Qbutt::ContentController::torrentReady,
        [&handoffs, &magnet](const QString &url)
        {
            ++handoffs;
            magnet = url;
        });
    const auto disconnectCapture = qScopeGuard([&] { QObject::disconnect(capture); });
    if (pointer)
        qInfo("FIXTURE click_ready_action");
    else
        controller->resolveVariant(selected);
    waitFor([&] { return handoffs == 1; }, u"Ready variant action during inspection"_s);
    require(magnet == u"magnet:?xt=urn:btih:0000000000000000000000000000000000009662"_s,
        u"Another variant was resolved"_s);
    require(controller->property("inspecting").toBool(), u"Inspection was cancelled by the action"_s);
    qInfo("FIXTURE release_late_inspection");

    if (failure)
    {
        waitFor([&]
        {
            for (int row = 0; row < sources->rowCount(); ++row)
            {
                if (value(sources, row, "sourceId") == u"yani"_s)
                    return value(sources, row, "failed") == u"true"_s;
            }
            return false;
        }, u"Late-publication failure"_s);
    }
    else
    {
        waitFor([&] { return variants->rowCount() == 3; }, u"Automatic late-publication inspection"_s);
    }
    waitFor([&]
    {
        return !controller->property("busy").toBool() && !controller->property("inspecting").toBool()
            && !controller->property("libraryPending").toBool();
    }, u"Progressive completion"_s);
    require(works->rowCount() == 1, u"Late source split the confirmed work"_s);
    require(controller->property("selectedWorkId").toString() == work,
        u"Late source replaced the selected work"_s);
    require(controller->property("selectedVariantId").toString() == selected,
        u"Late source replaced the exact selected variant"_s);
    require(value(variants, 1, "variantId") == selected, u"Late source reordered the selected row"_s);
    if (failure)
        require(variants->rowCount() == 2, u"Failed source replaced existing variants"_s);
    else
        require(value(variants, 2, "sourceId") == u"yani"_s, u"Late source variants are missing"_s);

    controller->setSelectedFavorite(true);
    waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Favorite write"_s);
    controller->setListMode(Qbutt::Search::ListMode::Favorites);
    controller->selectWork(work);
    require(controller->property("selectedVariantState").value<Qbutt::Search::VariantSelectionState>()
        == Qbutt::Search::VariantSelectionState::Inspecting,
        u"Unverified cached variant became actionable"_s);
    controller->resolveVariant(selected);
    require(handoffs == 1 && !controller->property("resolving").toBool(),
        u"Cached variant started an action before verification"_s);
    qInfo("FIXTURE release_cached_fast");
    const auto refreshedState = missing ? Qbutt::Search::VariantSelectionState::Unavailable
                                       : Qbutt::Search::VariantSelectionState::Available;
    waitFor([&]
    {
        return controller->property("selectedVariantState").value<Qbutt::Search::VariantSelectionState>()
            == refreshedState;
    }, u"Exact saved variant refreshed"_s);
    require(controller->property("inspecting").toBool(), u"Slow cached source was not held"_s);
    require(controller->property("selectedVariantId").toString() == selected,
        u"Saved choice was silently substituted"_s);
    controller->resolveVariant(selected);
    if (missing)
    {
        require(handoffs == 1 && !controller->property("resolving").toBool(),
            u"Disappeared cached variant started an action"_s);
    }
    else
    {
        waitFor([&] { return handoffs == 2; }, u"Verified saved variant during slow inspection"_s);
        require(magnet.endsWith(u"9662"_s), u"Saved action resolved another variant"_s);
    }
    qInfo("FIXTURE release_cached_late");
    waitFor([&]
    {
        return !controller->property("inspecting").toBool() && !controller->property("libraryPending").toBool();
    }, u"Saved publication completion"_s);
    qInfo("PASS progressive_selection late=inspected ready=actionable cached=verified handoff=exact selection=stable");
}
