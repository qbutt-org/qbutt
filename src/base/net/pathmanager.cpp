/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "pathmanager.h"

#include <algorithm>

#ifdef Q_OS_WIN
#include <winsock2.h>
#include <ws2ipdef.h>
#include <iphlpapi.h>
#endif

#include <QCoreApplication>
#include <QDir>
#include <QFileInfo>
#include <QJsonDocument>
#include <QNetworkAddressEntry>
#include <QNetworkInterface>
#include <QNetworkProxy>
#include <QNetworkReply>
#include <QNetworkRequest>
#include <QSaveFile>
#include <QSignalBlocker>
#include <QUrl>

#include "base/bittorrent/session.h"
#include "base/global.h"
#include "base/path.h"
#include "base/profile.h"
#include "proxyconfigurationmanager.h"

namespace
{
    constexpr int MAX_FRAME_BYTES = 65536;
    constexpr int MAX_SUBSCRIPTION_BYTES = 2 * 1024 * 1024;
}

Net::PathManager *Net::PathManager::m_instance = nullptr;

Net::PathManager::PathManager()
    : m_status {ProxyConfigurationManager::instance()->hasRuntimeProxy()
        ? tr("Pinned path unavailable. Start a path to reconnect; automatic Native fallback is disabled.")
        : tr("Native / saved connection settings. No qbutt-net path is active.")}
    , m_storeSubscriptionUrl {u"Network/Paths/SubscriptionUrl"_s}
    , m_storeConfigurationPath {u"Network/Paths/ConfigurationPath"_s}
    , m_storeProxyName {u"Network/Paths/ProxyName"_s}
    , m_storeInterfaceName {u"Network/Paths/InterfaceName"_s}
    , m_storeMixed {u"Network/Paths/Mixed"_s}
    , m_storeNativeInterface {u"Network/Paths/NativeInterface"_s}
{
    m_timeout.setSingleShot(true);
    m_timeout.setInterval(15000);
    // Child diagnostics are intentionally discarded. Its versioned control
    // responses contain safe errors; arbitrary transport logs may hold secrets.
    m_process.setStandardErrorFile(QProcess::nullDevice());
    // Subscription retrieval is an explicit control-network operation. It is
    // independent from the selected torrent path, including a failed path.
    m_network.setProxy(QNetworkProxy::NoProxy);
    connect(&m_timeout, &QTimer::timeout, this, [this]()
    {
        fail(tr("qbutt-net did not respond within 15 seconds."));
    });
    connect(&m_process, &QProcess::started, this, [this]()
    {
        send({{u"method"_s, u"hello"_s}});
    });
    connect(&m_process, &QProcess::readyReadStandardOutput, this, &PathManager::readOutput);
    connect(&m_process, &QProcess::errorOccurred, this, [this](QProcess::ProcessError)
    {
        fail(tr("Unable to run the bundled qbutt-net process."));
    });
    connect(&m_process, &QProcess::finished, this, [this](int, QProcess::ExitStatus)
    {
        fail(tr("qbutt-net stopped. The pinned path remains blocked."));
    });
    connect(BitTorrent::Session::instance(), &BitTorrent::Session::peerRouteClosed, this,
        [this](const quint64 pathId, const quint64 generation, const qint64 downloaded, const qint64 uploaded)
    {
        for (ActivePath &path : m_paths)
        {
            if ((path.endpoint.pathId == pathId) && (path.endpoint.generation == generation))
            {
                path.closedPayloadDownload += downloaded;
                path.closedPayloadUpload += uploaded;
                break;
            }
        }
    });
    if (ProxyConfigurationManager::instance()->hasRuntimeProxy())
        applyRoutes();
}

Net::PathManager::~PathManager()
{
    shutdown();
}

Net::PathManager *Net::PathManager::instance()
{
    return m_instance;
}

void Net::PathManager::initInstance()
{
    if (!m_instance)
        m_instance = new PathManager;
}

void Net::PathManager::freeInstance()
{
    delete m_instance;
    m_instance = nullptr;
}

bool Net::PathManager::isBusy() const
{
    return (m_pendingId != 0) || !m_queuedRequest.isEmpty() || m_subscriptionReply;
}

bool Net::PathManager::isOpen() const
{
    return !m_nativeEndpoints.isEmpty()
        || std::ranges::any_of(m_paths, [](const ActivePath &path) { return path.endpoint.port > 0; });
}

