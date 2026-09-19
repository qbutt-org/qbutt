/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "networkdiagnosticsdialog.h"

#include <algorithm>
#include <limits>
#include <utility>

#include <QDialogButtonBox>
#include <QFileDialog>
#include <QFuture>
#include <QHeaderView>
#include <QJsonArray>
#include <QJsonDocument>
#include <QLabel>
#include <QListWidget>
#include <QPushButton>
#include <QSaveFile>
#include <QTableWidget>
#include <QTimer>
#include <QVBoxLayout>

#include "base/bittorrent/session.h"
#include "base/bittorrent/torrent.h"
#include "base/global.h"
#include "base/net/pathmanager.h"
#include "base/utils/misc.h"

namespace
{
    constexpr qsizetype MAX_SAMPLES = 300;
    constexpr qint64 SAMPLE_TTL_MILLISECONDS = 15 * 60 * 1000;

    QJsonObject numericTorrentStatus(const BitTorrent::TorrentDiagnosticStatus &status)
    {
        return {{u"isFinished"_s, status.isFinished}, {u"isStopped"_s, status.isStopped},
            {u"hasMetadata"_s, status.hasMetadata}, {u"expectsConnections"_s, status.expectsConnections},
            {u"expectsDownload"_s, status.expectsDownload},
            {u"knownPeers"_s, status.knownPeers},
            {u"connectionCandidates"_s, status.connectionCandidates},
            {u"connections"_s, status.connections}, {u"establishedPeers"_s, status.establishedPeers},
            {u"wireDownloadRate"_s, status.wireDownloadRate}, {u"wireUploadRate"_s, status.wireUploadRate},
            {u"payloadDownloadRate"_s, status.payloadDownloadRate}, {u"payloadUploadRate"_s, status.payloadUploadRate},
            {u"totalPayloadDownload"_s, status.totalPayloadDownload},
            {u"totalPayloadUpload"_s, status.totalPayloadUpload}, {u"failedBytes"_s, status.failedBytes},
            {u"redundantBytes"_s, status.redundantBytes}};
    }

    QJsonObject numericPeerAggregate(const BitTorrent::TorrentPeerDiagnosticStatus &peers, QJsonArray &paths)
    {
        const auto numeric = [](const BitTorrent::PeerDiagnosticStatus &status)
        {
            return QJsonObject {{u"peers"_s, status.peers}, {u"connecting"_s, status.connecting},
                {u"handshaking"_s, status.handshaking}, {u"transferring"_s, status.transferring},
                {u"choked"_s, status.choked}, {u"noDemand"_s, status.noDemand},
                {u"diskQueued"_s, status.diskQueued}, {u"rateLimited"_s, status.rateLimited},
                {u"sourceMask"_s, status.sourceMask}, {u"payloadDownloadRate"_s, status.payloadDownloadRate},
                {u"wireDownloadRate"_s, status.wireDownloadRate}};
        };
        for (const BitTorrent::PeerPathDiagnosticStatus &path : peers.paths)
        {
            QJsonObject entry = numeric(path);
            entry.insert(u"pathId"_s, QString::number(path.pathId));
            entry.insert(u"generation"_s, static_cast<qint64>(path.generation));
            paths.append(entry);
        }
        return numeric(peers);
    }

    QJsonObject safeWire(const QJsonObject &source)
    {
        const QStringList keys {u"relayDownloadBytes"_s, u"relayUploadBytes"_s,
            u"carrierDownloadBytes"_s, u"carrierUploadBytes"_s, u"carrierDownloadPackets"_s,
            u"carrierUploadPackets"_s, u"relayDownloadCopies"_s};
        QJsonObject result;
        for (const QString &key : keys)
        {
            const QJsonValue value = source.value(key);
            if (!value.isDouble() || (value.toInteger(-1) < 0))
                return {};
            result.insert(key, value.toInteger());
        }
        return result;
    }

    QJsonObject safeGateway(const QJsonObject &source)
    {
        const QString state = source.value(u"state"_s).toString();
        if ((state == u"outgoing-only") && source.value(u"tcp"_s).isBool()
            && source.value(u"udp"_s).isBool() && !source.value(u"tcp"_s).toBool()
            && !source.value(u"udp"_s).toBool())
        {
            return {{u"state"_s, state}, {u"tcp"_s, source.value(u"tcp"_s).toBool()},
                {u"udp"_s, source.value(u"udp"_s).toBool()}};
        }
        if ((state == u"leased") && !source.value(u"publicEndpoint"_s).toString().isEmpty()
            && source.value(u"tcp"_s).isBool() && source.value(u"udp"_s).isBool())
        {
            return {{u"state"_s, state}, {u"publicEndpoint"_s, source.value(u"publicEndpoint"_s).toString()},
                {u"tcp"_s, source.value(u"tcp"_s).toBool()}, {u"udp"_s, source.value(u"udp"_s).toBool()}};
        }
        return source.isEmpty() ? QJsonObject {} : QJsonObject {{u"state"_s, u"unknown"_s}};
    }

