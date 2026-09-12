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
    constexpr int PROTOCOL_VERSION = 2;
    constexpr qint64 MAX_CONTROL_ID = 9007199254740991;

    bool validDnsFamily(const QString &family)
    {
        return (family == u"ipv4") || (family == u"ipv6") || (family == u"dual");
    }

    QString canonicalDnsServer(const QString &server)
    {
        const int separator = server.lastIndexOf(u':');
        if (separator < 1)
            return {};
        QString host = server.left(separator);
        if (host.startsWith(u'[') && host.endsWith(u']'))
            host = host.mid(1, host.size() - 2);
        else if (host.contains(u':'))
            return {};
        const QString portText = server.mid(separator + 1);
        bool valid = false;
        const quint16 port = portText.toUShort(&valid);
        const QHostAddress address(host);
        if (!valid || (port == 0) || portText.isEmpty()
            || !std::ranges::all_of(portText, [](const QChar c) { return (c >= u'0') && (c <= u'9'); })
            || address.isNull() || address.isMulticast() || !address.scopeId().isEmpty()
            || (address == QHostAddress::AnyIPv4) || (address == QHostAddress::AnyIPv6))
        {
            return {};
        }
        return ((address.protocol() == QAbstractSocket::IPv6Protocol)
            ? u"[%1]:%2"_s : u"%1:%2"_s).arg(address.toString()).arg(port);
    }
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
    , m_storePolicy {u"Network/Paths/Policy"_s}
    , m_storeNativeInterface {u"Network/Paths/NativeInterface"_s}
    , m_storeDnsServer {u"Network/Paths/DnsServer"_s}
    , m_storeBootstrapServer {u"Network/Paths/BootstrapServer"_s}
    , m_storeDnsFamily {u"Network/Paths/DnsFamily"_s}
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
        fail(m_pendingRequest.value(u"method"_s) == u"resolve"_s
            ? tr("qbutt-net did not complete DNS resolution within 8 seconds.")
            : tr("qbutt-net did not respond within 15 seconds."));
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
    {
        if (!applyRoutes())
            m_status = tr("Unable to restore the saved managed network policy.");
    }
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
            {u"dns"_s, path.dnsPolicy},
            {u"closedPayloadDownload"_s, path.closedPayloadDownload},
            {u"closedPayloadUpload"_s, path.closedPayloadUpload}});
    }
    for (const PeerRouteEndpoint &endpoint : m_nativeEndpoints)
    {
        paths.append(QJsonObject {{u"pathId"_s, QString::number(endpoint.pathId)},
            {u"generation"_s, static_cast<qint64>(endpoint.generation)}, {u"edgeId"_s, u"native"_s},
            {u"proxyName"_s, tr("Native")}, {u"interfaceName"_s, m_storeNativeInterface.get()},
            {u"open"_s, true}, {u"localAddress"_s, endpoint.localAddress}});
    }
    return {{u"v"_s, 1}, {u"busy"_s, isBusy()}, {u"open"_s, isOpen()},
        {u"pinned"_s, ProxyConfigurationManager::instance()->hasRuntimeProxy()},
        {u"status"_s, m_status}, {u"processId"_s, m_process.processId()},
        {u"mode"_s, m_storePolicy.get(u"pinned"_s)},
        {u"nativeInterface"_s, m_storeNativeInterface.get()},
        {u"paths"_s, paths},
        {u"peers"_s, includePeers ? BitTorrent::Session::instance()->peerRouteStatus() : QJsonArray {}},
        {u"generation"_s, m_generation}, {u"nodes"_s, m_proxies},
        {u"dns"_s, dnsPolicy()}, {u"resolution"_s, m_resolution},
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

QJsonObject Net::PathManager::dnsPolicy() const
{
    return {{u"server"_s, m_storeDnsServer.get(u"1.1.1.1:53"_s)},
        {u"bootstrapServer"_s, m_storeBootstrapServer.get(u"1.1.1.1:53"_s)},
        {u"family"_s, m_storeDnsFamily.get(u"dual"_s)}};
}

bool Net::PathManager::setDnsPolicy(const QString &server, const QString &bootstrapServer, const QString &family)
{
    if (isBusy())
        return false;
    const QString dns = canonicalDnsServer(server.trimmed());
    const QString bootstrap = canonicalDnsServer(bootstrapServer.trimmed());
    if (dns.isEmpty() || bootstrap.isEmpty() || !validDnsFamily(family))
    {
        reportError(tr("DNS servers must be numeric IP:port addresses, with IPv4, IPv6 or both selected."));
        return false;
    }
    const QJsonObject previous = dnsPolicy();
    m_storeDnsServer = dns;
    m_storeBootstrapServer = bootstrap;
    m_storeDnsFamily = family;
    if (!SettingsStorage::instance()->save())
    {
        m_storeDnsServer = previous.value(u"server"_s).toString();
        m_storeBootstrapServer = previous.value(u"bootstrapServer"_s).toString();
        m_storeDnsFamily = previous.value(u"family"_s).toString();
        reportError(tr("Unable to save DNS settings."));
        return false;
    }
    m_status = tr("DNS settings saved. They apply when connecting a node; existing paths keep their settings.");
    emit dnsPolicyChanged();
    emit changed();
    return true;
}

qint64 Net::PathManager::resolveHost(const QString &pathId, const quint64 generation,
    const QString &host, const QString &family)
{
    if (isBusy())
        return 0;
    const auto path = std::ranges::find(m_paths, pathId.toULongLong(),
        [](const ActivePath &entry) { return entry.endpoint.pathId; });
    if ((m_process.state() != QProcess::Running) || (path == m_paths.end()) || (path->endpoint.port == 0)
        || (QString::number(path->endpoint.pathId) != pathId) || (path->endpoint.generation != generation)
        || host.isEmpty() || (host.toUtf8().size() > 1024) || !validDnsFamily(family))
    {
        reportError(tr("Choose an active path generation, a hostname and a valid address family."));
        return 0;
    }
    m_resolution = {{u"requestId"_s, m_nextId + 1}, {u"pathId"_s, pathId},
        {u"generation"_s, static_cast<qint64>(generation)}, {u"family"_s, family}, {u"state"_s, u"pending"_s}};
    const qint64 requestId = m_nextId + 1;
    request({{u"method"_s, u"resolve"_s}, {u"pathId"_s, pathId},
        {u"generation"_s, static_cast<qint64>(generation)}, {u"host"_s, host}, {u"family"_s, family}});
    return requestId;
}

void Net::PathManager::finishResolution(const QList<QHostAddress> &addresses, const QString &errorCode)
{
    if (m_resolution.value(u"state"_s) != u"pending"_s)
        return;
    QJsonArray values;
    for (const QHostAddress &address : addresses)
        values.append(address.toString());
    m_resolution.insert(u"state"_s, errorCode.isEmpty() ? u"complete"_s : u"failed"_s);
    m_resolution.insert(u"addresses"_s, values);
    if (!errorCode.isEmpty())
        m_resolution.insert(u"errorCode"_s, errorCode);
    const qint64 requestId = m_resolution.value(u"requestId"_s).toInteger();
    const quint64 pathId = m_resolution.value(u"pathId"_s).toString().toULongLong();
    const quint64 generation = static_cast<quint64>(m_resolution.value(u"generation"_s).toInteger());
    // Consumers may start another lookup. Complete route/control transitions
    // before delivering the immutable result back to their owner.
    QMetaObject::invokeMethod(this, [this, requestId, pathId, generation, addresses, errorCode]()
    {
        emit hostResolved(requestId, pathId, generation, addresses, errorCode);
    }, Qt::QueuedConnection);
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
    if (!session->isRestored())
    {
        reportError(tr("The torrent session is not ready for a network policy transition."));
        return;
    }
    if (proxyName.isEmpty() || interfaceName.isEmpty())
    {
        reportError(tr("Select a proxy node and choose its physical interface."));
        return;
    }
    const QJsonObject dns = dnsPolicy();
    if (canonicalDnsServer(dns.value(u"server"_s).toString()).isEmpty()
        || canonicalDnsServer(dns.value(u"bootstrapServer"_s).toString()).isEmpty()
        || !validDnsFamily(dns.value(u"family"_s).toString()))
    {
        reportError(tr("Correct the saved DNS settings before connecting a node."));
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
        {u"generation"_s, m_generation}, {u"interfaceName"_s, interfaceName},
        {u"edgeId"_s, selectedEdge}, {u"dns"_s, dns}});
}