QString Net::PathManager::status() const
{
    return m_status;
}

QJsonObject Net::PathManager::statusData(const bool includePeers) const
{
    QJsonArray paths;
    for (const ActivePath &path : m_paths)
    {
        paths.append(QJsonObject {{u"pathId"_s, QString::number(path.endpoint.pathId)},
            {u"generation"_s, static_cast<qint64>(path.endpoint.generation)},
            {u"edgeId"_s, path.edgeId}, {u"proxyName"_s, path.proxyName},
            {u"open"_s, path.endpoint.port > 0},
            {u"interfaceName"_s, path.interfaceName}, {u"capabilities"_s, path.capabilities},
            {u"closedPayloadDownload"_s, path.closedPayloadDownload},
            {u"closedPayloadUpload"_s, path.closedPayloadUpload}});
    }
    for (const PeerRouteEndpoint &endpoint : m_nativeEndpoints)
    {
        paths.append(QJsonObject {{u"pathId"_s, QString::number(endpoint.pathId)},
            {u"generation"_s, static_cast<qint64>(endpoint.generation)}, {u"edgeId"_s, u"native"_s},
            {u"proxyName"_s, tr("Native")}, {u"interfaceName"_s, m_storeNativeInterface.get()},
            {u"open"_s, m_storeMixed.get()}, {u"localAddress"_s, endpoint.localAddress}});
    }
    return {{u"v"_s, 1}, {u"busy"_s, isBusy()}, {u"open"_s, isOpen()},
        {u"pinned"_s, ProxyConfigurationManager::instance()->hasRuntimeProxy()},
        {u"status"_s, m_status}, {u"processId"_s, m_process.processId()},
        {u"mode"_s, m_storeMixed.get() ? u"mixed"_s : u"pinned"_s},
        {u"nativeInterface"_s, m_storeNativeInterface.get()},
        {u"paths"_s, paths},
        {u"peers"_s, includePeers ? BitTorrent::Session::instance()->peerRouteStatus() : QJsonArray {}},
        {u"generation"_s, m_generation}, {u"nodes"_s, m_proxies},
        {u"proxyName"_s, proxyName()}, {u"interfaceName"_s, interfaceName()}};
}

QString Net::PathManager::subscriptionUrl() const
{
    return m_storeSubscriptionUrl;
}

QString Net::PathManager::configurationPath() const
{
    return m_storeConfigurationPath;
}

QString Net::PathManager::proxyName() const
{
    return m_storeProxyName;
}

QString Net::PathManager::interfaceName() const
{
    return m_storeInterfaceName;
}

void Net::PathManager::refreshSubscription(const QString &urlText)
{
    if (isBusy())
        return;
    const QUrl url(urlText.trimmed(), QUrl::StrictMode);
    if (!url.isValid() || (url.scheme() != u"https") || url.host().isEmpty()
        || !url.userInfo().isEmpty() || url.hasFragment())
    {
        reportError(tr("Enter an HTTPS subscription URL without user information or a fragment."));
        return;
    }

    QNetworkRequest request(url);
    request.setAttribute(QNetworkRequest::RedirectPolicyAttribute, QNetworkRequest::NoLessSafeRedirectPolicy);
    request.setMaximumRedirectsAllowed(5);
    request.setTransferTimeout(15000);
    auto *reply = m_network.get(request);
    m_subscriptionReply = reply;
    reply->setReadBufferSize(MAX_SUBSCRIPTION_BYTES + 1);
    auto *deadline = new QTimer(reply);
    deadline->setSingleShot(true);
    connect(deadline, &QTimer::timeout, reply, &QNetworkReply::abort);
    deadline->start(15000);

    const auto checkSize = [reply]()
    {
        if ((reply->bytesAvailable() > MAX_SUBSCRIPTION_BYTES)
            || (reply->header(QNetworkRequest::ContentLengthHeader).toLongLong() > MAX_SUBSCRIPTION_BYTES))
        {
            reply->abort();
        }
    };
    connect(reply, &QNetworkReply::readyRead, reply, checkSize);
    connect(reply, &QNetworkReply::metaDataChanged, reply, checkSize);
    connect(reply, &QNetworkReply::finished, this, [this, reply, url]()
    {
        m_subscriptionReply = nullptr;
        reply->deleteLater();
        if ((reply->error() != QNetworkReply::NoError)
            || (reply->attribute(QNetworkRequest::HttpStatusCodeAttribute).toInt() != 200)
            || (reply->bytesAvailable() > MAX_SUBSCRIPTION_BYTES))
        {
            reportError(tr("Unable to download the subscription (HTTPS, 2 MiB and 15 second limits)."));
            return;
        }
        const QByteArray config = reply->readAll();
        const QString configPath = (specialFolderLocation(SpecialFolder::Config) / Path(u"mihomo-subscription.yaml"_s)).toString();
        QSaveFile file(configPath);
        if (config.isEmpty() || !file.open(QIODevice::WriteOnly)
            || !file.setPermissions(QFileDevice::ReadOwner | QFileDevice::WriteOwner)
            || (file.write(config) != config.size()) || !file.commit())
        {
            reportError(tr("Unable to save the subscription in the private qbutt profile."));
            return;
        }
        m_storeSubscriptionUrl = url.toString(QUrl::FullyEncoded);
        inspectConfiguration(configPath);
    });
    emit changed();
}

