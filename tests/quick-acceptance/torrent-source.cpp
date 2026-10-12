// SPDX-License-Identifier: GPL-2.0-or-later
#include "torrent-source.h"

#include <functional>
#include <memory>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QApplication>
#include <QDebug>
#include <QDir>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QQmlComponent>
#include <QQmlEngine>
#include <QRegularExpression>
#include <QScopeGuard>
#include <QSqlDatabase>
#include <QSqlError>
#include <QSqlQuery>
#include <QTemporaryDir>
#include <QThread>
#include <QThreadPool>
#include <QTimer>
#include <QUrl>
#include <QUrlQuery>

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

    void waitFor(const std::function<bool()> &ready, const int timeout, const QString &operation)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!ready() && (elapsed.elapsed() < timeout))
        {
            QEventLoop loop;
            QTimer::singleShot(20, &loop, &QEventLoop::quit);
            loop.exec();
        }
        require(ready(), operation + u" timed out"_s);
    }

    QAbstractItemModel *model(Qbutt::ContentController &controller, const char *name)
    {
        auto *result = qobject_cast<QAbstractItemModel *>(controller.property(name).value<QObject *>());
        require(result && (result->thread() == qApp->thread()), u"Invalid typed model"_s);
        return result;
    }

    QString value(QAbstractItemModel *model, const int row, const QByteArray &name)
    {
        const auto roles = model->roleNames();
        for (auto iterator = roles.cbegin(); iterator != roles.cend(); ++iterator)
        {
            if (iterator.value() == name)
                return model->data(model->index(row, 0), iterator.key()).toString();
        }
        throw std::runtime_error("Missing role: " + name.toStdString());
    }

    void readyLibrary(Qbutt::ContentController &controller, const QString &profile)
    {
        controller.initLibrary(profile);
        waitFor([&]
        {
            const auto state = controller.property("libraryState").value<Qbutt::Library::LibraryState>();
            return (state == Qbutt::Library::LibraryState::Ready) || (state == Qbutt::Library::LibraryState::Failed);
        }, 10'000, u"Library open"_s);
        require(controller.property("libraryState").value<Qbutt::Library::LibraryState>()
            == Qbutt::Library::LibraryState::Ready, controller.property("libraryErrorText").toString());
    }

    void saved(Qbutt::ContentController &controller)
    {
        waitFor([&] { return !controller.property("libraryPending").toBool(); }, 10'000, u"Library write"_s);
        require(controller.property("libraryErrorText").toString().isEmpty(), u"Torrent library write failed"_s);
    }

    void inspect(Qbutt::ContentController &controller, const QString &publication, const QString &category)
    {
        controller.selectWork(publication);
        require(controller.property("inspecting").toBool(), u"Torrent selection did not request fresh metadata"_s);
        waitFor([&] { return !controller.property("inspecting").toBool(); }, 50'000, u"Torrent inspection"_s);
        require(controller.property("errorText").toString().isEmpty(), controller.property("errorText").toString());
        require(controller.property("selectedWorkCategory").toString() == category,
            u"Fresh torrent metadata lost its source category"_s);
    }
}

void exerciseTorrentSource(const TorrentCatalog catalog)
{
    const QString sourceId = (catalog == TorrentCatalog::Nyaa) ? u"nyaa"_s : u"rutor"_s;
    const QString sourceName = (catalog == TorrentCatalog::Nyaa) ? u"Nyaa.si"_s : u"RuTor"_s;
    const QString queryText = (catalog == TorrentCatalog::Nyaa) ? u"Frieren"_s : u"Blender"_s;
    const QString category = (catalog == TorrentCatalog::Nyaa) ? u"anime"_s : u"software"_s;

    QTemporaryDir profile;
    require(profile.isValid(), u"Temporary profile failed"_s);
    const auto drain = qScopeGuard([]
    {
        if (!QThreadPool::globalInstance()->waitForDone(100'000))
            qFatal("Torrent source fixture owners did not drain");
    });
    auto controller = std::make_unique<Qbutt::ContentController>();
    readyLibrary(*controller, profile.path());
    auto *works = model(*controller, "works");
    QObject observation;
    bool publishedWhileSearching = false;
    bool wrongThread = false;
    QObject::connect(works, &QAbstractItemModel::rowsInserted, &observation,
        [&](const QModelIndex &, const int first, const int last)
    {
        wrongThread |= QThread::currentThread() != qApp->thread();
        for (int row = first; row <= last; ++row)
        {
            if (value(works, row, "source") == sourceName)
            {
                publishedWhileSearching |= controller->property("busy").toBool();
            }
        }
    }, Qt::DirectConnection);
    controller->search(queryText);
    waitFor([&] { return !controller->property("busy").toBool(); }, 150'000, u"Broad native search"_s);
    require(controller->property("errorText").toString().isEmpty(), controller->property("errorText").toString());
    require(publishedWhileSearching && !wrongThread,
        u"Torrent catalog did not publish incremental results on the Qt thread"_s);

    QString publication;
    QString title;
    for (int row = 0; row < works->rowCount(); ++row)
    {
        if ((value(works, row, "source") == sourceName) && (value(works, row, "category") == category))
        {
            publication = value(works, row, "workId");
            title = value(works, row, "title");
            break;
        }
    }
    require(QRegularExpression(u"^%1:[1-9][0-9]*$"_s.arg(sourceId)).match(publication).hasMatch()
        && title.contains(queryText, Qt::CaseInsensitive), u"Torrent search returned no exact publication"_s);
    inspect(*controller, publication, category);
    auto *variants = model(*controller, "variants");
    require(variants->rowCount() == 1, u"Publication did not retain its single exact torrent"_s);
    const QString variant = value(variants, 0, "variantId");
    const QString hash = value(variants, 0, "infoHash");
    require(QRegularExpression(u"^[0-9a-f]{40}$"_s).match(hash).hasMatch()
        && (variant == publication + u":torrent:"_s + hash)
        && (value(variants, 0, "source") == sourceName)
        && (value(variants, 0, "kind") == u"torrent"_s)
        && value(variants, 0, "label").contains(queryText, Qt::CaseInsensitive),
        u"Exact hash, raw release title or transport was lost at the Qt boundary"_s);
    const auto *typedVariants = qobject_cast<Qbutt::VariantModel *>(variants);
    require(typedVariants && (typedVariants->idAt(typedVariants->indexOfId(variant)) == variant),
        u"Exact selector cannot be looked up through the typed model"_s);

    controller->selectVariant(variant);
    controller->setSelectedFavorite(true);
    saved(*controller);
    require(controller->property("selectedFavorite").toBool()
        && (model(*controller, "favorites")->rowCount() == 1)
        && (model(*controller, "history")->rowCount() == 0), u"Torrent favorite or exact preference was not persisted"_s);
    QObject::disconnect(works, nullptr, &observation, nullptr);
    controller.reset();

    // Stored metadata remains valid but cannot own a publication's fresh category.
    {
        const auto unregister = qScopeGuard([] { QSqlDatabase::removeDatabase(u"torrent-stale-category"_s); });
        QSqlDatabase database = QSqlDatabase::addDatabase(u"QSQLITE"_s, u"torrent-stale-category"_s);
        database.setDatabaseName(QDir(profile.path()).filePath(u"content-library.sqlite"_s));
        require(database.open(), database.lastError().text());
        QSqlQuery query(database);
        require(query.exec(u"UPDATE library SET entry = json_set(entry, '$.work.category', 'books')"_s),
            query.lastError().text());
        require(query.numRowsAffected() == 1, u"Generated stale category fixture did not update its entry"_s);
    }

    controller = std::make_unique<Qbutt::ContentController>();
    readyLibrary(*controller, profile.path());
    require((model(*controller, "works")->rowCount() == 0)
        && (model(*controller, "favorites")->rowCount() == 1), u"Reopened favorite requires a repeat search"_s);
    require(value(model(*controller, "favorites"), 0, "category") == u"books"_s,
        u"The generated cached category was not loaded"_s);
    QQmlEngine engine;
    QQmlComponent categoryProbe(&engine);
    categoryProbe.setData("import QtQml; import Qbutt.Content; QtObject { required property ContentController backend; "
        "readonly property string observedCategory: backend.selectedWorkCategory }", QUrl());
    std::unique_ptr<QObject> categoryBinding(categoryProbe.createWithInitialProperties({
        {u"backend"_s, QVariant::fromValue(controller.get())}}));
    require(categoryBinding != nullptr, u"Typed QML category binding did not load"_s);
    controller->setListMode(Qbutt::Search::ListMode::Favorites);
    bool cancelledFromCategory = false;
    const auto categoryConnection = QObject::connect(controller.get(), &Qbutt::ContentController::selectedWorkCategoryChanged,
        &observation, [&]
    {
        if (!cancelledFromCategory && (controller->property("selectedWorkCategory").toString() == u"books"_s))
        {
            cancelledFromCategory = true;
            controller->search(QString());
        }
    }, Qt::DirectConnection);
    controller->selectWork(publication);
    QObject::disconnect(categoryConnection);
    require(cancelledFromCategory && controller->property("selectedWorkId").toString().isEmpty()
        && !controller->property("inspecting").toBool() && (model(*controller, "variants")->rowCount() == 0),
        u"Reentrant category notification published stale inspection or variants"_s);
    inspect(*controller, publication, category);
    require(categoryBinding->property("observedCategory").toString() == category,
        u"Fresh category of the same publication did not update its QML binding"_s);
    require(controller->property("selectedVariantId").toString() == variant
        && (controller->property("selectedVariantState").value<Qbutt::Search::VariantSelectionState>()
            == Qbutt::Search::VariantSelectionState::Available), u"Reinspection replaced the preferred exact selector"_s);

    int handoffs = 0;
    bool exactHandoff = false;
    QObject::connect(controller.get(), &Qbutt::ContentController::torrentReady, &observation, [&](const QString &magnet)
    {
        ++handoffs;
        exactHandoff = QUrlQuery(QUrl(magnet)).queryItemValue(u"xt"_s).compare(u"urn:btih:"_s + hash,
            Qt::CaseInsensitive) == 0;
        wrongThread |= QThread::currentThread() != qApp->thread();
    });
    controller->resolveVariant(variant);
    require(controller->property("resolving").toBool(), u"Exact resolution did not start"_s);
    controller->search(QString());
    inspect(*controller, publication, category);
    require((handoffs == 0) && !controller->property("resolving").toBool(),
        u"Cancelled resolution emitted a torrent handoff"_s);

    controller->resolveVariant(variant);
    waitFor([&] { return !controller->property("resolving").toBool(); }, 50'000, u"Fresh exact torrent resolution"_s);
    saved(*controller);
    require(controller->property("errorText").toString().isEmpty()
        && exactHandoff && (handoffs == 1) && !wrongThread
        && (model(*controller, "history")->rowCount() == 0), u"Exact resolution failed or fabricated history"_s);
    require(controller->setSourceEnabled(sourceId, false), u"Source disable was rejected"_s);
    saved(*controller);
    controller->resolveVariant(variant);
    require((handoffs == 1) && !controller->property("resolving").toBool(),
        u"Disabled source remained actionable through its cached exact selector"_s);
    qInfo().noquote() << "PASS native_torrent_source=" + sourceId
        << "incremental=ok categories=confirmed category_binding=fresh category_reentrant=ok Qt_hash_selector=exact favorite_reopen=ok"
        << "cancel=no_handoff disabled=blocked torrentReady=exact history=empty publication=" + publication;
    // This host validates the descriptor handoff only. It has no libtorrent session.
}