    void addReason(QJsonArray &reasons, const QString &code, const QString &severity, const int count = 1)
    {
        reasons.append(QJsonObject {{u"code"_s, code}, {u"severity"_s, severity}, {u"count"_s, count}});
    }

    QJsonObject numericFields(const QJsonObject &source, const QStringList &keys)
    {
        QJsonObject result;
        for (const QString &key : keys)
        {
            const QJsonValue value = source.value(key);
            if (value.isDouble() && (value.toInteger(-1) >= 0))
                result.insert(key, value.toInteger());
        }
        return result;
    }

    void copyBool(QJsonObject &target, const QJsonObject &source, const QString &key)
    {
        if (source.value(key).isBool())
            target.insert(key, source.value(key).toBool());
    }

    QString safeEnum(const QJsonObject &source, const QString &key, const QStringList &allowed,
        const QString &fallback = {})
    {
        const QString value = source.value(key).toString();
        return allowed.contains(value) ? value : fallback;
    }

    QString reasonText(const QString &code, const int count)
    {
        if (code == u"no-candidates") return NetworkDiagnosticsDialog::tr("No peer connection candidates are available.");
        if (code == u"connecting") return NetworkDiagnosticsDialog::tr("%n peer connection(s) have not completed the handshake.", nullptr, count);
        if (code == u"choked") return NetworkDiagnosticsDialog::tr("%n wanted peer connection(s) are choked by the remote side.", nullptr, count);
        if (code == u"no-demand") return NetworkDiagnosticsDialog::tr("%n peer connection(s) have no wanted blocks.", nullptr, count);
        if (code == u"disk") return NetworkDiagnosticsDialog::tr("%n peer connection(s) are waiting for disk I/O.", nullptr, count);
        if (code == u"hash") return NetworkDiagnosticsDialog::tr("Hash verification rejected data since the previous sample.");
        if (code == u"rate") return NetworkDiagnosticsDialog::tr("%n peer connection(s) are waiting for bandwidth.", nullptr, count);
        if (code == u"transferring") return NetworkDiagnosticsDialog::tr("Payload is currently transferring.");
        if (code == u"stopped") return NetworkDiagnosticsDialog::tr("The torrent is stopped.");
        if (code == u"finished") return NetworkDiagnosticsDialog::tr("The wanted payload is complete.");
        if (code == u"metadata") return NetworkDiagnosticsDialog::tr("The torrent is downloading metadata; payload demand is not available yet.");
        if (code == u"inactive") return NetworkDiagnosticsDialog::tr("The current torrent state is not attempting a payload download.");
        return NetworkDiagnosticsDialog::tr("The torrent is idle without a single dominant cause.");
    }

    QString peerSourcesText(const int sources)
    {
        QStringList result;
        const auto add = [&result, sources](const int flag, const QString &name)
        {
            if (sources & flag)
                result.append(name);
        };
        add(BitTorrent::TrackerPeerSource, NetworkDiagnosticsDialog::tr("Tracker"));
        add(BitTorrent::DHTPeerSource, NetworkDiagnosticsDialog::tr("DHT"));
        add(BitTorrent::PeXPeerSource, NetworkDiagnosticsDialog::tr("Peer exchange"));
        add(BitTorrent::LSDPeerSource, NetworkDiagnosticsDialog::tr("Local discovery"));
        add(BitTorrent::ResumeDataPeerSource, NetworkDiagnosticsDialog::tr("Resume data"));
        add(BitTorrent::IncomingPeerSource, NetworkDiagnosticsDialog::tr("Incoming"));
        add(BitTorrent::WebSeedPeerSource, NetworkDiagnosticsDialog::tr("Web seed"));
        return result.isEmpty() ? NetworkDiagnosticsDialog::tr("None among active peers") : result.join(u", "_s);
    }
}