void Net::PathManager::inspectConfiguration(const QString &configPath)
{
    request({{u"method"_s, u"list"_s}, {u"configPath"_s, QFileInfo(configPath).absoluteFilePath()}});
}

void Net::PathManager::openPath(const QString &configPath, const QString &proxyName,
    const QString &interfaceName, const QString &edgeId)
{
    if (isBusy())
    {
        reportError(tr("A path operation is already running."));
        return;
    }

    const auto *session = BitTorrent::Session::instance();
    auto *proxyManager = ProxyConfigurationManager::instance();
    if (!session->isRestored() || (!proxyManager->hasRuntimeProxy() && !session->canSwitchConnectionMode()))
    {
        reportError(tr("The initial Native-to-Pinned transition requires no torrents or metadata downloads. "
            "Reconnecting an unavailable pinned path preserves existing jobs."));
        return;
    }
    if (proxyName.isEmpty() || interfaceName.isEmpty())
    {
        reportError(tr("Select a proxy node and choose its physical interface."));
        return;
    }
    const QString selectedEdge = edgeId.isEmpty() ? proxyName : edgeId;
    quint64 pathId = 0;
    for (const ActivePath &path : m_paths)
    {
        if (path.edgeId == selectedEdge)
        {
            if (path.endpoint.port > 0)
            {
                reportError(tr("This edge already has an active transport. Disconnect it before choosing another transport."));
                return;
            }
            pathId = path.endpoint.pathId;
            break;
        }
    }
    if ((pathId == 0) && (m_paths.size() >= 8))
    {
        reportError(tr("Eight edges are already selected. Reset the selection before adding another edge."));
        return;
    }
    if (selectedEdge.toUtf8().size() > 128)
    {
        reportError(tr("The edge name must fit within 128 UTF-8 bytes."));
        return;
    }
    if (!proxyManager->hasRuntimeProxy() && !proxyManager->setRuntimeProxy(blockedRuntimeProxy()))
    {
        reportError(tr("Unable to save the pinned startup policy. The path was not started."));
        return;
    }

    ++m_generation;
    if (pathId == 0)
        pathId = ++m_nextPathId;
    m_status = tr("Starting path. Egress and network capabilities have not been probed.");
    request({{u"method"_s, u"open"_s}, {u"configPath"_s, QFileInfo(configPath).absoluteFilePath()},
        {u"proxyName"_s, proxyName}, {u"pathId"_s, QString::number(pathId)},
        {u"generation"_s, m_generation}, {u"interfaceName"_s, interfaceName}, {u"edgeId"_s, selectedEdge}});
}