bool Net::PathManager::setPolicy(const QString &mode, const QString &nativeInterface)
{
    if (isBusy())
        return false;
    if ((mode != u"mixed") && (mode != u"pinned") && (mode != u"tunnels"))
    {
        reportError(tr("Choose Pinned, Mixed or Tunnels only."));
        return false;
    }
    const bool mixed = (mode == u"mixed");
    if (mixed && nativeInterface.isEmpty())
    {
        reportError(tr("Choose a physical Native interface for Mixed mode."));
        return false;
    }
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
                endpoint.supportsIPv4 = !ipv6;
                endpoint.supportsIPv6 = ipv6;
                endpoint.supportsUdp = true;
                nativeEndpoints.append(std::move(endpoint));
                (ipv6 ? haveIPv6 : haveIPv4) = true;
            }
            break;
        }
        if (nativeEndpoints.isEmpty())
        {
            reportError(tr("The selected physical Native interface has no usable address."));
            return false;
        }
    }
    const QString previous = m_storePolicy.get(u"pinned"_s);
    const QString previousInterface = m_storeNativeInterface;
    auto *proxyManager = ProxyConfigurationManager::instance();
    if (!proxyManager->hasRuntimeProxy())
    {
        if (!proxyManager->setRuntimeProxy(blockedRuntimeProxy()))
        {
            reportError(tr("Unable to save the managed startup policy."));
            return false;
        }
    }
    m_storePolicy = mode;
    m_storeNativeInterface = mixed ? nativeInterface : QString();
    if (((previous != m_storePolicy) || (previousInterface != m_storeNativeInterface))
        && !SettingsStorage::instance()->save())
    {
        m_storePolicy = previous;
        m_storeNativeInterface = previousInterface;
        reportError(tr("Unable to save the network policy."));
        return false;
    }
    const QList<PeerRouteEndpoint> oldNativeEndpoints = std::move(m_nativeEndpoints);
    m_nativeEndpoints = std::move(nativeEndpoints);
    ++m_nativeGeneration;
    if (!applyRoutes())
    {
        m_nativeEndpoints = oldNativeEndpoints;
        m_storePolicy = previous;
        m_storeNativeInterface = previousInterface;
        SettingsStorage::instance()->save();
        applyRoutes();
        reportError(tr("Unable to apply the network policy."));
        return false;
    }
    for (const PeerRouteEndpoint &endpoint : oldNativeEndpoints)
        BitTorrent::Session::instance()->invalidateNetworkRoute(endpoint.pathId, endpoint.generation);
    if (mode == u"mixed")
        m_status = tr("Mixed: public torrents use selected edges and the chosen Native interface. Private torrents stay pinned.");
    else if (mode == u"tunnels")
        m_status = tr("Tunnels only: public torrents use selected remote edges. Private torrents stay pinned.");
    else
        m_status = tr("Pinned: torrent traffic uses the first selected edge.");
    emit changed();
    return true;
}