NetworkDiagnosticsDialog::NetworkDiagnosticsDialog(QWidget *parent, BitTorrent::Torrent *torrent)
    : QDialog {parent}
    , m_torrent {torrent}
    , m_timer {new QTimer {this}}
    , m_torrentLabel {new QLabel {this}}
    , m_summary {new QTableWidget {this}}
    , m_paths {new QTableWidget {this}}
    , m_reasons {new QListWidget {this}}
    , m_refresh {new QPushButton {tr("Refresh"), this}}
    , m_export {new QPushButton {tr("Export anonymized diagnostics..."), this}}
    , m_status {new QLabel {this}}
{
    setObjectName(u"networkDiagnosticsDialog"_s);
    m_torrentLabel->setObjectName(u"diagnosticsTorrent"_s);
    m_summary->setObjectName(u"diagnosticsSummary"_s);
    m_paths->setObjectName(u"diagnosticsPaths"_s);
    m_reasons->setObjectName(u"diagnosticsReasons"_s);
    m_refresh->setObjectName(u"diagnosticsRefresh"_s);
    m_export->setObjectName(u"diagnosticsExport"_s);
    m_status->setObjectName(u"diagnosticsStatus"_s);

    setWindowTitle(tr("Network diagnostics"));
    resize(920, 680);
    m_torrentLabel->setText(torrent ? torrent->name() : tr("Torrent removed"));
    m_summary->setColumnCount(2);
    m_summary->horizontalHeader()->hide();
    m_summary->verticalHeader()->hide();
    m_summary->horizontalHeader()->setStretchLastSection(true);
    m_summary->setEditTriggers(QAbstractItemView::NoEditTriggers);
    m_paths->setColumnCount(13);
    m_paths->setHorizontalHeaderLabels({tr("Path"), tr("Generation"), tr("Peers (torrent)"), tr("Attempts"),
        tr("Connected"), tr("Failures"), tr("Verified bytes (session)"), tr("Verified rate (session)"),
        tr("Peer payload bytes (session)"), tr("Peer payload rate (torrent)"), tr("Relay bytes"),
        tr("Carrier wire bytes"), tr("Inbound")});
    m_paths->setEditTriggers(QAbstractItemView::NoEditTriggers);
    m_paths->horizontalHeader()->setStretchLastSection(true);

    auto *buttons = new QDialogButtonBox {QDialogButtonBox::Close, this};
    buttons->addButton(m_refresh, QDialogButtonBox::ActionRole);
    buttons->addButton(m_export, QDialogButtonBox::ActionRole);
    auto *layout = new QVBoxLayout {this};
    layout->addWidget(m_torrentLabel);
    layout->addWidget(new QLabel {tr("Current transfer evidence"), this});
    layout->addWidget(m_summary);
    auto *pathScope = new QLabel {tr("Path totals include all torrents in this session, including closed connections. "
        "Verified bytes count downloaded blocks once after a successful hash check, excluding padding and redundant copies. "
        "Existing local data is not counted. Retired path totals expire after 15 minutes without activity."), this};
    pathScope->setWordWrap(true);
    layout->addWidget(pathScope);
    layout->addWidget(m_paths);
    layout->addWidget(new QLabel {tr("Likely causes"), this});
    layout->addWidget(m_reasons);
    layout->addWidget(m_status);
    layout->addWidget(buttons);

    connect(buttons, &QDialogButtonBox::rejected, this, &QDialog::reject);
    connect(m_refresh, &QPushButton::clicked, this, &NetworkDiagnosticsDialog::refreshNow);
    connect(m_export, &QPushButton::clicked, this, &NetworkDiagnosticsDialog::exportDiagnostics);
    connect(m_timer, &QTimer::timeout, this, &NetworkDiagnosticsDialog::refreshNow);
    m_export->setEnabled(false);
    m_clock.start();
    m_timer->start(1000);
    refreshNow();
}

BitTorrent::Torrent *NetworkDiagnosticsDialog::torrent() const
{
    return m_torrent;
}

QJsonObject NetworkDiagnosticsDialog::currentSnapshot() const
{
    if (m_samples.isEmpty())
        return {};
    const qint64 cutoff = m_clock.elapsed() - SAMPLE_TTL_MILLISECONDS;
    return (m_samples.constLast().value(u"capturedElapsedMilliseconds"_s).toInteger() < cutoff)
        ? QJsonObject {} : m_samples.constLast();
}

void NetworkDiagnosticsDialog::refreshNow()
{
    prune();
    m_export->setEnabled(!m_samples.isEmpty());
    if (m_refreshing)
        return;
    if (!m_torrent)
    {
        m_refresh->setEnabled(false);
        m_export->setEnabled(!m_samples.isEmpty());
        m_status->setText(m_samples.isEmpty()
            ? tr("Torrent was removed and retained diagnostics have expired.")
            : tr("Torrent was removed. Retained samples remain available for export."));
        return;
    }

    m_refreshing = true;
    m_refresh->setEnabled(false);
    m_torrent->fetchPeerDiagnosticStatus().then(this,
        [this](const BitTorrent::TorrentPeerDiagnosticStatus &peers)
        {
            m_refreshing = false;
            m_refresh->setEnabled(!m_torrent.isNull());
            if (!m_torrent)
                return;
            if (!peers.known)
            {
                m_status->setText(tr("Unable to read peer diagnostics."));
                return;
            }
            const BitTorrent::TorrentDiagnosticStatus status = m_torrent->diagnosticStatus();
            QJsonObject snapshot = buildSnapshot(status, peers);
            retain(snapshot);
            render(snapshot);
        });
}