void Net::PathManager::setPolicy(const QString &mode, const QString &nativeInterface)
{
    if (isBusy())
        return;
    if ((mode != u"mixed") && (mode != u"pinned"))
    {
        reportError(tr("Choose Pinned or Mixed TCP. Other network policies are not available yet."));
        return;
    }
    const bool mixed = (mode == u"mixed");
    QList<PeerRouteEndpoint> nativeEndpoints;
    if (mixed && !nativeInterface.isEmpty())
    {
        for (const QNetworkInterface &iface : QNetworkInterface::allInterfaces())
        {
            if ((iface.name() != nativeInterface) && (iface.humanReadableName() != nativeInterface))
                continue;
            if (!iface.flags().testFlag(QNetworkInterface::IsUp)
                || !iface.flags().testFlag(QNetworkInterface::IsRunning))
                break;
#ifdef Q_OS_WIN
            MIB_IF_ROW2 row {};
            row.InterfaceIndex = static_cast<NET_IFINDEX>(iface.index());
            if ((GetIfEntry2(&row) != NO_ERROR) || !row.InterfaceAndOperStatusFlags.HardwareInterface)
                break;
#endif
            bool haveIPv4 = false;
            bool haveIPv6 = false;
            for (const QNetworkAddressEntry &entry : iface.addressEntries())
            {
                const QHostAddress address = entry.ip();
                const bool ipv6 = address.protocol() == QAbstractSocket::IPv6Protocol;
                if (address.isNull() || address.isLoopback() || address.isLinkLocal()
                    || (ipv6 ? haveIPv6 : haveIPv4))
                    continue;
                PeerRouteEndpoint endpoint;
                endpoint.type = PeerRouteEndpoint::Type::Native;
                endpoint.pathId = ipv6 ? 2 : 1;
                endpoint.generation = m_nativeGeneration + 1;
                endpoint.localAddress = address.toString();
                endpoint.interfaceIndex = static_cast<quint32>(iface.index());
                nativeEndpoints.append(std::move(endpoint));
                (ipv6 ? haveIPv6 : haveIPv4) = true;
            }
            break;
        }
        if (nativeEndpoints.isEmpty())
        {
            reportError(tr("The selected physical Native interface has no usable address."));
            return;
        }
    }
    const bool previous = m_storeMixed;
    const QString previousInterface = m_storeNativeInterface;
    auto *proxyManager = ProxyConfigurationManager::instance();
    if (!proxyManager->hasRuntimeProxy())
    {
        if (!BitTorrent::Session::instance()->canSwitchConnectionMode())
        {
            reportError(tr("Starting a managed network policy requires no torrents or metadata downloads."));
            return;
        }
        if (!proxyManager->setRuntimeProxy(blockedRuntimeProxy()))
        {
            reportError(tr("Unable to save the managed startup policy."));
            return;
        }
    }
    m_storeMixed = mixed;
    m_storeNativeInterface = mixed ? nativeInterface : QString();
    if (((previous != m_storeMixed) || (previousInterface != m_storeNativeInterface))
        && !SettingsStorage::instance()->save())
    {
        m_storeMixed = previous;
        m_storeNativeInterface = previousInterface;
        reportError(tr("Unable to save the network policy."));
        return;
    }
    const QList<PeerRouteEndpoint> oldNativeEndpoints = std::move(m_nativeEndpoints);
    m_nativeEndpoints = std::move(nativeEndpoints);
    ++m_nativeGeneration;
    applyRoutes();
    if (!mixed)
    {
        for (qsizetype index = 1; index < m_paths.size(); ++index)
        {
            const auto &endpoint = m_paths.at(index).endpoint;
            BitTorrent::Session::instance()->invalidatePeerRoute(endpoint.pathId, endpoint.generation);
        }
    }
    for (const PeerRouteEndpoint &endpoint : oldNativeEndpoints)
        BitTorrent::Session::instance()->invalidatePeerRoute(endpoint.pathId, endpoint.generation);
    m_status = mixed ? tr("Mixed TCP: new peer connections use the selected edges. Private torrents stay pinned.")
        : tr("Pinned TCP: peer connections use the first selected edge.");
    emit changed();
}

void Net::PathManager::applyRoutes()
{
    QList<PeerRouteEndpoint> endpoints;
    for (const ActivePath &path : m_paths)
        endpoints.append(path.endpoint);
    // An unavailable pinned edge must never turn into Native for private torrents.
    if (endpoints.isEmpty())
        endpoints.append(PeerRouteEndpoint {});
    if (m_storeMixed)
        endpoints.append(m_nativeEndpoints);
    BitTorrent::Session::instance()->setPeerRoutes(endpoints, m_storeMixed);
}