bool Net::PathManager::applyRoutes()
{
    QList<PeerRouteEndpoint> endpoints;
    for (const ActivePath &path : m_paths)
        endpoints.append(path.endpoint);
    // An unavailable pinned edge must never turn into Native for private torrents.
    if (endpoints.isEmpty())
        endpoints.append(PeerRouteEndpoint {});
    const QString mode = m_storePolicy.get(u"pinned"_s);
    if (mode == u"mixed")
        endpoints.append(m_nativeEndpoints);
    const RoutePolicy policy = (mode == u"mixed") ? RoutePolicy::Mixed
        : ((mode == u"tunnels") ? RoutePolicy::TunnelsOnly : RoutePolicy::Pinned);
    return BitTorrent::Session::instance()->setNetworkRoutes(endpoints, policy);
}

void Net::PathManager::useNative()
{
    if (isBusy())
        return;
    // Terminate accepted sockets before restoring saved connection settings.
    if (!shutdown())
    {
        reportError(tr("The previous qbutt-net process has not stopped. Native was not enabled."));
        return;
    }
    if (!BitTorrent::Session::instance()->resetNetworkRoutes())
    {
        reportError(tr("Unable to restore the default torrent network routes."));
        return;
    }
    if (!ProxyConfigurationManager::instance()->clearRuntimeProxy())
    {
        applyRoutes();
        fail(tr("Unable to save the Native startup policy. The pinned path remains blocked."));
        return;
    }
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
    if (m_nextId >= MAX_CONTROL_ID)
    {
        fail(tr("The qbutt-net request identifier limit was reached. Restart qbutt."));
        return;
    }
    m_pendingId = ++m_nextId;
    message.insert(u"v"_s, PROTOCOL_VERSION);
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
    m_timeout.start(message.value(u"method"_s) == u"resolve"_s ? 8000 : 15000);
    emit changed();
}

