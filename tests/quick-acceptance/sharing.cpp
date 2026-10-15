// SPDX-License-Identifier: GPL-2.0-or-later
#include "sharing.h"

#include <functional>
#include <memory>
#include <stdexcept>

#include <QAbstractItemModel>
#include <QByteArray>
#include <QDebug>
#include <QDir>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QScopeGuard>
#include <QSqlDatabase>
#include <QSqlError>
#include <QSqlQuery>
#include <QString>
#include <QTemporaryDir>
#include <QThreadPool>
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

    void waitFor(const std::function<bool()> &ready, const QString &operation)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!ready() && (elapsed.elapsed() < 50'000))
        {
            QEventLoop loop;
            QTimer::singleShot(20, &loop, &QEventLoop::quit);
            loop.exec();
        }
        require(ready(), operation + u" timed out"_s);
    }

    QJsonObject publication(const QString &id)
    {
        return {{u"source"_s, u"ani_liberty"_s}, {u"id"_s, id}};
    }

    QString link(const QJsonArray &publications, const QJsonObject &preferred = {})
    {
        QJsonObject document {
            {u"version"_s, 1},
            {u"publications"_s, publications},
        };
        if (!preferred.isEmpty())
            document.insert(u"preferred"_s, preferred);
        const auto bytes = QJsonDocument(document).toJson(QJsonDocument::Compact);
        const auto encoded = bytes.toBase64(QByteArray::Base64UrlEncoding | QByteArray::OmitTrailingEquals);
        return u"qbutt://open?d="_s + QString::fromLatin1(encoded);
    }

    QJsonObject preferredTorrent(const QString &publicationId, const int torrentId)
    {
        const QJsonObject selector {{u"torrent"_s, QJsonObject {{u"id"_s, torrentId}}}};
        return {
            {u"publication"_s, publication(publicationId)},
            {u"selector"_s, selector},
        };
    }

    QJsonObject storedJson(const QString &profile, const QString &sql)
    {
        const QString connection = u"sharing-fixture"_s;
        const auto removeConnection = qScopeGuard([&] { QSqlDatabase::removeDatabase(connection); });
        auto database = QSqlDatabase::addDatabase(u"QSQLITE"_s, connection);
        database.setDatabaseName(QDir(profile).filePath(u"content-library.sqlite"_s));
        require(database.open(), database.lastError().text());
        QSqlQuery query(database);
        require(query.exec(sql), query.lastError().text());
        require(query.next(), u"Shared fixture row is missing"_s);
        return QJsonDocument::fromJson(query.value(0).toString().toUtf8()).object();
    }

    QVariant sourceValue(Qbutt::ContentController &controller, const QByteArray &role)
    {
        auto *sources = qobject_cast<QAbstractItemModel *>(controller.property("sources").value<QObject *>());
        require(sources, u"Native source settings are missing"_s);
        const int sourceRole = sources->roleNames().key("sourceId", -1);
        const int valueRole = sources->roleNames().key(role, -1);
        require(sourceRole >= 0 && valueRole >= 0, u"Source status role is missing"_s);
        for (int row = 0; row < sources->rowCount(); ++row)
        {
            if (sources->data(sources->index(row, 0), sourceRole).toString() == u"aniliberty"_s)
                return sources->data(sources->index(row, 0), valueRole);
        }
        throw std::runtime_error("The shared fixture source is missing");
    }

    void readyLibrary(Qbutt::ContentController &controller, const QString &profile)
    {
        controller.initLibrary(profile);
        waitFor([&]
        {
            const auto state = controller.property("libraryState").value<Qbutt::Library::LibraryState>();
            return (state == Qbutt::Library::LibraryState::Ready)
                || (state == Qbutt::Library::LibraryState::Failed);
        }, u"Isolated library"_s);
        require(controller.property("libraryState").value<Qbutt::Library::LibraryState>()
            == Qbutt::Library::LibraryState::Ready, controller.property("libraryErrorText").toString());

        auto *sources = qobject_cast<QAbstractItemModel *>(controller.property("sources").value<QObject *>());
        require(sources, u"Native source settings are missing"_s);
        const int idRole = sources->roleNames().key("sourceId", -1);
        require(idRole >= 0, u"Source identity role is missing"_s);
        for (int row = 0; row < sources->rowCount(); ++row)
        {
            const QString id = sources->data(sources->index(row, 0), idRole).toString();
            if (id != u"aniliberty"_s)
                require(controller.setSourceEnabled(id, false), u"Could not isolate the fixture source"_s);
        }
        waitFor([&] { return !controller.property("libraryPending").toBool(); }, u"Source settings"_s);
        require(controller.property("sourceSettingsState").value<Qbutt::Sources::SettingsState>()
            == Qbutt::Sources::SettingsState::Ready, u"Fixture source settings failed"_s);
    }
}