void Net::PathManager::useNative()
{
    if (isBusy())
        return;
    const auto *session = BitTorrent::Session::instance();
    if (!session->canSwitchConnectionMode())
    {
        reportError(tr("Switching to the default connection requires no torrents or metadata downloads in this initial version."));
        return;
    }

    // Terminate accepted sockets before restoring saved connection settings.
    if (!shutdown())
    {
        reportError(tr("The previous qbutt-net process has not stopped. Native was not enabled."));
        return;
    }
    if (!ProxyConfigurationManager::instance()->clearRuntimeProxy())
    {
        fail(tr("Unable to save the Native startup policy. The pinned path remains blocked."));
        return;
    }
    BitTorrent::Session::instance()->resetPeerRoutes();
    m_paths.clear();
    m_nativeEndpoints.clear();
    m_status = tr("Native / saved connection settings. No qbutt-net path is active.");
    emit changed();
}

void Net::PathManager::request(QJsonObject message)
{
    if (isBusy())
        return;
    if (m_process.state() == QProcess::NotRunning)
    {
        m_queuedRequest = std::move(message);
        QString program = QDir(QCoreApplication::applicationDirPath()).filePath(u"qbutt-net"_s);
#ifdef Q_OS_WIN
        program += u".exe"_s;
#endif
        m_process.start(program, {u"--stdio"_s});
        m_timeout.start();
        emit changed();
    }
    else
    {
        send(std::move(message));
    }
}

void Net::PathManager::send(QJsonObject message)
{
    m_pendingId = ++m_nextId;
    message.insert(u"v"_s, 1);
    message.insert(u"id"_s, m_pendingId);
    m_pendingRequest = message;
    // Edge grouping belongs to the application, not to the transport process.
    message.remove(u"edgeId"_s);
    QByteArray frame = QJsonDocument(message).toJson(QJsonDocument::Compact);
    frame.append('\n');
    if ((frame.size() > MAX_FRAME_BYTES) || (m_process.write(frame) != frame.size()))
    {
        fail(tr("Unable to send a bounded qbutt-net control request."));
        return;
    }
    m_timeout.start();
    emit changed();
}

void Net::PathManager::readOutput()
{
    // Read in bounded increments; never accumulate unbounded child output.
    while ((m_process.bytesAvailable() > 0) || m_output.contains('\n'))
    {
        m_output += m_process.read(MAX_FRAME_BYTES + 1 - m_output.size());
        const qsizetype newline = m_output.indexOf('\n');
        if ((newline < 0) && (m_output.size() <= MAX_FRAME_BYTES))
            continue;
        if ((newline < 0) || (newline >= MAX_FRAME_BYTES))
        {
            fail(tr("qbutt-net exceeded the control frame size limit."));
            return;
        }
        const QJsonDocument document = QJsonDocument::fromJson(m_output.left(newline));
        m_output.remove(0, newline + 1);
        if (!document.isObject())
        {
            fail(tr("qbutt-net returned an invalid control frame."));
            return;
        }
        handleResponse(document.object());
        if (m_process.state() == QProcess::NotRunning)
            return;
    }
}