void Net::PathManager::sendQueuedRequest()
{
    if ((m_pendingId != 0) || m_queuedRequest.isEmpty())
        return;
    QJsonObject queued = std::move(m_queuedRequest);
    m_queuedRequest = {};
    send(std::move(queued));
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
    if ((message.value(u"v"_s) != QJsonValue(PROTOCOL_VERSION)) || (m_pendingId == 0)
        || (message.value(u"id"_s) != QJsonValue(m_pendingId)))
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
        if ((method == u"hello") || !message.value(u"error"_s).isObject() || message.contains(u"result"_s))
            fail(tr("The bundled qbutt-net rejected the protocol handshake."));
        else
        {
            if (method == u"resolve")
                finishResolution({}, u"path_dns_failed"_s);
            sendQueuedRequest();
            reportError(tr("qbutt-net rejected the request. Check the selected node and interface."));
        }
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
        if ((result.value(u"protocol"_s) != QJsonValue(PROTOCOL_VERSION))
            || (result.value(u"name"_s).toString() != u"qbutt-net")
            || (result.value(u"maxFrameBytes"_s).toInt() != MAX_FRAME_BYTES))
        {
            fail(tr("The bundled qbutt-net is incompatible with this application."));
            return;
        }
        sendQueuedRequest();
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
        const QJsonObject capabilities = result.value(u"capabilities"_s).toObject();
        const QString udp = capabilities.value(u"udp"_s).toString();
        if ((result.value(u"host"_s).toString() != u"127.0.0.1") || (port < 1) || (port > 65535)
            || username.isEmpty() || password.isEmpty() || (username.toUtf8().size() > 255)
            || (password.toUtf8().size() > 255) || (result.value(u"pathId"_s) != request.value(u"pathId"_s))
            || (result.value(u"generation"_s) != request.value(u"generation"_s))
            || (result.value(u"interfaceName"_s) != request.value(u"interfaceName"_s))
            || (capabilities.value(u"tcp"_s) != u"supported"_s)
            || ((udp != u"source-supported") && (udp != u"source-unsupported"))
            || (capabilities.value(u"dns"_s) != u"path-tcp"_s)
            || (capabilities.value(u"publicTcp"_s) != u"unknown"_s)
            || (capabilities.value(u"publicUdp"_s) != u"unknown"_s)
            || (capabilities.value(u"measurement"_s) != u"not-probed"_s))
        {
            fail(tr("qbutt-net returned an invalid authenticated loopback endpoint."));
            return;
        }
        const bool replacePrimary = (m_storePolicy.get(u"pinned"_s) == u"pinned") && !isOpen();
        ActivePath path;
        path.endpoint.type = PeerRouteEndpoint::Type::Socks5;
        path.endpoint.pathId = request.value(u"pathId"_s).toString().toULongLong();
        path.endpoint.generation = static_cast<quint64>(request.value(u"generation"_s).toInteger());
        path.endpoint.port = static_cast<quint16>(port);
        path.endpoint.username = username;
        path.endpoint.password = password;
        const QString family = request.value(u"dns"_s).toObject().value(u"family"_s).toString();
        path.endpoint.supportsIPv4 = family != u"ipv6";
        path.endpoint.supportsIPv6 = family != u"ipv4";
        path.endpoint.supportsUdp = udp == u"source-supported";
        path.edgeId = request.value(u"edgeId"_s).toString();
        path.proxyName = request.value(u"proxyName"_s).toString();
        path.interfaceName = request.value(u"interfaceName"_s).toString();
        path.capabilities = {{u"tcp"_s, u"supported"_s}, {u"udp"_s, udp}, {u"dns"_s, u"path-tcp"_s},
            {u"publicTcp"_s, u"unknown"_s}, {u"publicUdp"_s, u"unknown"_s}, {u"measurement"_s, u"not-probed"_s}};
        path.dnsPolicy = request.value(u"dns"_s).toObject();
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
        if (!applyRoutes())
        {
            fail(tr("Unable to apply the network routes returned by qbutt-net."));
            return;
        }
        m_storeConfigurationPath = request.value(u"configPath"_s).toString();
        m_storeProxyName = request.value(u"proxyName"_s).toString();
        m_storeInterfaceName = request.value(u"interfaceName"_s).toString();
        m_status = tr("TCP endpoint ready: %1\n"
            "Egress, UDP, public inbound and throughput: unknown (not probed).")
            .arg(request.value(u"proxyName"_s).toString());
    }
    else if (method == u"resolve")
    {
        const auto path = std::ranges::find(m_paths, request.value(u"pathId"_s).toString().toULongLong(),
            [](const ActivePath &entry) { return entry.endpoint.pathId; });
        if ((path == m_paths.end()) || (path->endpoint.port == 0)
            || (path->endpoint.generation != static_cast<quint64>(request.value(u"generation"_s).toInteger())))
        {
            finishResolution({}, u"path_stopped"_s);
        }
        else
        {
            const QJsonArray values = result.value(u"addresses"_s).toArray();
            if (!result.value(u"addresses"_s).isArray() || values.isEmpty() || (values.size() > 64))
            {
                fail(tr("qbutt-net returned an invalid DNS address list."));
                return;
            }
            QList<QHostAddress> addresses;
            const QString family = request.value(u"family"_s).toString();
            const QString policyFamily = path->dnsPolicy.value(u"family"_s).toString();
            for (const QJsonValue &value : values)
            {
                const QHostAddress address(value.toString());
                const bool ipv4 = address.protocol() == QAbstractSocket::IPv4Protocol;
                bool convertibleIPv4 = false;
                address.toIPv4Address(&convertibleIPv4);
                if (!value.isString() || address.isNull() || !address.scopeId().isEmpty()
                    || (!ipv4 && convertibleIPv4)
                    || address.isMulticast() || (address == QHostAddress::AnyIPv4) || (address == QHostAddress::AnyIPv6)
                    || (((family == u"ipv4") || (policyFamily == u"ipv4")) && !ipv4)
                    || (((family == u"ipv6") || (policyFamily == u"ipv6")) && ipv4)
                    || addresses.contains(address))
                {
                    fail(tr("qbutt-net returned an invalid DNS address or family."));
                    return;
                }
                addresses.append(address);
            }
            finishResolution(addresses);
        }
    }
    sendQueuedRequest();
    emit changed();
}