QJsonObject NetworkDiagnosticsDialog::buildSnapshot(const BitTorrent::TorrentDiagnosticStatus &status,
    const BitTorrent::TorrentPeerDiagnosticStatus &peers) const
{
    QJsonArray peerPaths;
    const QJsonObject peerTotals = numericPeerAggregate(peers, peerPaths);
    const QJsonObject selector = BitTorrent::Session::instance()->peerRouteDiagnostics();
    const QJsonObject pathState = Net::PathManager::instance()->statusData();

    QJsonArray paths;
    for (const QJsonValue &value : pathState.value(u"paths"_s).toArray())
    {
        const QJsonObject path = value.toObject();
        QJsonObject entry {{u"pathId"_s, path.value(u"pathId"_s)}, {u"generation"_s, path.value(u"generation"_s)},
            {u"edgeId"_s, path.value(u"edgeId"_s)}, {u"proxyName"_s, path.value(u"proxyName"_s)},
            {u"open"_s, path.value(u"open"_s)}};
        const QJsonObject gateway = safeGateway(path.value(u"gateway"_s).toObject());
        if (!gateway.isEmpty())
        {
            entry.insert(u"gateway"_s, QJsonObject {{u"state"_s, gateway.value(u"state"_s)},
                {u"publicEndpoint"_s, gateway.value(u"publicEndpoint"_s)}, {u"tcp"_s, gateway.value(u"tcp"_s)},
                {u"udp"_s, gateway.value(u"udp"_s)}});
        }
        const QJsonObject wire = safeWire(path.value(u"wire"_s).toObject());
        if (!wire.isEmpty())
            entry.insert(u"wire"_s, wire);
        paths.append(entry);
    }

    QJsonObject recentDecisions;
    for (const QJsonValue &value : selector.value(u"events"_s).toArray())
    {
        const QJsonObject event = value.toObject();
        if ((event.value(u"ageMilliseconds"_s).toInteger() > 60000)
            || (event.value(u"event"_s).toString() != u"selected"))
            continue;
        const QString decision = event.value(u"decision"_s).toString();
        if ((decision == u"pinned") || (decision == u"best-score") || (decision == u"exploration")
            || (decision == u"blocked-no-route") || (decision == u"blocked-cooldown"))
            recentDecisions.insert(decision, recentDecisions.value(decision).toInt() + 1);
    }
    const QJsonObject previousSnapshot = currentSnapshot();
    const QJsonObject previousTorrent = previousSnapshot.value(u"torrent"_s).toObject();
    const qint64 failedBytesDelta = previousTorrent.isEmpty() ? 0
        : std::max<qint64>(0, status.failedBytes - previousTorrent.value(u"failedBytes"_s).toInteger());
    QJsonObject torrentStatus = numericTorrentStatus(status);
    torrentStatus.insert(u"failedBytesDelta"_s, failedBytesDelta);
    const int unfinished = peerTotals.value(u"connecting"_s).toInt() + peerTotals.value(u"handshaking"_s).toInt();
    QJsonArray reasons;
    if (status.expectsConnections && (status.connections == 0)
        && (status.connectionCandidates == 0) && (status.payloadDownloadRate == 0))
        addReason(reasons, u"no-candidates"_s, u"info"_s);
    if (status.expectsConnections && (unfinished > 0))
        addReason(reasons, u"connecting"_s, u"warning"_s, unfinished);
    if (status.expectsDownload && (peerTotals.value(u"choked"_s).toInt() > 0)
        && (status.payloadDownloadRate == 0))
        addReason(reasons, u"choked"_s, u"warning"_s, peerTotals.value(u"choked"_s).toInt());
    if (status.expectsDownload && (peerTotals.value(u"noDemand"_s).toInt() > 0)
        && (status.payloadDownloadRate == 0))
        addReason(reasons, u"no-demand"_s, u"info"_s, peerTotals.value(u"noDemand"_s).toInt());
    if (status.expectsDownload && (status.payloadDownloadRate == 0)
        && (peerTotals.value(u"diskQueued"_s).toInt() > 0))
        addReason(reasons, u"disk"_s, u"warning"_s, std::max(1, peerTotals.value(u"diskQueued"_s).toInt()));
    if (failedBytesDelta > 0)
        addReason(reasons, u"hash"_s, u"warning"_s);
    if (status.expectsDownload && (status.payloadDownloadRate == 0)
        && (peerTotals.value(u"rateLimited"_s).toInt() > 0))
        addReason(reasons, u"rate"_s, u"info"_s,
            peerTotals.value(u"rateLimited"_s).toInt());
    if (reasons.isEmpty())
    {
        QString code = u"idle"_s;
        if (status.isStopped)
            code = u"stopped"_s;
        else if (status.isFinished)
            code = u"finished"_s;
        else if (status.expectsConnections && !status.expectsDownload)
            code = u"metadata"_s;
        else if (!status.expectsDownload)
            code = u"inactive"_s;
        else if (status.payloadDownloadRate > 0)
            code = u"transferring"_s;
        addReason(reasons, code, u"info"_s);
    }

    QHash<QString, QJsonObject> previousRoutes;
    for (const QJsonValue &value : previousSnapshot.value(u"selector"_s).toObject().value(u"routes"_s).toArray())
    {
        const QJsonObject route = value.toObject();
        previousRoutes.insert(route.value(u"pathId"_s).toString() + u':'
            + QString::number(route.value(u"generation"_s).toInteger()), route);
    }
    QJsonArray routes;
    const qint64 previousElapsed = previousSnapshot.value(u"capturedElapsedMilliseconds"_s).toInteger(-1);
    const qint64 elapsed = m_clock.elapsed();
    for (const QJsonValue &value : selector.value(u"routes"_s).toArray())
    {
        QJsonObject route = value.toObject();
        const QString key = route.value(u"pathId"_s).toString() + u':'
            + QString::number(route.value(u"generation"_s).toInteger());
        const QJsonObject previous = previousRoutes.value(key);
        if (!previous.isEmpty() && (previousElapsed >= 0) && (elapsed > previousElapsed))
        {
            const qint64 delta = std::max<qint64>(0, route.value(u"verifiedDownload"_s).toInteger()
                - previous.value(u"verifiedDownload"_s).toInteger());
            const long double rate = (static_cast<long double>(delta) * 1000) / (elapsed - previousElapsed);
            route.insert(u"verifiedDownloadRate"_s, static_cast<qint64>(std::min<long double>(
                rate, std::numeric_limits<qint64>::max())));
        }
        routes.append(route);
    }
    const QJsonObject selectorSample {{u"scope"_s, u"session"_s},
        {u"blockedSelections"_s, selector.value(u"blockedSelections"_s)},
        {u"eventsTruncated"_s, selector.value(u"eventsTruncated"_s)},
        {u"routes"_s, routes}, {u"recentDecisions"_s, recentDecisions}};

    return {{u"capturedElapsedMilliseconds"_s, elapsed},
        {u"torrent"_s, torrentStatus}, {u"peers"_s, peerTotals}, {u"peerPaths"_s, peerPaths},
        {u"paths"_s, paths}, {u"selector"_s, selectorSample}, {u"reasons"_s, reasons}};
}

