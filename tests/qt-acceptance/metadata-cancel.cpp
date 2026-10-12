// SPDX-License-Identifier: GPL-2.0-or-later
#include "metadata-cancel.h"

#include <functional>
#include <optional>
#include <stdexcept>

#include <QApplication>
#include <QDir>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QFile>
#include <QJsonArray>
#include <QJsonObject>
#include <QPointer>
#include <QThread>
#include <QThreadPool>

#include "app/application.h"
#include "base/bittorrent/addtorrenterror.h"
#include "base/bittorrent/addtorrentparams.h"
#include "base/bittorrent/sessionimpl.h"
#include "base/bittorrent/torrentdescriptor.h"
#include "base/bittorrent/torrentimpl.h"
#include "base/global.h"
#include "base/preferences.h"
#include "gui/addnewtorrentdialog.h"
#include "gui/guiaddtorrentmanager.h"
#include "gui/mainwindow.h"
#include "gui/quick/contentview.h"
#include "qbutt_quick/src/controller.cxxqt.h"

namespace
{
    void require(const bool condition, const QString &message)
    {
        if (!condition)
            throw std::runtime_error(message.toStdString());
    }

    void waitFor(const QString &label, const std::function<bool()> &ready)
    {
        QElapsedTimer elapsed;
        elapsed.start();
        while (!ready() && (elapsed.elapsed() < 10000))
        {
            QCoreApplication::processEvents(QEventLoop::AllEvents, 20);
            QThread::msleep(1);
        }
        require(ready(), u"Timed out: %1"_s.arg(label));
    }
}

