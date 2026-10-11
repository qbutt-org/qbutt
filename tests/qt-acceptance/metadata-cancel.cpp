// SPDX-License-Identifier: GPL-2.0-or-later
#include "metadata-cancel.h"

#include <functional>
#include <optional>
#include <stdexcept>

#include <QCoreApplication>
#include <QElapsedTimer>
#include <QEventLoop>
#include <QJsonArray>
#include <QJsonObject>
#include <QThread>

#include "base/bittorrent/addtorrenterror.h"
#include "base/bittorrent/addtorrentparams.h"
#include "base/bittorrent/sessionimpl.h"
#include "base/bittorrent/torrentdescriptor.h"
#include "base/bittorrent/torrentimpl.h"
#include "base/global.h"

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