void Net::PathManager::handleResponse(const QJsonObject &message)
{
    if ((message.value(u"v"_s).toInt() != 1) || (m_pendingId == 0)
        || (message.value(u"id"_s).toInt() != m_pendingId))
    {
        fail(tr("qbutt-net control version or request identifier does not match."));
        return;
    }
    const QJsonObject request = m_pendingRequest;
    m_pendingRequest = {};
    m_pendingId = 0;
    m_timeout.stop();
    const QString method = request.value(u"method"_s).toString();
    if (message.contains(u"error"_s))
    {
        // Do not expose arbitrary child strings: malformed subscriptions can
        // place credentials in parser/adapter errors.
        if (method == u"hello")
            fail(tr("The bundled qbutt-net rejected the protocol handshake."));
        else
            reportError(tr("qbutt-net rejected the request. Check the selected node and interface."));
        return;
    }
    if (!message.value(u"result"_s).isObject())
    {
        fail(tr("qbutt-net returned a missing control result."));
        return;
    }

    const QJsonObject result = message.value(u"result"_s).toObject();
    if (method == u"hello")
    {
        if ((result.value(u"protocol"_s).toInt() != 1)
            || (result.value(u"name"_s).toString() != u"qbutt-net")
            || (result.value(u"maxFrameBytes"_s).toInt() != MAX_FRAME_BYTES))
        {
            fail(tr("The bundled qbutt-net is incompatible with this application."));
            return;
        }
        QJsonObject queued = std::move(m_queuedRequest);
        m_queuedRequest = {};
        send(std::move(queued));
        return;
    }
    if (method == u"list")
    {
        const QJsonArray entries = result.value(u"proxies"_s).toArray();
        if (!result.value(u"proxies"_s).isArray() || (entries.size() > 1024))
        {
            fail(tr("qbutt-net returned an invalid proxy list."));
            return;
        }
        QJsonArray proxies;
        for (const QJsonValue &value : entries)
        {
            const QJsonObject entry = value.toObject();
            const QString name = entry.value(u"name"_s).toString();
            const QString type = entry.value(u"type"_s).toString();
            if (name.isEmpty() || type.isEmpty())
            {
                fail(tr("qbutt-net returned an invalid node description."));
                return;
            }
            // Only display-safe fields cross into UI/API diagnostics.
            proxies.append(QJsonObject {{u"name"_s, name}, {u"type"_s, type}});
        }
        m_storeConfigurationPath = request.value(u"configPath"_s).toString();
        m_proxies = proxies;
        emit proxiesLoaded(proxies);
    }
    else if (method == u"open")
    {
        const int port = result.value(u"port"_s).toInt();
        const QString username = result.value(u"socksUsername"_s).toString();
        const QString password = result.value(u"socksPassword"_s).toString();
        if ((result.value(u"host"_s).toString() != u"127.0.0.1") || (port < 1) || (port > 65535)
            || username.isEmpty() || password.isEmpty() || (username.toUtf8().size() > 255)
            || (password.toUtf8().size() > 255) || (result.value(u"pathId"_s) != request.value(u"pathId"_s))
            || (result.value(u"generation"_s) != request.value(u"generation"_s))
            || (result.value(u"interfaceName"_s) != request.value(u"interfaceName"_s)))
        {
            fail(tr("qbutt-net returned an invalid authenticated loopback endpoint."));
            return;
        }
        const bool replacePrimary = !m_storeMixed && !isOpen();
        ActivePath path;
        path.endpoint.type = PeerRouteEndpoint::Type::Socks5;
        path.endpoint.pathId = request.value(u"pathId"_s).toString().toULongLong();
        path.endpoint.generation = static_cast<quint64>(request.value(u"generation"_s).toInteger());
        path.endpoint.port = static_cast<quint16>(port);
        path.endpoint.username = username;
        path.endpoint.password = password;
        path.edgeId = request.value(u"edgeId"_s).toString();
        path.proxyName = request.value(u"proxyName"_s).toString();
        path.interfaceName = request.value(u"interfaceName"_s).toString();
        path.capabilities = result.value(u"capabilities"_s).toObject();
        auto existing = std::ranges::find(m_paths, path.endpoint.pathId,
            [](const ActivePath &entry) { return entry.endpoint.pathId; });
        if (existing != m_paths.end())
        {
            const qsizetype index = std::distance(m_paths.begin(), existing);
            *existing = std::move(path);
            if (replacePrimary)
                m_paths.move(index, 0);
        }
        else if (!replacePrimary)
        {
            m_paths.append(std::move(path));
        }
        else
        {
            m_paths.prepend(std::move(path));
        }

        const PeerRouteEndpoint &primary = m_paths.front().endpoint;
        ProxyConfiguration proxy = blockedRuntimeProxy();
        proxy.port = primary.port;
        proxy.username = primary.username;
        proxy.password = primary.password;
        if (!ProxyConfigurationManager::instance()->setRuntimeProxy(proxy))
        {
            fail(tr("Unable to save the pinned startup policy. The path remains blocked."));
            return;
        }
        applyRoutes();
        if (!m_storeMixed)
        {
            for (qsizetype index = 1; index < m_paths.size(); ++index)
            {
                const auto &endpoint = m_paths.at(index).endpoint;
                BitTorrent::Session::instance()->invalidatePeerRoute(endpoint.pathId, endpoint.generation);
            }
        }
        m_storeConfigurationPath = request.value(u"configPath"_s).toString();
        m_storeProxyName = request.value(u"proxyName"_s).toString();
        m_storeInterfaceName = request.value(u"interfaceName"_s).toString();
        m_status = tr("TCP endpoint ready: %1\n"
            "Egress, UDP, public inbound and throughput: unknown (not probed).")
            .arg(request.value(u"proxyName"_s).toString());
    }
    emit changed();
}