void exerciseMetadataCancel(const QJsonObject &spec, QJsonObject &evidence)
{
    auto *session = static_cast<BitTorrent::SessionImpl *>(BitTorrent::Session::instance());
    require(session->torrentsCount() == 0 && !session->hasPendingStorageJobs(),
        u"Metadata cancellation requires an empty isolated session"_s);
    require(!session->isDHTEnabled() && !session->isLSDEnabled() && !session->isPeXEnabled()
            && !session->isAddTrackersEnabled() && !session->isAddTrackersFromURLEnabled()
            && session->networkInterfaceAddress() == u"127.0.0.1",
        u"Metadata cancellation requires isolated loopback discovery settings"_s);

    const auto descriptor = BitTorrent::TorrentDescriptor::loadFromFile(Path(spec.value(u"torrentPath"_s).toString()));
    require(descriptor && descriptor->info() && descriptor->info()->isPrivate(),
        u"Cannot load the generated private metadata fixture"_s);
    const BitTorrent::InfoHash infoHash = descriptor->infoHash();
    const BitTorrent::TorrentID id = infoHash.toTorrentID();
    const auto magnet = BitTorrent::TorrentDescriptor::parse(id.toString());
    require(magnet && !magnet->info() && magnet->infoHash() == infoHash,
        u"Generated magnet does not match the fixture"_s);

    qsizetype addAlerts = 0;
    int addedTorrents = 0;
    int metadataNotifications = 0;
    std::optional<BitTorrent::AddTorrentError> addFailure;
    QObject context;
    QObject::connect(session, &BitTorrent::SessionImpl::addTorrentAlertsReceived, &context,
        [&](const qsizetype count) { addAlerts += count; });
    QObject::connect(session, &BitTorrent::Session::metadataDownloaded, &context,
        [&] { ++metadataNotifications; });
    QObject::connect(session, &BitTorrent::Session::torrentAdded, &context,
        [&] { ++addedTorrents; });
    QObject::connect(session, &BitTorrent::Session::addTorrentFailed, &context,
        [&](const BitTorrent::InfoHash &failedHash, const BitTorrent::AddTorrentError &error)
    {
        if (failedHash == infoHash)
        {
            addFailure = error;
            evidence[u"addFailure"_s] = QJsonObject {{u"kind"_s, static_cast<int>(error.kind)},
                {u"message"_s, error.message}};
        }
    });

    // Stay in this GUI turn: the queued add alert cannot replace the empty handle.
    require(session->downloadMetadata(*magnet), u"Metadata request was rejected"_s);
    require(session->cancelDownloadMetadata(id), u"Pending metadata request was not cancelled"_s);
    require(addAlerts == 0,
        u"Cancellation did not precede the native add callback"_s);
    waitFor(u"Cancelled metadata add alert"_s, [&] { return addAlerts == 1; });

    // Wait for the first callback before re-adding. Simultaneous accept is a separate race.
    BitTorrent::AddTorrentParams params;
    params.savePath = Path(spec.value(u"destination"_s).toString());
    params.useAutoTMM = false;
    params.useDownloadPath = false;
    params.addStopped = true;
    require(session->addTorrent(*descriptor, params), u"Normal fixture add was rejected"_s);
    waitFor(u"Normal add after metadata cancellation"_s,
        [&] { return addedTorrents == 1 || addFailure.has_value(); });
    if (addFailure)
        throw std::runtime_error(u"Normal add failed after cancellation: %1"_s.arg(addFailure->message).toStdString());

    auto *torrent = static_cast<BitTorrent::TorrentImpl *>(session->findTorrent(infoHash));
    require(addAlerts == 2 && addedTorrents == 1 && torrent
            && torrent->savePath() == params.savePath && torrent->isStopped(),
        u"Normal add retained the hidden torrent's destination or running state"_s);
    const lt::torrent_handle nativeHandle = torrent->nativeHandle();
    require(session->removeTorrent(torrent->id(), BitTorrent::TorrentRemoveOption::KeepContent),
        u"Cannot remove the accepted fixture torrent"_s);
    waitFor(u"Fixture native handle removal"_s, [&] { return !nativeHandle.is_valid(); });
    require(session->torrentsCount() == 0 && metadataNotifications == 0,
        u"Cancelled metadata emitted a result or left a registered torrent"_s);

    QJsonArray checks = evidence.value(u"checks"_s).toArray();
    checks.append(QJsonObject {{u"name"_s, u"metadata-cancel"_s}, {u"infoHash"_s, id.toString()},
        {u"addAlerts"_s, addAlerts}, {u"addedTorrents"_s, addedTorrents},
        {u"metadataNotifications"_s, metadataNotifications},
        {u"stoppedAddSucceeded"_s, true}, {u"nativeHandleRemoved"_s, true}});
    evidence[u"checks"_s] = checks;
}

