// SPDX-License-Identifier: GPL-2.0-or-later
#include "downloads.h"

#include <functional>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QApplication>
#include <QDebug>
#include <QDir>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFileInfo>
#include <QQuickItem>
#include <QQuickWidget>
#include <QSet>
#include <QTimer>
#include <QVariant>

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
        while (!condition() && (elapsed.elapsed() < 10'000))
        {
            QEventLoop loop;
            QTimer::singleShot(20, &loop, &QEventLoop::quit);
            loop.exec();
        }
        require(condition(), operation + u" timed out"_s);
    }

    int role(const QAbstractItemModel *model, const QByteArray &name)
    {
        const auto roles = model->roleNames();
        for (auto iterator = roles.cbegin(); iterator != roles.cend(); ++iterator)
        {
            if (iterator.value() == name)
                return iterator.key();
        }
        throw std::runtime_error("Missing download role: " + name.toStdString());
    }

    QVariant value(QAbstractItemModel *model, const int row, const QByteArray &name)
    {
        return model->data(model->index(row, 0), role(model, name));
    }
}

void exerciseDownloads(const QString &profile)
{
    require(QDir::isAbsolutePath(profile), u"Pass an isolated absolute fixture profile"_s);
    require(QFileInfo::exists(QDir(profile).filePath(u"content-library.sqlite"_s)),
        u"Run the generated Rust download fixture first"_s);

    Qbutt::ContentController controller;
    controller.initLibrary(profile);
    waitFor([&]
    {
        const auto state = controller.property("libraryState").value<Qbutt::Library::LibraryState>();
        return state == Qbutt::Library::LibraryState::Ready
            || state == Qbutt::Library::LibraryState::Failed;
    }, u"Library open"_s);
    require(controller.property("libraryState").value<Qbutt::Library::LibraryState>()
        == Qbutt::Library::LibraryState::Ready, u"Fixture library failed to open"_s);

    const QString destination = QDir::cleanPath(QDir(profile).filePath(u"../downloads"_s));
    controller.initDownloads(destination);
    auto *model = qobject_cast<QAbstractItemModel *>(controller.property("downloads").value<QObject *>());
    require(model && model->thread() == qApp->thread(), u"Download model has the wrong Qt owner"_s);
    waitFor([&] { return model->rowCount() == 4; }, u"Persisted download model"_s);

    QSet<QString> statuses;
    QSet<QString> ids;
    int completed = 0;
    int errors = 0;
    for (int row = 0; row < model->rowCount(); ++row)
    {
        const QString id = value(model, row, "jobId").toString();
        const QString status = value(model, row, "status").toString();
        require(id.size() == 32 && !ids.contains(id), u"Invalid persisted download identity"_s);
        ids.insert(id);
        statuses.insert(status);
        require(!value(model, row, "title").toString().isEmpty(), u"Missing download title"_s);
        if (status == u"complete"_s)
        {
            const QString path = value(model, row, "path").toString();
            const qint64 size = QFileInfo(path).size();
            require(size > 0 && size == value(model, row, "bytes").toLongLong(),
                u"Published payload does not match model progress"_s);
            ++completed;
        }
        else if (status == u"error"_s)
        {
            require(!value(model, row, "errorText").toString().isEmpty(),
                u"Failed download has no diagnostic"_s);
            ++errors;
        }
    }
    require(completed == 3 && errors == 1 && statuses.size() == 2,
        u"Persisted download states differ from generated fixture"_s);

    QQuickWidget page;
    page.setInitialProperties({{u"backend"_s, QVariant::fromValue(&controller)}});
    page.loadFromModule(u"Qbutt.Views"_s, u"DownloadsPage"_s);
    waitFor([&] { return page.status() != QQuickWidget::Loading; }, u"Downloads page"_s);
    require(page.status() == QQuickWidget::Ready && page.rootObject(),
        u"Downloads page did not load"_s);
    auto *list = page.rootObject()->findChild<QQuickItem *>(u"contentDownloads"_s);
    require(list && list->property("count").toInt() == 4,
        u"Downloads page did not bind the persisted jobs"_s);
    qInfo().noquote() << "PASS native Qt downloads model and page jobs=4 complete=3 error=1";
}