void Net::PathManager::fail(const QString &message)
{
    finishResolution({}, u"transport_failed"_s);
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
    const bool resolving = (m_pendingRequest.value(u"method"_s) == u"resolve"_s);
    if (!pathId.isEmpty() && isBusy() && (!resolving || !m_queuedRequest.isEmpty()))
        return;
    if (!pathId.isEmpty())
    {
        if ((pathId == u"1") || (pathId == u"2"))
        {
            setPolicy(u"tunnels"_s);
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
        if (!applyRoutes())
        {
            fail(tr("Unable to revoke the selected network path."));
            return;
        }
        BitTorrent::Session::instance()->invalidateNetworkRoute(path->endpoint.pathId, generation);
        if (path == m_paths.begin())
            ProxyConfigurationManager::instance()->setRuntimeProxy(blockedRuntimeProxy());
        m_status = tr("Path disconnected. Its existing peer connections have been closed.");
        QJsonObject close {{u"method"_s, u"close"_s}, {u"pathId"_s, pathId},
            {u"generation"_s, static_cast<qint64>(generation)}};
        if (resolving)
        {
            m_queuedRequest = std::move(close);
            if ((m_resolution.value(u"pathId"_s).toString() == pathId)
                && (static_cast<quint64>(m_resolution.value(u"generation"_s).toInteger()) == generation))
            {
                finishResolution({}, u"path_stopped"_s);
            }
            emit changed();
        }
        else
        {
            request(std::move(close));
        }
        return;
    }
    const QList<PeerRouteEndpoint> nativeEndpoints = std::move(m_nativeEndpoints);
    m_nativeEndpoints.clear();
    if (!nativeEndpoints.isEmpty())
        applyRoutes();
    for (const PeerRouteEndpoint &endpoint : nativeEndpoints)
        BitTorrent::Session::instance()->invalidateNetworkRoute(endpoint.pathId, endpoint.generation);
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
    finishResolution({}, u"path_stopped"_s);
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
        session->invalidateNetworkRoute(path.endpoint.pathId, path.endpoint.generation);
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
