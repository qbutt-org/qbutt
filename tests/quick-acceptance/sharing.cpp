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
#include <QSqlDatabase>
#include <QSqlError>
#include <QSqlQuery>
#include <QString>
#include <QTemporaryDir>
#include <QThreadPool>
#include <QTimer>

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

    QString link(const QJsonArray &publications, const QJsonObject &preferred)
    {
        const QJsonObject document {
            {u"version"_s, 1},
            {u"publications"_s, publications},
            {u"preferred"_s, preferred},
        };
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

    QJsonObject savedGroup(const QString &profile)
    {
        QJsonObject group;
        {
            QSqlDatabase database = QSqlDatabase::addDatabase(u"QSQLITE"_s, u"sharing-fixture"_s);
            database.setDatabaseName(QDir(profile).filePath(u"content-library.sqlite"_s));
            require(database.open(), database.lastError().text());
            QSqlQuery query(database);
            require(query.exec(u"SELECT entry FROM library"_s), query.lastError().text());
            require(query.next(), u"Shared work was not saved"_s);
            group = QJsonDocument::fromJson(query.value(0).toString().toUtf8()).object().value(u"group"_s).toObject();
            database.close();
        }
        QSqlDatabase::removeDatabase(u"sharing-fixture"_s);
        return group;
    }

    void readyLibrary(Qbutt::ContentController &controller, const QString &profile)
    {
        controller.initLibrary(profile);
        waitFor([&]
        {
            return controller.property("libraryState").value<Qbutt::Library::LibraryState>()
                != Qbutt::Library::LibraryState::Loading;
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
    const QJsonArray confirmed = savedGroup(profile.path()).value(u"references"_s).toArray();
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
    controller.reset();
    require(QThreadPool::globalInstance()->waitForDone(100'000), u"Shared receiver owner did not drain"_s);
    qInfo("PASS share exact=unavailable unrelated=isolated missing=unselected malformed=rejected");
}
