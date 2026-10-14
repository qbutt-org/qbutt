// SPDX-License-Identifier: GPL-2.0-or-later
#include "progressive-selection.h"

#include <functional>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QApplication>
#include <QDebug>
#include <QElapsedTimer>
#include <QEventLoop>
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
    QTemporaryDir profile;
    require(profile.isValid(), u"Progressive profile failed"_s);
    const auto drainOwners = qScopeGuard([]
    {
        if (!QThreadPool::globalInstance()->waitForDone(10'000))
            qFatal("Progressive library owner did not drain");
    });
    Content::ContentView view(profile.path());
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
    const QString work = value(works, 0, "workId");
    controller->selectWork(work);
    waitFor([&]
    {
        return !controller->property("inspecting").toBool() && (variants->rowCount() == 2);
    }, u"Fast publication variants"_s);
    const QString selected = value(variants, 1, "variantId");
    controller->selectVariant(selected);
    qInfo("FIXTURE release_late_search");

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
    qInfo("PASS progressive_selection late=inspected work=stable variant=stable row=stable");
}