QJsonObject NetworkDiagnosticsDialog::anonymizedExport() const
{
    const qint64 now = m_clock.elapsed();
    const qint64 cutoff = now - SAMPLE_TTL_MILLISECONDS;
    QHash<QString, QString> aliases;
    const auto alias = [&aliases](const QJsonValue &pathId)
    {
        const QString id = pathId.toString();
        if (!aliases.contains(id))
            aliases.insert(id, u"path-%1"_s.arg(aliases.size() + 1));
        return aliases.value(id);
    };
    const QStringList torrentKeys {u"knownPeers"_s, u"connectionCandidates"_s, u"connections"_s,
        u"establishedPeers"_s, u"wireDownloadRate"_s, u"wireUploadRate"_s, u"payloadDownloadRate"_s,
        u"payloadUploadRate"_s,
        u"totalPayloadDownload"_s, u"totalPayloadUpload"_s, u"failedBytes"_s, u"failedBytesDelta"_s,
        u"redundantBytes"_s};
    const QStringList peerKeys {u"peers"_s, u"connecting"_s, u"handshaking"_s, u"transferring"_s,
        u"choked"_s, u"noDemand"_s, u"diskQueued"_s, u"rateLimited"_s, u"sourceMask"_s,
        u"payloadDownloadRate"_s, u"wireDownloadRate"_s};
    const QStringList routeKeys {u"generation"_s, u"attempts"_s, u"connected"_s,
        u"closed"_s, u"connectionFailures"_s, u"timeouts"_s, u"payloadDownload"_s, u"payloadUpload"_s,
        u"verifiedDownload"_s, u"verifiedDownloadRate"_s, u"demandMilliseconds"_s, u"chokedMilliseconds"_s};
    QJsonArray samples;
    for (const QJsonObject &sample : m_samples)
    {
        const qint64 captured = sample.value(u"capturedElapsedMilliseconds"_s).toInteger();
        if (captured < cutoff)
            continue;
        QJsonArray paths;
        for (const QJsonValue &value : sample.value(u"paths"_s).toArray())
        {
            const QJsonObject raw = value.toObject();
            QJsonObject path {{u"path"_s, alias(raw.value(u"pathId"_s))}};
            path.insert(u"generation"_s, numericFields(raw, {u"generation"_s}).value(u"generation"_s));
            copyBool(path, raw, u"open"_s);
            const QJsonObject gateway = safeGateway(raw.value(u"gateway"_s).toObject());
            if (!gateway.isEmpty())
            {
                const QString state = safeEnum(gateway, u"state"_s,
                    {u"leased"_s, u"outgoing-only"_s, u"unknown"_s}, u"unknown"_s);
                QJsonObject exportedGateway {{u"state"_s, state}};
                if (state != u"unknown")
                {
                    copyBool(exportedGateway, gateway, u"tcp"_s);
                    copyBool(exportedGateway, gateway, u"udp"_s);
                    if (state == u"leased")
                        exportedGateway.insert(u"endpointAvailable"_s, true);
                }
                path.insert(u"gateway"_s, exportedGateway);
            }
            const QJsonObject rawWire = safeWire(raw.value(u"wire"_s).toObject());
            if (!rawWire.isEmpty())
                path.insert(u"wire"_s, rawWire);
            paths.append(path);
        }
        QJsonArray peerPaths;
        for (const QJsonValue &value : sample.value(u"peerPaths"_s).toArray())
        {
            const QJsonObject raw = value.toObject();
            QJsonObject path = numericFields(raw, peerKeys);
            path.insert(u"path"_s, alias(raw.value(u"pathId"_s)));
            path.insert(u"generation"_s, numericFields(raw, {u"generation"_s}).value(u"generation"_s));
            peerPaths.append(path);
        }
        const QJsonObject selector = sample.value(u"selector"_s).toObject();
        QJsonArray routes;
        for (const QJsonValue &value : selector.value(u"routes"_s).toArray())
        {
            const QJsonObject raw = value.toObject();
            QJsonObject route = numericFields(raw, routeKeys);
            route.insert(u"path"_s, alias(raw.value(u"pathId"_s)));
            route.insert(u"type"_s, safeEnum(raw, u"type"_s,
                {u"blocked"_s, u"relay"_s, u"native"_s, u"unknown"_s}, u"unknown"_s));
            routes.append(route);
        }
        QJsonObject recentDecisions;
        const QJsonObject rawDecisions = selector.value(u"recentDecisions"_s).toObject();
        for (const QString &decision : {u"pinned"_s, u"best-score"_s, u"exploration"_s,
                 u"blocked-no-route"_s, u"blocked-cooldown"_s})
        {
            const QJsonObject value = numericFields(rawDecisions, {decision});
            if (value.contains(decision))
                recentDecisions.insert(decision, value.value(decision));
        }
        QJsonArray reasons;
        for (const QJsonValue &value : sample.value(u"reasons"_s).toArray())
        {
            const QJsonObject raw = value.toObject();
            const QString code = safeEnum(raw, u"code"_s, {u"no-candidates"_s, u"connecting"_s, u"choked"_s,
                u"no-demand"_s, u"disk"_s, u"hash"_s, u"rate"_s, u"transferring"_s,
                u"stopped"_s, u"finished"_s, u"metadata"_s, u"inactive"_s, u"idle"_s});
            const QString severity = safeEnum(raw, u"severity"_s, {u"info"_s, u"warning"_s});
            if (code.isEmpty() || severity.isEmpty())
                continue;
            QJsonObject reason {{u"code"_s, code}, {u"severity"_s, severity}};
            reason.insert(u"count"_s, numericFields(raw, {u"count"_s}).value(u"count"_s));
            reasons.append(reason);
        }
        const QJsonObject rawTorrent = sample.value(u"torrent"_s).toObject();
        QJsonObject torrent = numericFields(rawTorrent, torrentKeys);
        for (const QString &key : {u"isFinished"_s, u"isStopped"_s, u"hasMetadata"_s,
                 u"expectsConnections"_s, u"expectsDownload"_s})
            copyBool(torrent, rawTorrent, key);
        QJsonObject selectorExport {{u"scope"_s, u"session"_s},
            {u"routes"_s, routes}, {u"recentDecisions"_s, recentDecisions}};
        const QJsonObject blocked = numericFields(selector, {u"blockedSelections"_s});
        if (blocked.contains(u"blockedSelections"_s))
            selectorExport.insert(u"blockedSelections"_s, blocked.value(u"blockedSelections"_s));
        copyBool(selectorExport, selector, u"eventsTruncated"_s);
        samples.append(QJsonObject {{u"ageMilliseconds"_s,
            std::max<qint64>(0, now - captured)}, {u"torrent"_s, torrent},
            {u"peers"_s, numericFields(sample.value(u"peers"_s).toObject(), peerKeys)}, {u"peerPaths"_s, peerPaths},
            {u"paths"_s, paths}, {u"selector"_s, selectorExport},
            {u"reasons"_s, reasons}});
    }
    return {{u"schema"_s, u"qbutt-diagnostics-v1"_s}, {u"torrent"_s, u"torrent-1"_s},
        {u"samples"_s, samples}};
}