void Net::PathManager::fail(const QString &message)
{
    auto *proxyManager = ProxyConfigurationManager::instance();
    if (proxyManager->hasRuntimeProxy())
        proxyManager->setRuntimeProxy(blockedRuntimeProxy());
    shutdown();
    reportError(message);
}

void Net::PathManager::reportError(const QString &message)
{
    m_status = message;
    emit changed();
}

void Net::PathManager::stopPath(const QString &pathId)
{
    if (isBusy())
        return;
    if (!pathId.isEmpty())
    {
        if ((pathId == u"1") || (pathId == u"2"))
        {
            setPolicy(m_storeMixed ? u"mixed"_s : u"pinned"_s);
            return;
        }
        auto path = std::ranges::find(m_paths, pathId.toULongLong(),
            [](const ActivePath &entry) { return entry.endpoint.pathId; });
        if ((path == m_paths.end()) || (path->endpoint.port == 0))
        {
            reportError(tr("The selected path is not active."));
            return;
        }
        const quint64 generation = path->endpoint.generation;
        path->endpoint.type = PeerRouteEndpoint::Type::Blocked;
        path->endpoint.port = 0;
        path->endpoint.username.clear();
        path->endpoint.password.clear();
        applyRoutes();
        BitTorrent::Session::instance()->invalidatePeerRoute(path->endpoint.pathId, generation);
        if (path == m_paths.begin())
            ProxyConfigurationManager::instance()->setRuntimeProxy(blockedRuntimeProxy());
        m_status = tr("Path disconnected. Its existing peer connections have been closed.");
        request({{u"method"_s, u"close"_s}, {u"pathId"_s, pathId},
            {u"generation"_s, static_cast<qint64>(generation)}});
        return;
    }
    const QList<PeerRouteEndpoint> nativeEndpoints = std::move(m_nativeEndpoints);
    m_nativeEndpoints.clear();
    if (!nativeEndpoints.isEmpty())
        applyRoutes();
    for (const PeerRouteEndpoint &endpoint : nativeEndpoints)
        BitTorrent::Session::instance()->invalidatePeerRoute(endpoint.pathId, endpoint.generation);
    auto *proxyManager = ProxyConfigurationManager::instance();
    if (proxyManager->hasRuntimeProxy())
        proxyManager->setRuntimeProxy(blockedRuntimeProxy());
    if (!shutdown())
    {
        reportError(tr("The qbutt-net process has not stopped yet."));
        return;
    }
    m_status = ProxyConfigurationManager::instance()->hasRuntimeProxy()
        ? tr("Pinned path stopped. Automatic Native fallback is disabled.")
        : tr("Native / saved connection settings. No qbutt-net path is active.");
    emit changed();
}

bool Net::PathManager::shutdown()
{
    auto *session = BitTorrent::Session::instance();
    for (ActivePath &path : m_paths)
    {
        path.endpoint.type = PeerRouteEndpoint::Type::Blocked;
        path.endpoint.port = 0;
        path.endpoint.username.clear();
        path.endpoint.password.clear();
    }
    if (ProxyConfigurationManager::instance()->hasRuntimeProxy())
        applyRoutes();
    for (const ActivePath &path : m_paths)
        session->invalidatePeerRoute(path.endpoint.pathId, path.endpoint.generation);
    m_timeout.stop();
    if (m_subscriptionReply)
    {
        m_subscriptionReply->disconnect(this);
        m_subscriptionReply->abort();
        m_subscriptionReply->deleteLater();
        m_subscriptionReply = nullptr;
    }
    // Stop callbacks before waiting; this method is also used during teardown.
    const QSignalBlocker blocker(&m_process);
    m_process.closeWriteChannel();
    if ((m_process.state() != QProcess::NotRunning) && !m_process.waitForFinished(500))
    {
        m_process.kill();
        m_process.waitForFinished(500);
    }
    m_output.clear();
    m_queuedRequest = {};
    m_pendingRequest = {};
    m_pendingId = 0;
    return m_process.state() == QProcess::NotRunning;
}