void exerciseSharing()
{
    QTemporaryDir profile;
    require(profile.isValid(), u"Temporary receiver profile failed"_s);
    auto controller = std::make_unique<Qbutt::ContentController>();
    readyLibrary(*controller, profile.path());

    int unavailableSignals = 0;
    int malformedSignals = 0;
    QObject::connect(controller.get(), &Qbutt::ContentController::sharedLinkFailed,
        controller.get(), [&](const bool malformed)
    {
        if (malformed)
            ++malformedSignals;
        else
            ++unavailableSignals;
    });

    controller->openSharedLink(link({publication(u"9542"_s)}, preferredTorrent(u"9542"_s, 999'999)));
    waitFor([&] { return !controller->property("sharedImporting").toBool(); }, u"Unavailable exact variant"_s);
    require((controller->property("selectedWorkId").toString() == u"aniliberty:9542"_s)
        && (controller->property("selectedVariantState").value<Qbutt::Search::VariantSelectionState>()
            == Qbutt::Search::VariantSelectionState::Unavailable)
        && (unavailableSignals == 0), u"Unavailable exact variant was substituted"_s);

    controller->openSharedLink(link({publication(u"9542"_s), publication(u"9543"_s)},
        preferredTorrent(u"9542"_s, 28'083)));
    waitFor([&] { return !controller->property("sharedImporting").toBool(); }, u"Unrelated publications"_s);
    require((controller->property("selectedWorkId").toString() == u"aniliberty:9542"_s)
        && (unavailableSignals == 1), u"Unrelated publication changed the imported work"_s);
    controller->setSelectedFavorite(true);
    waitFor([&] { return !controller->property("libraryPending").toBool(); }, u"Imported favorite"_s);
    const QJsonArray confirmed = storedJson(profile.path(), u"SELECT entry FROM library"_s)
        .value(u"group"_s).toObject().value(u"references"_s).toArray();
    require((confirmed.size() == 1) && (confirmed[0].toObject() == publication(u"9542"_s)),
        u"Unverified publication escaped into the library"_s);

    controller->openSharedLink(link({publication(u"9544"_s), publication(u"9542"_s)},
        preferredTorrent(u"9544"_s, 48'083)));
    waitFor([&] { return !controller->property("sharedImporting").toBool(); }, u"Missing preferred publication"_s);
    require((controller->property("selectedWorkId").toString() == u"aniliberty:9542"_s)
        && controller->property("selectedVariantId").toString().isEmpty()
        && (unavailableSignals == 2), u"Missing preferred publication selected another version"_s);

    controller->openSharedLink(u"qbutt://open?d=invalid!"_s);
    require(malformedSignals == 1, u"Malformed shared link was accepted"_s);

    const QJsonArray disabledPublications {
        QJsonObject {{u"source"_s, u"onlinefix"_s}, {u"id"_s, u"games/action/42-sample-game.html"_s}},
        QJsonObject {{u"source"_s, u"freetp"_s}, {u"id"_s, u"42"_s}},
        QJsonObject {{u"source"_s, u"byrut"_s}, {u"id"_s, u"42-game"_s}},
    };
    for (const QJsonValue &publicationRef : disabledPublications)
    {
        const int previousUnavailable = unavailableSignals;
        controller->openSharedLink(link({publicationRef.toObject()}));
        require((malformedSignals == 1) && (unavailableSignals == previousUnavailable + 1),
            u"Valid public source reference was rejected as malformed"_s);
    }

    const QJsonObject privateBook {
        {u"source"_s, u"local_book"_s},
        {u"id"_s, QString(64, QChar(u'a'))},
    };
    controller->openSharedLink(link({privateBook}));
    require((malformedSignals == 2) && (unavailableSignals == 5),
        u"Private local book was accepted as a public reference"_s);

    controller->openSharedLink(link({publication(u"9550"_s), publication(u"9551"_s)},
        preferredTorrent(u"9550"_s, 38'083)));
    auto *variants = qobject_cast<QAbstractItemModel *>(controller->property("variants").value<QObject *>());
    require(variants, u"Shared variants model is missing"_s);
    waitFor([&] { return variants->rowCount() == 2; }, u"First shared publication"_s);
    require(controller->property("sharedImporting").toBool(), u"Delayed publication arrived before the action"_s);

    const int idRole = variants->roleNames().key("variantId", -1);
    require(idRole >= 0, u"Variant identity role is missing"_s);
    const QString preferred = controller->property("selectedVariantId").toString();
    QString selected;
    for (int row = 0; row < variants->rowCount(); ++row)
    {
        const QString id = variants->data(variants->index(row, 0), idRole).toString();
        if (id != preferred)
            selected = id;
    }
    require(!selected.isEmpty(), u"Alternative shared variant is missing"_s);
    QString receivedMagnet;
    QObject::connect(controller.get(), &Qbutt::ContentController::torrentReady,
        controller.get(), [&](const QString &magnet) { receivedMagnet = magnet; });
    controller->resolveVariant(selected);
    require(controller->property("resolving").toBool(), u"Shared action did not start"_s);
    waitFor([&] { return !controller->property("sharedImporting").toBool(); }, u"Additional shared publication"_s);
    require(controller->property("resolving").toBool() || !receivedMagnet.isEmpty(),
        u"Late publication cancelled the user's running action"_s);
    require((controller->property("selectedVariantId").toString() == selected) && (variants->rowCount() == 4),
        u"Late publication replaced the user's choice or lost alternatives"_s);
    waitFor([&] { return !receivedMagnet.isEmpty(); }, u"Exact shared torrent"_s);
    require(receivedMagnet.endsWith(QString::number(38'084, 16).rightJustified(40, QChar(u'0'))),
        u"Shared action returned a different torrent"_s);

    const int previousUnavailable = unavailableSignals;
    controller->openSharedLink(link({publication(u"9542"_s), publication(u"9545"_s)},
        preferredTorrent(u"9542"_s, 28'084)));
    waitFor([&]
    {
        return !controller->property("sharedImporting").toBool() && !controller->property("libraryPending").toBool();
    }, u"Failed alternate shared publication"_s);
    require(unavailableSignals == previousUnavailable && variants->rowCount() == 3
        && controller->property("selectedVariantId").toString() == u"aniliberty:9542:torrent:28084"_s
        && controller->property("selectedVariantState").value<Qbutt::Search::VariantSelectionState>()
            == Qbutt::Search::VariantSelectionState::Available,
        u"Failed alternate blocked or replaced the available exact shared variant"_s);
    require(sourceValue(*controller, "failed").toBool()
        && !sourceValue(*controller, "failureText").toString().isEmpty(),
        u"Failed alternate was not reported in Sources"_s);
    const QJsonObject failure = storedJson(profile.path(),
        u"SELECT failure FROM source_settings WHERE source = 'aniliberty'"_s);
    require(failure.value(u"operation"_s).toObject().value(u"inspect"_s).toObject().value(u"reference"_s).toObject()
        == publication(u"9545"_s), u"Shared source failure lost its exact failed publication"_s);

    const QString userChoice = u"aniliberty:9542:torrent:28085"_s;
    controller->selectVariant(userChoice);
    require(controller->retrySource(u"aniliberty"_s), u"Failed shared alternate retry was rejected"_s);
    waitFor([&]
    {
        return !sourceValue(*controller, "retrying").toBool() && !controller->property("libraryPending").toBool();
    }, u"Shared alternate retry"_s);
    require(!sourceValue(*controller, "failed").toBool() && variants->rowCount() == 5
        && controller->property("selectedVariantId").toString() == userChoice
        && unavailableSignals == previousUnavailable,
        u"Shared retry lost the user's choice, failed association or left its source error"_s);

    controller->openSharedLink(link({publication(u"9542"_s), publication(u"9546"_s)},
        preferredTorrent(u"9542"_s, 28'084)));
    waitFor([&]
    {
        return !controller->property("sharedImporting").toBool() && !controller->property("libraryPending").toBool();
    }, u"Cancellable shared alternate"_s);
    const QString beforeCancel = controller->property("selectedVariantId").toString();
    require(controller->retrySource(u"aniliberty"_s), u"Cancellable shared retry was rejected"_s);
    controller->setListMode(Qbutt::Search::ListMode::Favorites);
    qInfo("FIXTURE release_cancelled_shared_retry");
    waitFor([&]
    {
        return !sourceValue(*controller, "retrying").toBool() && !controller->property("libraryPending").toBool();
    }, u"Cancelled shared retry outcome"_s);
    require(variants->rowCount() == 3 && controller->property("selectedVariantId").toString() == beforeCancel
        && unavailableSignals == previousUnavailable,
        u"Cancelled shared retry changed selection, added a publication or opened an error"_s);

    controller->openSharedLink(link({publication(u"9547"_s)}));
    waitFor([&] { return !controller->property("sharedImporting").toBool(); }, u"Entirely unavailable shared work"_s);
    require(unavailableSignals == previousUnavailable + 1 && variants->rowCount() == 0
        && controller->property("selectedWorkId").toString().isEmpty(), u"Full shared failure was hidden"_s);

    controller.reset();
    require(QThreadPool::globalInstance()->waitForDone(100'000), u"Shared receiver owner did not drain"_s);
    qInfo("PASS share exact=unavailable unrelated=isolated missing=unselected public=accepted private=rejected malformed=rejected late=preserved partial=usable source_failure=exact retry=joined cancelled_retry=ignored full_failure=visible");
}