void NetworkDiagnosticsDialog::retain(QJsonObject snapshot)
{
    prune();
    while (m_samples.size() >= MAX_SAMPLES)
        m_samples.removeFirst();
    m_samples.append(std::move(snapshot));
    m_export->setEnabled(true);
}

void NetworkDiagnosticsDialog::prune()
{
    const qint64 cutoff = m_clock.elapsed() - SAMPLE_TTL_MILLISECONDS;
    m_samples.removeIf([cutoff](const QJsonObject &sample)
    {
        return sample.value(u"capturedElapsedMilliseconds"_s).toInteger() < cutoff;
    });
}

void NetworkDiagnosticsDialog::render(const QJsonObject &snapshot)
{
    const QJsonObject torrent = snapshot.value(u"torrent"_s).toObject();
    const QJsonObject peers = snapshot.value(u"peers"_s).toObject();
    int remotePaths = 0;
    int wireKnownPaths = 0;
    qint64 relayDownload = 0;
    qint64 relayUpload = 0;
    qint64 carrierDownload = 0;
    qint64 carrierUpload = 0;
    for (const QJsonValue &value : snapshot.value(u"paths"_s).toArray())
    {
        const QJsonObject path = value.toObject();
        if (path.value(u"edgeId"_s).toString() == u"native")
            continue;
        ++remotePaths;
        const QJsonObject wire = safeWire(path.value(u"wire"_s).toObject());
        if (wire.isEmpty())
            continue;
        ++wireKnownPaths;
        relayDownload += wire.value(u"relayDownloadBytes"_s).toInteger();
        relayUpload += wire.value(u"relayUploadBytes"_s).toInteger();
        carrierDownload += wire.value(u"carrierDownloadBytes"_s).toInteger();
        carrierUpload += wire.value(u"carrierUploadBytes"_s).toInteger();
    }
    const bool wireKnown = (remotePaths > 0) && (wireKnownPaths == remotePaths);
    const QString relaySummary = wireKnown ? tr("%1 down / %2 up")
        .arg(Utils::Misc::friendlyUnit(relayDownload), Utils::Misc::friendlyUnit(relayUpload))
        : tr("Unknown (transport does not report this counter)");
    const QString carrierSummary = wireKnown ? tr("%1 down / %2 up")
        .arg(Utils::Misc::friendlyUnit(carrierDownload), Utils::Misc::friendlyUnit(carrierUpload))
        : tr("Unknown (transport does not report this counter)");
    const QJsonObject decisions = snapshot.value(u"selector"_s).toObject().value(u"recentDecisions"_s).toObject();
    const QString decisionCounts = tr("Pinned %1 · scored %2 · exploration %3 · denied %4")
        .arg(decisions.value(u"pinned"_s).toInt()).arg(decisions.value(u"best-score"_s).toInt())
        .arg(decisions.value(u"exploration"_s).toInt())
        .arg(decisions.value(u"blocked-no-route"_s).toInt() + decisions.value(u"blocked-cooldown"_s).toInt());
    const QString decisionSummary = snapshot.value(u"selector"_s).toObject().value(u"eventsTruncated"_s).toBool()
        ? tr("Bounded event tail (counts may be lower): %1").arg(decisionCounts) : decisionCounts;
    const QList<QPair<QString, QString>> summary {
        {tr("Candidates / connections / established"), tr("%1 / %2 / %3").arg(torrent.value(u"connectionCandidates"_s).toInt())
            .arg(torrent.value(u"connections"_s).toInt()).arg(torrent.value(u"establishedPeers"_s).toInt())},
        {tr("Sources among active peers"), peerSourcesText(peers.value(u"sourceMask"_s).toInt())},
        {tr("Transferring / choked / no wanted blocks"), tr("%1 / %2 / %3").arg(peers.value(u"transferring"_s).toInt())
            .arg(peers.value(u"choked"_s).toInt()).arg(peers.value(u"noDemand"_s).toInt())},
        {tr("Peer payload down"), Utils::Misc::friendlyUnit(torrent.value(u"payloadDownloadRate"_s).toInteger(), true)},
        {tr("Peer protocol wire down"), Utils::Misc::friendlyUnit(torrent.value(u"wireDownloadRate"_s).toInteger(), true)},
        {tr("Verified download rate"), tr("See per-path session rates below")},
        {tr("Relay bytes"), relaySummary},
        {tr("Carrier wire bytes"), carrierSummary},
        {tr("Recent session route decisions"), decisionSummary},
        {tr("Disk / bandwidth waits"), tr("%1 / %2").arg(peers.value(u"diskQueued"_s).toInt())
            .arg(peers.value(u"rateLimited"_s).toInt())},
        {tr("Hash verification rejected since previous sample"),
            Utils::Misc::friendlyUnit(torrent.value(u"failedBytesDelta"_s).toInteger())}};
    m_summary->setRowCount(summary.size());
    for (qsizetype row = 0; row < summary.size(); ++row)
    {
        m_summary->setItem(row, 0, new QTableWidgetItem {summary[row].first});
        m_summary->setItem(row, 1, new QTableWidgetItem {summary[row].second});
    }
    m_summary->resizeColumnsToContents();

    QHash<QString, QJsonObject> pathState;
    for (const QJsonValue &value : snapshot.value(u"paths"_s).toArray())
    {
        const QJsonObject path = value.toObject();
        pathState.insert(path.value(u"pathId"_s).toString() + u':' + QString::number(path.value(u"generation"_s).toInteger()), path);
    }
    QHash<QString, QJsonObject> peerPaths;
    for (const QJsonValue &value : snapshot.value(u"peerPaths"_s).toArray())
    {
        const QJsonObject path = value.toObject();
        peerPaths.insert(path.value(u"pathId"_s).toString() + u':' + QString::number(path.value(u"generation"_s).toInteger()), path);
    }
    QHash<QString, QJsonObject> routeState;
    for (const QJsonValue &value : snapshot.value(u"selector"_s).toObject().value(u"routes"_s).toArray())
    {
        const QJsonObject route = value.toObject();
        routeState.insert(route.value(u"pathId"_s).toString() + u':'
            + QString::number(route.value(u"generation"_s).toInteger()), route);
    }
    QStringList keys = pathState.keys();
    for (const QString &key : peerPaths.keys())
    {
        if (!keys.contains(key))
            keys.append(key);
    }
    for (const QString &key : routeState.keys())
    {
        if (!keys.contains(key))
            keys.append(key);
    }
    std::ranges::sort(keys);
    m_paths->setRowCount(keys.size());
    for (qsizetype row = 0; row < keys.size(); ++row)
    {
        const QString &key = keys[row];
        const QJsonObject path = pathState.value(key);
        const QJsonObject peer = peerPaths.value(key);
        const QJsonObject route = routeState.value(key);
        const QString pathId = !path.isEmpty() ? path.value(u"pathId"_s).toString()
            : (!peer.isEmpty() ? peer.value(u"pathId"_s).toString() : route.value(u"pathId"_s).toString());
        const qint64 generation = !path.isEmpty() ? path.value(u"generation"_s).toInteger()
            : (!peer.isEmpty() ? peer.value(u"generation"_s).toInteger() : route.value(u"generation"_s).toInteger());
        const QString name = path.value(u"proxyName"_s).toString();
        const QString routeType = route.value(u"type"_s).toString();
        const QString pathLabel = ((path.value(u"edgeId"_s).toString() == u"native") || (routeType == u"native"))
            ? tr("Native") : ((routeType == u"blocked") ? tr("Blocked route")
                : ((pathId == u"0") ? tr("Unmanaged/default route")
                    : (name.isEmpty() ? tr("Path %1").arg(pathId) : name)));
        const QJsonObject gateway = safeGateway(path.value(u"gateway"_s).toObject());
        const QJsonObject wire = safeWire(path.value(u"wire"_s).toObject());
        QString inbound = tr("Unknown");
        if (!gateway.isEmpty())
        {
            const QString gatewayState = gateway.value(u"state"_s).toString();
            if (gatewayState == u"leased")
                inbound = tr("%1 (TCP %2, UDP %3)").arg(gateway.value(u"publicEndpoint"_s).toString(),
                    gateway.value(u"tcp"_s).toBool() ? tr("yes") : tr("no"),
                    gateway.value(u"udp"_s).toBool() ? tr("yes") : tr("no"));
            else if (gatewayState == u"outgoing-only")
                inbound = tr("Outgoing only");
        }
        const QString relay = wire.isEmpty() ? tr("Unknown") : tr("%1 down / %2 up")
            .arg(Utils::Misc::friendlyUnit(wire.value(u"relayDownloadBytes"_s).toInteger()),
                Utils::Misc::friendlyUnit(wire.value(u"relayUploadBytes"_s).toInteger()));
        const QString carrier = wire.isEmpty() ? tr("Unknown") : tr("%1 down / %2 up")
            .arg(Utils::Misc::friendlyUnit(wire.value(u"carrierDownloadBytes"_s).toInteger()),
                Utils::Misc::friendlyUnit(wire.value(u"carrierUploadBytes"_s).toInteger()));
        const QString attempts = route.isEmpty() ? tr("Unknown")
            : QString::number(route.value(u"attempts"_s).toInteger());
        const QString connected = route.isEmpty() ? tr("Unknown")
            : QString::number(route.value(u"connected"_s).toInteger());
        const QString failures = route.isEmpty() ? tr("Unknown")
            : tr("%1 (%2 timeout)").arg(route.value(u"connectionFailures"_s).toInteger())
                .arg(route.value(u"timeouts"_s).toInteger());
        const QJsonValue verifiedRate = route.value(u"verifiedDownloadRate"_s);
        const QString verifiedBytes = route.isEmpty() ? tr("Unknown")
            : Utils::Misc::friendlyUnit(route.value(u"verifiedDownload"_s).toInteger());
        const QString payloadBytes = route.isEmpty() ? tr("Unknown") : tr("%1 down / %2 up")
            .arg(Utils::Misc::friendlyUnit(route.value(u"payloadDownload"_s).toInteger()),
                Utils::Misc::friendlyUnit(route.value(u"payloadUpload"_s).toInteger()));
        const QStringList values {pathLabel, QString::number(generation),
            QString::number(peer.value(u"peers"_s).toInt()), attempts, connected, failures, verifiedBytes,
            verifiedRate.isDouble() ? Utils::Misc::friendlyUnit(verifiedRate.toInteger(), true) : tr("Unknown"),
            payloadBytes, Utils::Misc::friendlyUnit(peer.value(u"payloadDownloadRate"_s).toInteger(), true), relay, carrier, inbound};
        for (int column = 0; column < values.size(); ++column)
            m_paths->setItem(row, column, new QTableWidgetItem {values[column]});
    }
    m_paths->resizeColumnsToContents();

    m_reasons->clear();
    for (const QJsonValue &value : snapshot.value(u"reasons"_s).toArray())
    {
        const QJsonObject reason = value.toObject();
        m_reasons->addItem(reasonText(reason.value(u"code"_s).toString(), reason.value(u"count"_s).toInt()));
    }
    m_status->setText(tr("%n sample(s), capped at 300 and retained for at most 15 minutes.", nullptr,
        m_samples.size()));
}

void NetworkDiagnosticsDialog::exportDiagnostics()
{
    const QString fileName = QFileDialog::getSaveFileName(this, tr("Export anonymized diagnostics"),
        u"qbutt-diagnostics.json"_s, tr("JSON files (*.json)"));
    if (fileName.isEmpty())
        return;
    QSaveFile file {fileName};
    if (!file.open(QIODevice::WriteOnly))
    {
        m_status->setText(tr("Unable to open the export file."));
        return;
    }
    const QByteArray data = QJsonDocument {anonymizedExport()}.toJson(QJsonDocument::Indented);
    if ((file.write(data) != data.size()) || !file.commit())
    {
        m_status->setText(tr("Unable to write the export file."));
        return;
    }
    m_status->setText(tr("Anonymized diagnostics exported."));
}