void exerciseTorrentData(Application &application, const QJsonObject &spec, QJsonObject &evidence)
{
    auto *session = BitTorrent::Session::instance();
    auto *manager = static_cast<IGUIApplication &>(application).addTorrentManager();
    const auto findDialog = []() -> AddNewTorrentDialog *
    {
        for (QWidget *widget : QApplication::topLevelWidgets())
        {
            if (auto *dialog = qobject_cast<AddNewTorrentDialog *>(widget))
                return dialog;
        }
        return nullptr;
    };
    require(session->torrents().isEmpty() && !session->isDHTEnabled() && !session->isLSDEnabled()
            && !session->isPeXEnabled() && session->networkInterfaceAddress() == u"127.0.0.1",
        u"Torrent data requires an empty isolated session"_s);

    QFile file(spec.value(u"torrentPath"_s).toString());
    require(file.open(QIODevice::ReadOnly), u"Cannot open generated torrent metadata"_s);
    const QByteArray data = file.readAll();
    file.close();
    const auto expected = BitTorrent::TorrentDescriptor::load(data);
    require(expected && expected->info() && expected->info()->isPrivate(), u"Invalid legal metadata fixture"_s);

    const QString identity = u"dodi:4242--legal-fixture:torrent:file-me:fixture-id"_s;
    int failures = 0;
    int cancellations = 0;
    QObject context;
    QObject::connect(manager, &AddTorrentManager::addTorrentFailed, &context,
        [&](const QString &source, const BitTorrent::AddTorrentError &)
    {
        require(source == identity, u"Torrent data error lost its public selection identity"_s);
        require(QThread::currentThread() == application.thread(), u"Native outcome left the GUI thread"_s);
        ++failures;
    });
    QObject::connect(manager, &GUIAddTorrentManager::addTorrentCancelled, &context,
        [&](const BitTorrent::InfoHash &hash)
    {
        require(hash == expected->infoHash(), u"Cancelled dialog substituted torrent metadata"_s);
        ++cancellations;
    });
    require(manager->addTorrentData(identity, QByteArrayLiteral("<html>not torrent metadata</html>")),
        u"Malformed metadata task was not accepted"_s);
    waitFor(u"Malformed torrent data outcome"_s, [&] { return failures == 1; });

    auto *pref = Preferences::instance();
    const qint64 sizeLimit = pref->getTorrentFileSizeLimit();
    pref->setTorrentFileSizeLimit(data.size() - 1);
    require(manager->addTorrentData(identity, data), u"Size-limited metadata task was not accepted"_s);
    pref->setTorrentFileSizeLimit(sizeLimit);
    waitFor(u"Size limit snapshot outcome"_s, [&] { return failures == 2; });

    const int depthLimit = pref->getBdecodeDepthLimit();
    pref->setBdecodeDepthLimit(1);
    require(manager->addTorrentData(identity, data), u"Depth-limited metadata task was not accepted"_s);
    pref->setBdecodeDepthLimit(depthLimit);
    waitFor(u"Decode limit snapshot outcome"_s, [&] { return failures == 3; });

    auto *retired = new GUIAddTorrentManager(&static_cast<IGUIApplication &>(application), session);
    require(retired->addTorrentData(identity, data), u"Retired manager did not accept metadata"_s);
    delete retired;
    QThreadPool::globalInstance()->waitForDone();
    require(!findDialog(), u"Retired parser opened a torrent dialog"_s);

    const auto views = application.mainWindow()->findChildren<Content::ContentView *>();
    require(views.size() == 1, u"MainWindow does not own exactly one content view"_s);
    auto *controller = views.first()->findChild<Qbutt::ContentController *>();
    require(controller, u"Native content controller is missing"_s);
    controller->torrentMetadataReady(identity, data);

    QPointer<AddNewTorrentDialog> dialog;
    waitFor(u"Native torrent data dialog"_s, [&]
    {
        dialog = findDialog();
        return dialog && dialog->isVisible();
    });
    bool exactMetadata = false;
    QObject::connect(dialog, &AddNewTorrentDialog::torrentRejected, &context,
        [&](const BitTorrent::TorrentDescriptor &descriptor)
    {
        exactMetadata = descriptor.infoHash() == expected->infoHash()
            && descriptor.info()->filePaths() == expected->info()->filePaths();
    });
    dialog->reject();
    waitFor(u"Cancelled native torrent data dialog"_s, [&] { return !dialog && cancellations == 1; });
    require(exactMetadata && session->torrents().isEmpty(), u"Torrent data cancellation changed metadata or added payload"_s);
    require(QDir(spec.value(u"destination"_s).toString()).entryList(QDir::AllEntries | QDir::NoDotAndDotDot).isEmpty(),
        u"Metadata handoff wrote payload files"_s);

    QJsonArray checks = evidence.value(u"checks"_s).toArray();
    checks.append(QJsonObject {{u"name"_s, u"torrent-data"_s}, {u"infoHash"_s, expected->infoHash().toString()},
        {u"failures"_s, failures}, {u"cancellations"_s, cancellations}, {u"exactMetadata"_s, exactMetadata},
        {u"decodeLimitsSnapshotted"_s, true}, {u"retiredParserSilent"_s, true}, {u"payloadEmpty"_s, true}});
    evidence[u"checks"_s] = checks;
}
