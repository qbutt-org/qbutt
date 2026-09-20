/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "pathmanager.h"

#include <algorithm>
#include <cmath>

#ifdef Q_OS_WIN
#include <winsock2.h>
#include <ws2ipdef.h>
#include <iphlpapi.h>
#endif

#include <QCoreApplication>
#include <QDateTime>
#include <QDir>
#include <QFileInfo>
#include <QJsonDocument>
#include <QNetworkAddressEntry>
#include <QNetworkInterface>
#include <QNetworkProxy>
#include <QNetworkReply>
#include <QNetworkRequest>
#include <QRegularExpression>
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
    constexpr int PROTOCOL_VERSION = 5;
    constexpr auto QBT_NET_UPSTREAM_REVISION = u"d3ec342d441b086ec4318332f59dd05d8a2b5697";
    constexpr qint64 MAX_CONTROL_ID = 9007199254740991;
    constexpr qint64 GATEWAY_TTL_SECONDS = 90;
    constexpr qint64 GATEWAY_RENEWAL_HEADROOM_MS = 70000;

    bool isSafeUnsignedInteger(const QJsonValue &value)
    {
        if (!value.isDouble())
            return false;
        const double number = value.toDouble();
        return std::isfinite(number) && (number >= 0) && (number <= MAX_CONTROL_ID)
            && (std::trunc(number) == number);
    }

    std::optional<QJsonObject> wireCounters(const QJsonValue &value)
    {
        static const QStringList fields {
            u"relayDownloadBytes"_s, u"relayUploadBytes"_s,
            u"carrierDownloadBytes"_s, u"carrierUploadBytes"_s,
            u"carrierDownloadPackets"_s, u"carrierUploadPackets"_s,
            u"relayDownloadCopies"_s};
        if (!value.isObject())
            return {};
        const QJsonObject input = value.toObject();
        if (input.size() != fields.size())
            return {};
        QJsonObject output;
        for (const QString &field : fields)
        {
            if (!isSafeUnsignedInteger(input.value(field)))
                return {};
            output.insert(field, input.value(field));
        }
        return output;
    }

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

    struct NumericEndpoint
    {
        QHostAddress address;
        quint16 port = 0;
        QString text;
    };

    std::optional<NumericEndpoint> numericEndpoint(const QString &text)
    {
        const int separator = text.lastIndexOf(u':');
        if (separator < 1)
            return {};
        QString host = text.left(separator);
        if (host.startsWith(u'[') && host.endsWith(u']'))
            host = host.mid(1, host.size() - 2);
        else if (host.contains(u':'))
            return {};
        bool validPort = false;
        const QString portText = text.mid(separator + 1);
        const quint16 port = portText.toUShort(&validPort);
        const QHostAddress address(host);
        bool mappedIPv4 = false;
        address.toIPv4Address(&mappedIPv4);
        if (!validPort || (port == 0) || address.isNull() || address.isMulticast()
            || ((address.protocol() == QAbstractSocket::IPv6Protocol) && mappedIPv4)
            || !address.scopeId().isEmpty() || (address == QHostAddress::AnyIPv4)
            || (address == QHostAddress::AnyIPv6))
        {
            return {};
        }
        return NumericEndpoint {address, port, ((address.protocol() == QAbstractSocket::IPv6Protocol)
            ? u"[%1]:%2"_s : u"%1:%2"_s).arg(address.toString()).arg(port)};
    }

    QString canonicalGatewayAddress(const QString &text)
    {
        const QUrl url(u"tcp://"_s + text.trimmed(), QUrl::StrictMode);
        const int port = url.port();
        if (!url.isValid() || url.host().isEmpty() || !url.userInfo().isEmpty()
            || (!url.path().isEmpty() && (url.path() != u"/")) || url.hasQuery() || url.hasFragment()
            || (port < 1) || (port > 65535) || (url.host().toUtf8().size() > 253))
        {
            return {};
        }
        const QString host = url.host();
        return host.contains(u':') ? u"[%1]:%2"_s.arg(host).arg(port) : u"%1:%2"_s.arg(host).arg(port);
    }

    bool readableAbsoluteFile(const QString &path)
    {
        const QFileInfo info(path);
        const QString absolutePath = info.absoluteFilePath();
        return info.isAbsolute() && (absolutePath.toUtf8().size() <= 1024)
            && !QDir::toNativeSeparators(absolutePath).startsWith(u"\\\\")
            && info.isFile() && info.isReadable() && (info.size() <= (256 * 1024));
    }

    bool validGatewayServerName(QString name)
    {
        const QHostAddress address(name);
        if (!address.isNull())
        {
            return !address.isMulticast() && address.scopeId().isEmpty()
                && (address != QHostAddress::AnyIPv4) && (address != QHostAddress::AnyIPv6);
        }
        if (name.endsWith(u'.'))
            name.chop(1);
        const QByteArray ace = QUrl::toAce(name);
        if (ace.isEmpty() || (ace.size() > 253))
            return false;
        return std::ranges::all_of(ace.split('.'), [](const QByteArray &label)
        {
            if (label.isEmpty() || (label.size() > 63) || (label.front() == '-') || (label.back() == '-'))
                return false;
            return std::ranges::all_of(label, [](const char c)
            {
                return ((c >= 'a') && (c <= 'z')) || ((c >= 'A') && (c <= 'Z'))
                    || ((c >= '0') && (c <= '9')) || (c == '-');
            });
        });
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
    , m_storeGatewayControlAddress {u"Network/Paths/Gateway/ControlAddress"_s}
    , m_storeGatewayDatagramAddress {u"Network/Paths/Gateway/DatagramAddress"_s}
    , m_storeGatewayServerName {u"Network/Paths/Gateway/ServerName"_s}
    , m_storeGatewayCaPath {u"Network/Paths/Gateway/CaPath"_s}
    , m_storeGatewayCertificatePath {u"Network/Paths/Gateway/CertificatePath"_s}
    , m_storeGatewayPrivateKeyPath {u"Network/Paths/Gateway/PrivateKeyPath"_s}
    , m_storeGatewayPort {u"Network/Paths/Gateway/Port"_s}
    , m_storeGatewayTcp {u"Network/Paths/Gateway/Tcp"_s}
    , m_storeGatewayUdp {u"Network/Paths/Gateway/Udp"_s}
{
    m_timeout.setSingleShot(true);
    m_timeout.setInterval(15000);
    m_gatewayRenewal.setSingleShot(true);
    m_statusRefresh.setInterval(1000);
    // Child diagnostics are intentionally discarded. Its versioned control
    // responses contain safe errors; arbitrary transport logs may hold secrets.
    m_process.setStandardErrorFile(QProcess::nullDevice());
    // Subscription retrieval is an explicit control-network operation. It is
    // independent from the selected torrent path, including a failed path.
    m_network.setProxy(QNetworkProxy::NoProxy);
    connect(&m_timeout, &QTimer::timeout, this, [this]()
    {
        fail(m_pendingRequest.value(u"method"_s).toString().startsWith(u"resolve")
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
    connect(&m_gatewayRenewal, &QTimer::timeout, this, [this]()
    {
        const qint64 now = QDateTime::currentMSecsSinceEpoch();
        if (std::ranges::any_of(m_paths, [now](const ActivePath &path)
            { return path.publicLease && (path.publicLease->expiresUnixMilli <= now); }))
        {
            if (!beginPathRollover(activePathRollover(),
                tr("A public gateway lease expired. Reconnecting paths with new generations.")))
            {
                fail(tr("An expired public gateway generation could not be retired safely."));
            }
            return;
        }
        ActivePath *renew = nullptr;
        for (ActivePath &path : m_paths)
        {
            if (!path.publicLease)
                continue;
            if (!renew || (path.publicLease->expiresUnixMilli < renew->publicLease->expiresUnixMilli))
                renew = &path;
        }
        if (renew && !controlBusy())
        {
            request({{u"method"_s, u"gateway.renew"_s},
                {u"pathId"_s, QString::number(renew->endpoint.pathId)},
                {u"generation"_s, static_cast<qint64>(renew->endpoint.generation)}});
        }
        else if (renew)
        {
            m_gatewayRenewal.start(1000);
        }
    });
    connect(&m_statusRefresh, &QTimer::timeout, this, [this]()
    {
        if ((m_process.state() == QProcess::Running) && (m_pendingId == 0) && !controlBusy()
            && std::ranges::any_of(m_paths, [](const ActivePath &path) { return path.endpoint.port > 0; }))
        {
            request({{u"method"_s, u"status"_s}});
        }
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
    connect(BitTorrent::Session::instance(), &BitTorrent::Session::udpRouteReady,
        this, &PathManager::queueDhtBootstrap);
    connect(this, &PathManager::changed, this, &PathManager::processDhtBootstrap, Qt::QueuedConnection);
    connect(BitTorrent::Session::instance(), &BitTorrent::Session::dhtSettingsChanged, this, [this]()
    {
        m_dhtBootstrap.clear();
        m_bootstrapRequestId = 0;
        const auto queue = [this](const PeerRouteEndpoint &endpoint)
        {
            if (endpoint.supportsUdp)
            {
                if (endpoint.supportsIPv4)
                    queueDhtBootstrap(endpoint.pathId, endpoint.generation, false);
                if (endpoint.supportsIPv6)
                    queueDhtBootstrap(endpoint.pathId, endpoint.generation, true);
            }
        };
        for (const ActivePath &path : m_paths)
            queue(path.endpoint);
        for (const PeerRouteEndpoint &endpoint : m_nativeEndpoints)
            queue(endpoint);
    });
    connect(this, &PathManager::hostResolved, this, [this](const qint64 requestId, const quint64 pathId,
        const quint64 generation, const QList<QHostAddress> &addresses, const QString &)
    {
        if ((requestId != m_bootstrapRequestId) || m_dhtBootstrap.isEmpty())
            return;
        m_bootstrapRequestId = 0;
        const DhtBootstrap &bootstrap = m_dhtBootstrap.front();
        if ((bootstrap.pathId != pathId) || (bootstrap.generation != generation))
            return;
        for (const QHostAddress &address : addresses)
            BitTorrent::Session::instance()->addDHTRouteNode(pathId, generation, address, bootstrap.port);
        processDhtBootstrap();
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
    return controlBusy();
}

bool Net::PathManager::controlBusy() const
{
    const bool foregroundRequest = (m_pendingId != 0)
        && (m_pendingRequest.value(u"method"_s) != u"status"_s);
    return foregroundRequest || !m_requestQueue.isEmpty() || m_subscriptionReply
        || m_rolloverOpening || !m_pathRollover.isEmpty();
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
        QJsonObject gateway {{u"state"_s, u"outgoing-only"_s},
            {u"tcp"_s, false}, {u"udp"_s, false}};
        if (path.publicLease)
        {
            gateway = {{u"state"_s, u"leased"_s},
                {u"publicEndpoint"_s, path.publicLease->publicEndpoint},
                {u"family"_s, path.publicLease->family},
                {u"tcp"_s, path.publicLease->tcp}, {u"udp"_s, path.publicLease->udp},
                {u"expiresUnixMilli"_s, path.publicLease->expiresUnixMilli}};
        }
        QJsonObject data {{u"pathId"_s, QString::number(path.endpoint.pathId)},
            {u"generation"_s, static_cast<qint64>(path.endpoint.generation)},
            {u"edgeId"_s, path.edgeId}, {u"proxyName"_s, path.proxyName},
            {u"open"_s, path.endpoint.port > 0},
            {u"interfaceName"_s, path.interfaceName}, {u"capabilities"_s, path.capabilities},
            {u"dns"_s, path.dnsPolicy}, {u"gateway"_s, gateway},
            {u"closedPayloadDownload"_s, path.closedPayloadDownload},
            {u"closedPayloadUpload"_s, path.closedPayloadUpload}};
        if (!path.wire.isEmpty())
            data.insert(u"wire"_s, path.wire);
        paths.append(data);
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

QJsonObject Net::PathManager::gatewayConfiguration() const
{
    return {{u"controlAddress"_s, m_storeGatewayControlAddress.get()},
        {u"datagramAddress"_s, m_storeGatewayDatagramAddress.get()},
        {u"serverName"_s, m_storeGatewayServerName.get()},
        {u"caPath"_s, m_storeGatewayCaPath.get()},
        {u"certificatePath"_s, m_storeGatewayCertificatePath.get()},
        {u"privateKeyPath"_s, m_storeGatewayPrivateKeyPath.get()},
        {u"port"_s, m_storeGatewayPort.get()}, {u"tcp"_s, m_storeGatewayTcp.get()},
        {u"udp"_s, m_storeGatewayUdp.get()}};
}

bool Net::PathManager::setGatewayConfiguration(const QJsonObject &configuration)
{
    if (controlBusy())
        return false;
    const bool tcp = configuration.value(u"tcp"_s).toBool();
    const bool udp = configuration.value(u"udp"_s).toBool();
    const bool enabled = tcp || udp;
    const QString control = canonicalGatewayAddress(configuration.value(u"controlAddress"_s).toString());
    const QString datagram = canonicalGatewayAddress(configuration.value(u"datagramAddress"_s).toString());
    const QString serverName = configuration.value(u"serverName"_s).toString().trimmed();
    const auto absolutePath = [](const QString &path)
    {
        return path.trimmed().isEmpty() ? QString() : QFileInfo(path.trimmed()).absoluteFilePath();
    };
    const QString caPath = absolutePath(configuration.value(u"caPath"_s).toString());
    const QString certificatePath = absolutePath(configuration.value(u"certificatePath"_s).toString());
    const QString privateKeyPath = absolutePath(configuration.value(u"privateKeyPath"_s).toString());
    const int port = configuration.value(u"port"_s).toInt(-1);
    if ((port < 0) || (port > 65535) || (enabled && (control.isEmpty() || !validGatewayServerName(serverName)
        || !readableAbsoluteFile(caPath) || !readableAbsoluteFile(certificatePath)
        || !readableAbsoluteFile(privateKeyPath) || (udp && datagram.isEmpty()))))
    {
        reportError(tr("Enter valid gateway addresses, TLS name, readable certificate files and a port from 0 to 65535."));
        return false;
    }

    const QJsonObject normalized {{u"controlAddress"_s, enabled
            ? control : configuration.value(u"controlAddress"_s).toString().trimmed()},
        {u"datagramAddress"_s, enabled
            ? datagram : configuration.value(u"datagramAddress"_s).toString().trimmed()},
        {u"serverName"_s, serverName}, {u"caPath"_s, caPath},
        {u"certificatePath"_s, certificatePath}, {u"privateKeyPath"_s, privateKeyPath},
        {u"port"_s, port}, {u"tcp"_s, tcp}, {u"udp"_s, udp}};
    const QJsonObject previous = gatewayConfiguration();
    const bool previouslyEnabled = previous.value(u"tcp"_s).toBool() || previous.value(u"udp"_s).toBool();
    if (normalized == previous)
        return true;
    const auto store = [this](const QJsonObject &values)
    {
        m_storeGatewayControlAddress = values.value(u"controlAddress"_s).toString();
        m_storeGatewayDatagramAddress = values.value(u"datagramAddress"_s).toString();
        m_storeGatewayServerName = values.value(u"serverName"_s).toString();
        m_storeGatewayCaPath = values.value(u"caPath"_s).toString();
        m_storeGatewayCertificatePath = values.value(u"certificatePath"_s).toString();
        m_storeGatewayPrivateKeyPath = values.value(u"privateKeyPath"_s).toString();
        m_storeGatewayPort = values.value(u"port"_s).toInt();
        m_storeGatewayTcp = values.value(u"tcp"_s).toBool();
        m_storeGatewayUdp = values.value(u"udp"_s).toBool();
    };
    store(normalized);
    if (!SettingsStorage::instance()->save())
    {
        store(previous);
        reportError(tr("Unable to save public gateway settings."));
        return false;
    }

    if (!enabled && !previouslyEnabled)
    {
        m_status = tr("Public gateway disabled.");
        emit changed();
        return true;
    }

    QList<PathRollover> rollover = activePathRollover();
    if (rollover.isEmpty())
    {
        m_status = enabled ? tr("Public gateway settings saved. They apply when a path is connected.")
            : tr("Public gateway disabled.");
        emit changed();
        return true;
    }
    if (!beginPathRollover(std::move(rollover),
        tr("Reconnecting paths with new generations for the public gateway settings.")))
    {
        store(previous);
        if (!SettingsStorage::instance()->save())
        {
            fail(tr("The previous public gateway settings could not be restored. Network paths remain stopped."));
            return false;
        }
        reportError(tr("The active paths could not be stopped for a gateway generation change."));
        return false;
    }
    return true;
}

bool Net::PathManager::setDnsPolicy(const QString &server, const QString &bootstrapServer, const QString &family)
{
    if (controlBusy())
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

const Net::PeerRouteEndpoint *Net::PathManager::findEndpoint(const quint64 pathId, const quint64 generation) const
{
    for (const ActivePath &path : m_paths)
    {
        if ((path.endpoint.pathId == pathId) && (path.endpoint.generation == generation) && (path.endpoint.port > 0))
            return &path.endpoint;
    }
    for (const PeerRouteEndpoint &endpoint : m_nativeEndpoints)
    {
        if ((endpoint.pathId == pathId) && (endpoint.generation == generation))
            return &endpoint;
    }
    return nullptr;
}

void Net::PathManager::queueDhtBootstrap(const quint64 pathId, const quint64 generation, const bool ipv6)
{
    if (!BitTorrent::Session::instance()->isDHTEnabled() || !findEndpoint(pathId, generation))
        return;
    if (!std::ranges::any_of(m_dhtBootstrap, [=](const DhtBootstrap &entry)
        { return (entry.pathId == pathId) && (entry.generation == generation) && (entry.ipv6 == ipv6); }))
    {
        m_dhtBootstrap.append({pathId, generation, ipv6});
        QMetaObject::invokeMethod(this, &PathManager::processDhtBootstrap, Qt::QueuedConnection);
    }
}

void Net::PathManager::processDhtBootstrap()
{
    if (isBusy() || (m_bootstrapRequestId != 0) || !BitTorrent::Session::instance()->isDHTEnabled())
        return;
    const QStringList nodes = BitTorrent::Session::instance()->getDHTBootstrapNodes().split(u',', Qt::SkipEmptyParts);
    while (!m_dhtBootstrap.isEmpty())
    {
        DhtBootstrap &bootstrap = m_dhtBootstrap.front();
        if (!findEndpoint(bootstrap.pathId, bootstrap.generation) || (bootstrap.nodeIndex >= nodes.size()))
        {
            m_dhtBootstrap.removeFirst();
            continue;
        }
        const QUrl node(u"tcp://"_s + nodes[bootstrap.nodeIndex++].trimmed(), QUrl::StrictMode);
        if (!node.isValid() || node.host().isEmpty() || !node.userInfo().isEmpty()
            || !node.path().isEmpty() || node.hasQuery() || node.hasFragment() || (node.port() <= 0))
        {
            continue;
        }
        bootstrap.port = node.port();
        const QHostAddress numeric(node.host());
        if (!numeric.isNull())
        {
            if ((numeric.protocol() == QAbstractSocket::IPv6Protocol) == bootstrap.ipv6)
                BitTorrent::Session::instance()->addDHTRouteNode(bootstrap.pathId,
                    bootstrap.generation, numeric, bootstrap.port);
            continue;
        }
        m_bootstrapRequestId = resolveHost(QString::number(bootstrap.pathId), bootstrap.generation,
            node.host(), bootstrap.ipv6 ? u"ipv6"_s : u"ipv4"_s);
        if (m_bootstrapRequestId != 0)
            return;
    }
}

qint64 Net::PathManager::resolveHost(const QString &pathId, const quint64 generation,
    const QString &host, const QString &family)
{
    if (controlBusy())
        return 0;
    const PeerRouteEndpoint *endpoint = findEndpoint(pathId.toULongLong(), generation);
    if ((m_process.state() != QProcess::Running) || !endpoint
        || (QString::number(endpoint->pathId) != pathId)
        || host.isEmpty() || (host.toUtf8().size() > 1024) || !validDnsFamily(family))
    {
        reportError(tr("Choose an active path generation, a hostname and a valid address family."));
        return 0;
    }
    m_resolution = {{u"requestId"_s, m_nextId + 1}, {u"pathId"_s, pathId},
        {u"generation"_s, static_cast<qint64>(generation)}, {u"family"_s, family}, {u"state"_s, u"pending"_s}};
    const qint64 requestId = m_nextId + 1;
    QJsonObject message {{u"method"_s, u"resolve"_s}, {u"pathId"_s, pathId},
        {u"generation"_s, static_cast<qint64>(generation)}, {u"host"_s, host}, {u"family"_s, family}};
    if (endpoint->type == PeerRouteEndpoint::Type::Native)
    {
        message.insert(u"method"_s, u"resolveNative"_s);
        message.insert(u"interfaceName"_s, QNetworkInterface::interfaceFromIndex(endpoint->interfaceIndex).humanReadableName());
        message.insert(u"dns"_s, dnsPolicy());
    }
    request(std::move(message));
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
    if (controlBusy())
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
    const QString &interfaceName)
{
    if (controlBusy())
    {
        reportError(tr("A path operation is already running."));
        return;
    }

    auto *session = BitTorrent::Session::instance();
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
    // qbutt-net owns subscription parsing. Resolve identity again for each open,
    // including API calls that never loaded the UI's node list.
    m_status = tr("Checking the selected node's server identity.");
    request({{u"method"_s, u"list"_s}, {u"configPath"_s, QFileInfo(configPath).absoluteFilePath()},
        {u"proxyName"_s, proxyName}, {u"openInterfaceName"_s, interfaceName}});
}

void Net::PathManager::openIdentifiedPath(const QString &configPath, const QString &proxyName,
    const QString &interfaceName, const QString &configuredServerId)
{
    auto *session = BitTorrent::Session::instance();
    auto *proxyManager = ProxyConfigurationManager::instance();
    const QJsonObject dns = dnsPolicy();
    if (canonicalDnsServer(dns.value(u"server"_s).toString()).isEmpty()
        || canonicalDnsServer(dns.value(u"bootstrapServer"_s).toString()).isEmpty()
        || !validDnsFamily(dns.value(u"family"_s).toString()))
    {
        reportError(tr("Correct the saved DNS settings before connecting a node."));
        return;
    }
    quint64 pathId = 0;
    for (const ActivePath &path : m_paths)
    {
        if (path.edgeId == configuredServerId)
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
    const bool enableManagedRoutes = !proxyManager->hasRuntimeProxy();
    if (enableManagedRoutes && !proxyManager->setRuntimeProxy(blockedRuntimeProxy()))
    {
        reportError(tr("Unable to save the pinned startup policy. The path was not started."));
        return;
    }
    if (enableManagedRoutes && !applyRoutes())
    {
        const bool routesReset = session->resetNetworkRoutes();
        const bool proxyCleared = proxyManager->clearRuntimeProxy();
        reportError((routesReset && proxyCleared)
            ? tr("Unable to install the blocked startup route before connecting the path.")
            : tr("Unable to restore Native after the blocked startup route failed."));
        return;
    }

    ++m_generation;
    if (pathId == 0)
        pathId = ++m_nextPathId;
    m_status = tr("Starting path. Egress and network capabilities have not been probed.");
    request({{u"method"_s, u"open"_s}, {u"configPath"_s, QFileInfo(configPath).absoluteFilePath()},
        {u"proxyName"_s, proxyName}, {u"pathId"_s, QString::number(pathId)},
        {u"generation"_s, m_generation}, {u"interfaceName"_s, interfaceName},
        {u"configuredServerId"_s, configuredServerId}, {u"dns"_s, dns}});
}

bool Net::PathManager::setPolicy(const QString &mode, const QString &nativeInterface)
{
    if (controlBusy())
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
    const QString currentMode = m_storePolicy.get(u"pinned"_s);
    const QString currentInterface = m_storeNativeInterface;
    if ((currentMode == mode) && (currentInterface == (mixed ? nativeInterface : QString())))
        return true;
    const QString previous = currentMode;
    const QString previousInterface = currentInterface;
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
    if (controlBusy())
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
    const bool foreground = (message.value(u"method"_s) != u"status"_s);
    m_requestQueue.append(std::move(message));
    if (m_process.state() == QProcess::NotRunning)
    {
        QString program = QDir(QCoreApplication::applicationDirPath()).filePath(u"qbutt-net"_s);
#ifdef Q_OS_WIN
        program += u".exe"_s;
#endif
        m_process.start(program, {u"--stdio"_s});
        m_timeout.start();
    }
    else
    {
        sendQueuedRequest();
    }
    if (foreground)
        emit changed();
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
    if (message.value(u"method"_s) == u"status"_s)
    {
        QJsonArray expectedPaths;
        for (const ActivePath &path : std::as_const(m_paths))
        {
            if (path.endpoint.port > 0)
            {
                expectedPaths.append(QJsonObject {{u"pathId"_s, QString::number(path.endpoint.pathId)},
                    {u"generation"_s, static_cast<qint64>(path.endpoint.generation)}});
            }
        }
        m_pendingRequest.insert(u"expectedPaths"_s, expectedPaths);
    }
    // Retain the continuation in the pending request, never in child IPC.
    message.remove(u"openInterfaceName"_s);
    QByteArray frame = QJsonDocument(message).toJson(QJsonDocument::Compact);
    frame.append('\n');
    if ((frame.size() > MAX_FRAME_BYTES) || (m_process.write(frame) != frame.size()))
    {
        fail(tr("Unable to send a bounded qbutt-net control request."));
        return;
    }
    m_timeout.start(message.value(u"method"_s).toString().startsWith(u"resolve") ? 8000 : 15000);
}

void Net::PathManager::sendQueuedRequest()
{
    if ((m_pendingId != 0) || m_requestQueue.isEmpty())
        return;
    QJsonObject queued = m_requestQueue.takeFirst();
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
    if (message.value(u"id"_s) == QJsonValue(0))
    {
        if (message.value(u"v"_s) != QJsonValue(PROTOCOL_VERSION))
        {
            fail(tr("qbutt-net returned an incompatible event."));
            return;
        }
        handleEvent(message);
        return;
    }
    if ((message.value(u"v"_s) != QJsonValue(PROTOCOL_VERSION)) || (m_pendingId == 0)
        || (message.value(u"id"_s) != QJsonValue(m_pendingId)))
    {
        fail(tr("qbutt-net control version or request identifier does not match."));
        return;
    }
    const bool errorResponse = message.value(u"error"_s).isObject();
    const bool resultResponse = message.value(u"result"_s).isObject();
    if ((message.size() != 3) || (errorResponse == resultResponse))
    {
        fail(tr("qbutt-net returned an invalid control response envelope."));
        return;
    }
    const QJsonObject request = m_pendingRequest;
    m_pendingRequest = {};
    m_pendingId = 0;
    m_timeout.stop();
    const QString method = request.value(u"method"_s).toString();
    if (errorResponse)
    {
        // Do not expose arbitrary child strings: malformed subscriptions can
        // place credentials in parser/adapter errors.
        const QJsonObject error = message.value(u"error"_s).toObject();
        const QString errorCode = error.value(u"code"_s).toString();
        const bool exactError = (error.size() == 2) && error.value(u"code"_s).isString()
            && error.value(u"message"_s).isString() && (error.value(u"message"_s).toString() == errorCode);
        const bool operationalGatewayError = exactError
            && (((method == u"gateway.open") && ((errorCode == u"invalid_gateway")
                || (errorCode == u"gateway_credentials_unreadable")
                || (errorCode == u"gateway_credentials_invalid") || (errorCode == u"gateway_relay_failed")
                || (errorCode == u"gateway_connect_failed") || (errorCode == u"gateway_authentication_failed")
                || (errorCode == u"gateway_protocol_error") || (errorCode == u"gateway_datagrams_failed")
                || (errorCode == u"gateway_lease_rejected")))
            || ((method == u"gateway.renew") && ((errorCode == u"gateway_renew_failed")
                || (errorCode == u"gateway_not_open")))
            || ((method == u"gateway.close") && ((errorCode == u"gateway_close_failed")
                || (errorCode == u"gateway_not_open"))));
        if (operationalGatewayError)
        {
            handleGatewayFailure(request);
        }
        else if (method.startsWith(u"gateway."))
        {
            fail(tr("qbutt-net rejected a gateway path-generation invariant."));
        }
        else if ((method == u"hello") || !exactError)
        {
            fail(tr("The bundled qbutt-net rejected the protocol handshake."));
        }
        else if (method == u"status")
        {
            fail(tr("The bundled qbutt-net rejected transport status reporting."));
        }
        else if (method == u"close")
        {
            fail(tr("qbutt-net could not retire a stopped path generation."));
        }
        else
        {
            const bool bootstrapRequest = request.value(u"id"_s).toInteger() == m_bootstrapRequestId;
            if ((method == u"resolve") || (method == u"resolveNative"))
                finishResolution({}, u"path_dns_failed"_s);
            if ((method == u"open") && m_rolloverOpening)
            {
                m_rolloverFailed = true;
                m_rolloverOpening = false;
                reportError(tr("qbutt-net rejected a path while applying the public gateway settings."));
                startNextPathRollover();
            }
            else
            {
                sendQueuedRequest();
                if (!bootstrapRequest)
                    reportError(tr("qbutt-net rejected the request. Check the selected node and interface."));
                else
                    emit changed();
            }
        }
        return;
    }
    const QJsonObject result = message.value(u"result"_s).toObject();
    if (method == u"hello")
    {
        if ((result.size() != 4) || (result.value(u"protocol"_s) != QJsonValue(PROTOCOL_VERSION))
            || (result.value(u"name"_s).toString() != u"qbutt-net")
            || (result.value(u"upstreamRevision"_s).toString() != QBT_NET_UPSTREAM_REVISION)
            || !isSafeUnsignedInteger(result.value(u"maxFrameBytes"_s))
            || (result.value(u"maxFrameBytes"_s).toInteger() != MAX_FRAME_BYTES))
        {
            fail(tr("The bundled qbutt-net is incompatible with this application."));
            return;
        }
        sendQueuedRequest();
        return;
    }
    if (method == u"list")
    {
        static const QRegularExpression serverIdPattern {u"^[0-9a-f]{64}$"_s};
        const QJsonArray entries = result.value(u"proxies"_s).toArray();
        if ((result.size() != 1) || !result.value(u"proxies"_s).isArray() || (entries.size() > 1024))
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
            const QString serverId = entry.value(u"configuredServerId"_s).toString();
            if (!value.isObject() || (entry.size() != 3) || !entry.value(u"name"_s).isString()
                || !entry.value(u"type"_s).isString() || name.isEmpty() || type.isEmpty()
                || !entry.value(u"configuredServerId"_s).isString()
                || (!serverId.isEmpty() && !serverIdPattern.match(serverId).hasMatch()))
            {
                fail(tr("qbutt-net returned an invalid node description."));
                return;
            }
            // Only display-safe fields cross into UI/API diagnostics.
            proxies.append(QJsonObject {{u"name"_s, name}, {u"type"_s, type}, {u"configuredServerId"_s, serverId}});
        }
        if (request.contains(u"openInterfaceName"_s))
        {
            if ((proxies.size() != 1)
                || (proxies.first().toObject().value(u"name"_s) != request.value(u"proxyName"_s))
                || proxies.first().toObject().value(u"configuredServerId"_s).toString().isEmpty())
            {
                fail(tr("qbutt-net returned an unexpected selected node."));
                return;
            }
            openIdentifiedPath(request.value(u"configPath"_s).toString(), request.value(u"proxyName"_s).toString(),
                request.value(u"openInterfaceName"_s).toString(),
                proxies.first().toObject().value(u"configuredServerId"_s).toString());
        }
        else
        {
            m_storeConfigurationPath = request.value(u"configPath"_s).toString();
            m_proxies = proxies;
            emit proxiesLoaded(proxies);
        }
    }
    else if (method == u"open")
    {
        const int port = result.value(u"port"_s).toInt();
        const QString username = result.value(u"socksUsername"_s).toString();
        const QString password = result.value(u"socksPassword"_s).toString();
        const QJsonObject capabilities = result.value(u"capabilities"_s).toObject();
        const QString udp = capabilities.value(u"udp"_s).toString();
        if ((result.size() != 9) || (capabilities.size() != 6)
            || !isSafeUnsignedInteger(result.value(u"port"_s)) || !result.value(u"pathId"_s).isString()
            || !isSafeUnsignedInteger(result.value(u"generation"_s)) || !result.value(u"interfaceName"_s).isString()
            || !result.value(u"socksUsername"_s).isString() || !result.value(u"socksPassword"_s).isString()
            || !result.value(u"capabilities"_s).isObject()
            || (result.value(u"host"_s).toString() != u"127.0.0.1") || (port < 1) || (port > 65535)
            || username.isEmpty() || password.isEmpty() || (username.toUtf8().size() > 255)
            || (password.toUtf8().size() > 255) || (result.value(u"pathId"_s) != request.value(u"pathId"_s))
            || (result.value(u"generation"_s) != request.value(u"generation"_s))
            || (result.value(u"interfaceName"_s) != request.value(u"interfaceName"_s))
            || (result.value(u"configuredServerId"_s) != request.value(u"configuredServerId"_s))
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
        const bool replacePrimary = !m_rolloverOpening
            && (m_storePolicy.get(u"pinned"_s) == u"pinned") && !isOpen();
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
        path.configurationPath = request.value(u"configPath"_s).toString();
        path.edgeId = result.value(u"configuredServerId"_s).toString();
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
        if (primary.port > 0)
        {
            proxy.port = primary.port;
            proxy.username = primary.username;
            proxy.password = primary.password;
        }
        if (!ProxyConfigurationManager::instance()->setRuntimeProxy(proxy))
        {
            fail(tr("Unable to save the pinned startup policy. The path remains blocked."));
            return;
        }
        m_storeConfigurationPath = request.value(u"configPath"_s).toString();
        m_storeProxyName = request.value(u"proxyName"_s).toString();
        m_storeInterfaceName = request.value(u"interfaceName"_s).toString();
        const auto opened = std::ranges::find(m_paths, path.endpoint.pathId,
            [](const ActivePath &entry) { return entry.endpoint.pathId; });
        const bool gatewayQueued = (opened != m_paths.end()) && queueGatewayOpen(*opened);
        if (!gatewayQueued && !applyRoutes())
        {
            fail(tr("Unable to apply the network routes returned by qbutt-net."));
            return;
        }
        m_status = gatewayQueued
            ? tr("TCP endpoint ready for %1. Acquiring its public gateway lease before route activation.")
                .arg(request.value(u"proxyName"_s).toString())
            : tr("TCP endpoint ready: %1\n"
                "Egress, UDP, public inbound and throughput: unknown (not probed).")
                .arg(request.value(u"proxyName"_s).toString());
        if (m_rolloverOpening && !gatewayQueued)
        {
            m_rolloverOpening = false;
            startNextPathRollover();
        }
        if (!m_statusRefresh.isActive())
            m_statusRefresh.start();
    }
    else if ((method == u"gateway.open") || (method == u"gateway.renew"))
    {
        const QString pathIdText = result.value(u"pathId"_s).toString();
        bool validPathId = false;
        const quint64 pathId = pathIdText.toULongLong(&validPathId);
        const qint64 generationValue = result.value(u"generation"_s).toInteger();
        const auto path = std::ranges::find(m_paths, pathId,
            [](const ActivePath &entry) { return entry.endpoint.pathId; });
        const auto publicEndpoint = numericEndpoint(result.value(u"publicEndpoint"_s).toString());
        const auto relayEndpoint = numericEndpoint(u"%1:%2"_s.arg(result.value(u"relayHost"_s).toString())
            .arg(result.value(u"relayPort"_s).toInt()));
        const bool tcp = result.value(u"tcp"_s).toBool();
        const bool udp = result.value(u"udp"_s).toBool();
        const bool publicIPv4 = publicEndpoint
            && (publicEndpoint->address.protocol() == QAbstractSocket::IPv4Protocol);
        const qint64 expires = result.value(u"expiresUnixMilli"_s).toInteger();
        const qint64 now = QDateTime::currentMSecsSinceEpoch();
        const bool exactShape = (result.size() == 8) && result.value(u"pathId"_s).isString()
            && isSafeUnsignedInteger(result.value(u"generation"_s)) && result.value(u"publicEndpoint"_s).isString()
            && result.value(u"tcp"_s).isBool() && result.value(u"udp"_s).isBool()
            && isSafeUnsignedInteger(result.value(u"expiresUnixMilli"_s))
            && result.value(u"relayHost"_s).isString()
            && isSafeUnsignedInteger(result.value(u"relayPort"_s));
        if (!exactShape || !validPathId || (QString::number(pathId) != pathIdText) || (generationValue <= 0)
            || (path == m_paths.end()) || (path->endpoint.port == 0)
            || (path->endpoint.generation != static_cast<quint64>(generationValue))
            || (request.value(u"pathId"_s) != result.value(u"pathId"_s))
            || (request.value(u"generation"_s) != result.value(u"generation"_s))
            || !publicEndpoint || (publicEndpoint->text != result.value(u"publicEndpoint"_s).toString())
            || !publicEndpoint->address.isGlobal()
            || (publicIPv4 ? !path->endpoint.supportsIPv4 : !path->endpoint.supportsIPv6)
            || (udp && !path->endpoint.supportsUdp)
            || !relayEndpoint || (relayEndpoint->address != QHostAddress::LocalHost)
            || (result.value(u"relayHost"_s).toString() != u"127.0.0.1")
            || (expires <= now) || (expires > (now + 300000))
            || ((method == u"gateway.open")
                && ((tcp != request.value(u"gateway"_s).toObject().value(u"tcp"_s).toBool())
                    || (udp != request.value(u"gateway"_s).toObject().value(u"udp"_s).toBool())))
            || ((method == u"gateway.renew") && (!path->publicLease
                || (path->publicLease->publicEndpoint != publicEndpoint->text)
                || (path->publicLease->route.relayPort != relayEndpoint->port)
                || (path->publicLease->tcp != tcp) || (path->publicLease->udp != udp)
                || (expires <= path->publicLease->expiresUnixMilli))))
        {
            fail(tr("qbutt-net returned an invalid public gateway lease."));
            return;
        }
        if (method == u"gateway.open")
        {
            ActivePath::PublicLease lease;
            lease.route = {.pathId = path->endpoint.pathId, .generation = path->endpoint.generation,
                .publicAddress = publicEndpoint->address.toString(), .publicPort = publicEndpoint->port,
                .relayAddress = relayEndpoint->address.toString(), .relayPort = relayEndpoint->port};
            lease.publicEndpoint = publicEndpoint->text;
            lease.family = publicIPv4 ? u"ipv4"_s : u"ipv6"_s;
            lease.tcp = tcp;
            lease.udp = udp;
            lease.expiresUnixMilli = expires;
            path->publicLease = std::move(lease);
            path->endpoint.publicAddress = publicEndpoint->address.toString();
            path->endpoint.publicPort = publicEndpoint->port;
            path->endpoint.publicTcp = tcp;
            path->endpoint.publicUdp = udp;
            if (!applyTrustedInboundRoutes() || !applyRoutes())
            {
                fail(tr("Unable to register the verified public gateway lease with libtorrent."));
                return;
            }
        }
        else
        {
            path->publicLease->expiresUnixMilli = expires;
        }
        path->capabilities.insert(u"publicTcp"_s, tcp ? u"leased"_s : u"unavailable"_s);
        path->capabilities.insert(u"publicUdp"_s, udp ? u"leased"_s : u"unavailable"_s);
        m_status = tr("Public gateway lease active for %1 until %2.")
            .arg(publicEndpoint->text, QDateTime::fromMSecsSinceEpoch(expires).toString(Qt::ISODate));
        scheduleGatewayRenewal();
        if ((method == u"gateway.open") && m_rolloverOpening)
        {
            m_rolloverOpening = false;
            startNextPathRollover();
        }
    }
    else if (method == u"gateway.close")
    {
        const QString pathIdText = request.value(u"pathId"_s).toString();
        const quint64 pathId = pathIdText.toULongLong();
        auto path = std::ranges::find(m_paths, pathId,
            [](const ActivePath &entry) { return entry.endpoint.pathId; });
        if (!result.isEmpty() || (path == m_paths.end()) || !path->publicLease
            || (path->endpoint.generation != static_cast<quint64>(request.value(u"generation"_s).toInteger())))
        {
            fail(tr("qbutt-net did not retire the expected public gateway lease."));
            return;
        }
        if (m_pendingStopPath != pathIdText)
        {
            fail(tr("qbutt-net retired an unexpected public gateway lease."));
            return;
        }
        clearGatewayLease(*path);
        m_pendingStopPath.clear();
        if (!finishStopPath(pathIdText))
            return;
    }
    else if (method == u"status")
    {
        const QJsonArray entries = result.value(u"paths"_s).toArray();
        const QJsonArray expectedPaths = request.value(u"expectedPaths"_s).toArray();
        if ((result.size() != 1) || !result.value(u"paths"_s).isArray()
            || (entries.size() != expectedPaths.size()) || (entries.size() > 8))
        {
            fail(tr("qbutt-net returned an invalid transport status."));
            return;
        }
        QList<quint64> seen;
        for (const QJsonValue &value : entries)
        {
            const QJsonObject entry = value.toObject();
            const QString pathIdText = entry.value(u"pathId"_s).toString();
            bool validPathId = false;
            const quint64 pathId = pathIdText.toULongLong(&validPathId);
            const qint64 generation = entry.value(u"generation"_s).toInteger();
            const auto wire = wireCounters(entry.value(u"wire"_s));
            const auto expected = std::ranges::find_if(expectedPaths, [&](const QJsonValue &candidate)
            {
                const QJsonObject path = candidate.toObject();
                return (path.value(u"pathId"_s) == entry.value(u"pathId"_s))
                    && (path.value(u"generation"_s) == entry.value(u"generation"_s));
            });
            if (!value.isObject() || (entry.size() != 3) || !validPathId || (pathId == 0)
                || (QString::number(pathId) != pathIdText) || !isSafeUnsignedInteger(entry.value(u"generation"_s))
                || (generation <= 0) || !wire || seen.contains(pathId) || (expected == expectedPaths.end()))
            {
                fail(tr("qbutt-net returned status for an invalid path generation."));
                return;
            }
            seen.append(pathId);

            // Foreground work may have changed local path state while this
            // low-priority snapshot was in flight. Validate it, then discard
            // its now-stale counters before sending queued work.
            if (!m_requestQueue.isEmpty())
                continue;
            const auto path = std::ranges::find(m_paths, pathId,
                [](const ActivePath &candidate) { return candidate.endpoint.pathId; });
            const bool monotonic = (path != m_paths.end()) && (path->wire.isEmpty()
                || std::ranges::all_of(path->wire.keys(), [&](const QString &field)
                {
                    return wire->value(field).toDouble() >= path->wire.value(field).toDouble();
                }));
            if ((path == m_paths.end()) || (path->endpoint.port == 0)
                || (path->endpoint.generation != static_cast<quint64>(generation)) || !monotonic)
            {
                fail(tr("qbutt-net returned stale or decreasing transport counters."));
                return;
            }
            path->wire = *wire;
        }
    }
    else if ((method == u"resolve") || (method == u"resolveNative"))
    {
        const PeerRouteEndpoint *endpoint = findEndpoint(request.value(u"pathId"_s).toString().toULongLong(),
            static_cast<quint64>(request.value(u"generation"_s).toInteger()));
        const auto path = std::ranges::find(m_paths, request.value(u"pathId"_s).toString().toULongLong(),
            [](const ActivePath &entry) { return entry.endpoint.pathId; });
        if (!endpoint)
        {
            finishResolution({}, u"path_stopped"_s);
        }
        else
        {
            const QJsonArray values = result.value(u"addresses"_s).toArray();
            if ((result.size() != ((method == u"resolveNative") ? 3 : 1)) || !result.value(u"addresses"_s).isArray()
                || values.isEmpty() || (values.size() > 64))
            {
                fail(tr("qbutt-net returned an invalid DNS address list."));
                return;
            }
            QList<QHostAddress> addresses;
            const QString family = request.value(u"family"_s).toString();
            const QString policyFamily = ((method == u"resolveNative")
                ? request.value(u"dns"_s).toObject() : path->dnsPolicy).value(u"family"_s).toString();
            if ((method == u"resolveNative") && ((result.value(u"pathId"_s) != request.value(u"pathId"_s))
                || (result.value(u"generation"_s) != request.value(u"generation"_s))))
            {
                fail(tr("qbutt-net returned a mismatched DNS path generation."));
                return;
            }
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
    else if (method == u"close")
    {
        if (!result.isEmpty())
        {
            fail(tr("qbutt-net returned an invalid path close response."));
            return;
        }
        if (!std::ranges::any_of(m_paths, [](const ActivePath &path) { return path.endpoint.port > 0; }))
            m_statusRefresh.stop();
    }
    else
    {
        fail(tr("qbutt-net returned a response for an unknown request."));
        return;
    }
    sendQueuedRequest();
    if (method != u"status")
        emit changed();
}

void Net::PathManager::handleEvent(const QJsonObject &message)
{
    if (message.value(u"event"_s).toString() == u"gatewayClosed")
    {
        const QString pathIdText = message.value(u"pathId"_s).toString();
        bool validPathId = false;
        const quint64 pathId = pathIdText.toULongLong(&validPathId);
        const qint64 generation = message.value(u"generation"_s).toInteger();
        if ((message.size() != 6) || !isSafeUnsignedInteger(message.value(u"v"_s))
            || !isSafeUnsignedInteger(message.value(u"id"_s)) || !message.value(u"event"_s).isString()
            || !message.value(u"pathId"_s).isString() || !isSafeUnsignedInteger(message.value(u"generation"_s))
            || !message.value(u"reason"_s).isString() || !validPathId || (pathId == 0)
            || (QString::number(pathId) != pathIdText) || (generation <= 0)
            || (message.value(u"reason"_s).toString() != u"gateway_closed"))
        {
            fail(tr("qbutt-net returned an invalid terminal gateway event."));
            return;
        }
        const auto path = std::ranges::find(m_paths, pathId,
            [](const ActivePath &entry) { return entry.endpoint.pathId; });
        if ((path == m_paths.end()) || (path->endpoint.generation < static_cast<quint64>(generation)))
        {
            fail(tr("qbutt-net returned a terminal event for an unknown path generation."));
            return;
        }
        if ((path->endpoint.generation > static_cast<quint64>(generation)) || !path->publicLease)
        {
            return;
        }
        if (!beginPathRollover(activePathRollover(),
            tr("A public gateway connection ended. Reconnecting paths with new generations.")))
        {
            fail(tr("The ended public gateway generation could not be retired safely."));
        }
        return;
    }

    const QString pathIdText = message.value(u"pathId"_s).toString();
    bool validPathId = false;
    const quint64 pathId = pathIdText.toULongLong(&validPathId);
    const qint64 generationValue = message.value(u"generation"_s).toInteger();
    const auto path = std::ranges::find(m_paths, pathId,
        [](const ActivePath &entry) { return entry.endpoint.pathId; });
    const auto remote = numericEndpoint(message.value(u"remote"_s).toString());
    const QString tokenText = message.value(u"relayToken"_s).toString();
    const bool exactShape = (message.size() == 10) && isSafeUnsignedInteger(message.value(u"v"_s))
        && isSafeUnsignedInteger(message.value(u"id"_s)) && message.value(u"event"_s).isString()
        && message.value(u"pathId"_s).isString() && isSafeUnsignedInteger(message.value(u"generation"_s))
        && message.value(u"remote"_s).isString() && message.value(u"publicEndpoint"_s).isString()
        && message.value(u"relayHost"_s).isString() && isSafeUnsignedInteger(message.value(u"relayPort"_s))
        && message.value(u"relayToken"_s).isString();
    if (!exactShape || (message.value(u"event"_s).toString() != u"incomingTcp") || !validPathId
        || (pathId == 0) || (QString::number(pathId) != pathIdText) || (generationValue <= 0)
        || (path == m_paths.end()) || !path->publicLease || !path->publicLease->tcp
        || (path->endpoint.generation != static_cast<quint64>(generationValue))
        || (path->publicLease->expiresUnixMilli <= QDateTime::currentMSecsSinceEpoch())
        || (message.value(u"publicEndpoint"_s).toString() != path->publicLease->publicEndpoint)
        || (message.value(u"relayHost"_s).toString() != u"127.0.0.1")
        || (message.value(u"relayPort"_s).toInt() != path->publicLease->route.relayPort)
        || !remote || (remote->text != message.value(u"remote"_s).toString())
        || !QRegularExpression(u"^[0-9a-f]{64}$"_s).match(tokenText).hasMatch())
    {
        fail(tr("qbutt-net returned invalid trusted incoming metadata."));
        return;
    }
    QByteArray token = QByteArray::fromHex(tokenText.toLatin1());
    const bool accepted = BitTorrent::Session::instance()->acceptTrustedInbound(path->publicLease->route,
        remote->address.toString(), remote->port, token);
    token.fill('\0');
    if (!accepted)
    {
        m_status = tr("An incoming gateway connection was rejected before it entered the torrent session.");
        emit changed();
    }
}

bool Net::PathManager::applyTrustedInboundRoutes()
{
    QList<TrustedInboundRoute> routes;
    for (const ActivePath &path : std::as_const(m_paths))
    {
        if (path.publicLease && path.publicLease->tcp)
            routes.append(path.publicLease->route);
    }
    return BitTorrent::Session::instance()->setTrustedInboundRoutes(routes);
}

bool Net::PathManager::queueGatewayOpen(const ActivePath &path)
{
    if (path.publicLease || (path.endpoint.port == 0) || (!m_storeGatewayTcp.get() && !m_storeGatewayUdp.get()))
        return false;
    const QJsonObject gateway {{u"controlAddress"_s, m_storeGatewayControlAddress.get()},
        {u"datagramAddress"_s, m_storeGatewayDatagramAddress.get()},
        {u"serverName"_s, m_storeGatewayServerName.get()}, {u"caPath"_s, m_storeGatewayCaPath.get()},
        {u"certificatePath"_s, m_storeGatewayCertificatePath.get()},
        {u"privateKeyPath"_s, m_storeGatewayPrivateKeyPath.get()},
        {u"port"_s, m_storeGatewayPort.get()}, {u"tcp"_s, m_storeGatewayTcp.get()},
        {u"udp"_s, m_storeGatewayUdp.get()}, {u"ttlSeconds"_s, GATEWAY_TTL_SECONDS}};
    m_requestQueue.append({{u"method"_s, u"gateway.open"_s},
        {u"pathId"_s, QString::number(path.endpoint.pathId)},
        {u"generation"_s, static_cast<qint64>(path.endpoint.generation)}, {u"gateway"_s, gateway}});
    return true;
}

void Net::PathManager::queueGatewayClose(const ActivePath &path)
{
    m_gatewayRenewal.stop();
    m_requestQueue.append({{u"method"_s, u"gateway.close"_s},
        {u"pathId"_s, QString::number(path.endpoint.pathId)},
        {u"generation"_s, static_cast<qint64>(path.endpoint.generation)}});
}

void Net::PathManager::scheduleGatewayRenewal()
{
    m_gatewayRenewal.stop();
    qint64 earliest = 0;
    for (const ActivePath &path : std::as_const(m_paths))
    {
        if (path.publicLease && ((earliest == 0) || (path.publicLease->expiresUnixMilli < earliest)))
            earliest = path.publicLease->expiresUnixMilli;
    }
    if (earliest == 0)
        return;
    const qint64 remaining = earliest - QDateTime::currentMSecsSinceEpoch();
    m_gatewayRenewal.start(static_cast<int>(std::clamp(
        remaining - GATEWAY_RENEWAL_HEADROOM_MS, 0LL, 2100000000LL)));
}

void Net::PathManager::clearGatewayLease(ActivePath &path)
{
    path.publicLease.reset();
    path.endpoint.publicAddress.clear();
    path.endpoint.publicPort = 0;
    path.endpoint.publicTcp = false;
    path.endpoint.publicUdp = false;
    path.capabilities.insert(u"publicTcp"_s, u"unknown"_s);
    path.capabilities.insert(u"publicUdp"_s, u"unknown"_s);
}

QList<Net::PathManager::PathRollover> Net::PathManager::activePathRollover() const
{
    QList<PathRollover> result;
    const auto append = [this, &result](PathRollover path)
    {
        if ((path.pathId == 0) || (QString::number(path.pathId) == m_pendingStopPath)
            || std::ranges::any_of(result, [pathId = path.pathId](const PathRollover &entry)
            { return entry.pathId == pathId; }))
        {
            return;
        }
        result.append(std::move(path));
    };
    for (const ActivePath &path : m_paths)
    {
        if (path.endpoint.port > 0)
        {
            append({path.endpoint.pathId, path.configurationPath, path.proxyName, path.interfaceName,
                path.edgeId, path.dnsPolicy});
        }
    }
    const auto appendOpenRequest = [&append](const QJsonObject &request)
    {
        if (request.value(u"method"_s) != u"open"_s)
            return;
        append({request.value(u"pathId"_s).toString().toULongLong(),
            request.value(u"configPath"_s).toString(), request.value(u"proxyName"_s).toString(),
            request.value(u"interfaceName"_s).toString(), request.value(u"configuredServerId"_s).toString(),
            request.value(u"dns"_s).toObject()});
    };
    appendOpenRequest(m_pendingRequest);
    for (const QJsonObject &request : m_requestQueue)
        appendOpenRequest(request);
    for (const PathRollover &path : m_pathRollover)
        append(path);
    return result;
}

bool Net::PathManager::beginPathRollover(QList<PathRollover> paths, const QString &status)
{
    if (paths.isEmpty() || !shutdown())
        return false;
    m_pathRollover = std::move(paths);
    m_rolloverFailed = false;
    m_status = status;
    startNextPathRollover();
    return true;
}

void Net::PathManager::startNextPathRollover()
{
    if (m_rolloverOpening || (m_pendingId != 0) || !m_requestQueue.isEmpty())
        return;
    if (m_pathRollover.isEmpty())
    {
        const bool failed = m_rolloverFailed;
        m_rolloverFailed = false;
        m_status = failed
            ? tr("One or more paths or public gateway leases could not be reconnected.")
            : tr("Paths reconnected with new transport generations.");
        emit changed();
        return;
    }
    const PathRollover path = m_pathRollover.takeFirst();
    m_rolloverOpening = true;
    request({{u"method"_s, u"open"_s}, {u"configPath"_s, path.configurationPath},
        {u"proxyName"_s, path.proxyName}, {u"pathId"_s, QString::number(path.pathId)},
        {u"generation"_s, ++m_generation}, {u"interfaceName"_s, path.interfaceName},
        {u"configuredServerId"_s, path.edgeId}, {u"dns"_s, path.dnsPolicy}});
}

void Net::PathManager::handleGatewayFailure(const QJsonObject &request)
{
    const QString pathIdText = request.value(u"pathId"_s).toString();
    const auto path = std::ranges::find(m_paths, pathIdText.toULongLong(),
        [](const ActivePath &entry) { return entry.endpoint.pathId; });
    if ((path == m_paths.end())
        || (path->endpoint.generation != static_cast<quint64>(request.value(u"generation"_s).toInteger())))
    {
        fail(tr("qbutt-net failed a gateway request for an unknown path generation."));
        return;
    }
    const QString method = request.value(u"method"_s).toString();
    if (method == u"gateway.renew")
    {
        if (!path->publicLease || !beginPathRollover(activePathRollover(),
            tr("The public gateway lease ended. Reconnecting paths with new generations.")))
        {
            fail(tr("The ended public gateway generation could not be retired safely."));
        }
        return;
    }
    if (method == u"gateway.close")
    {
        if (!path->publicLease || (m_pendingStopPath != pathIdText))
        {
            fail(tr("qbutt-net failed an unexpected public gateway transition."));
            return;
        }
        clearGatewayLease(*path);
        m_pendingStopPath.clear();
        finishStopPath(pathIdText);
        return;
    }
    if ((method != u"gateway.open") || path->publicLease || !applyRoutes())
    {
        fail(tr("A failed public gateway transition could not be removed safely."));
        return;
    }
    m_status = tr("The public gateway could not be opened. The path is active for outgoing traffic only.");
    if (m_rolloverOpening)
    {
        m_rolloverFailed = true;
        m_rolloverOpening = false;
        startNextPathRollover();
    }
    else
    {
        sendQueuedRequest();
    }
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
    const bool resolving = m_pendingRequest.value(u"method"_s).toString().startsWith(u"resolve");
    if (!pathId.isEmpty() && controlBusy() && (!resolving || !m_requestQueue.isEmpty()))
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
        if (path->publicLease)
        {
            m_pendingStopPath = pathId;
            queueGatewayClose(*path);
            m_status = tr("Retiring the public gateway lease before disconnecting the path.");
            if (resolving && (m_resolution.value(u"pathId"_s).toString() == pathId))
                finishResolution({}, u"path_stopped"_s);
            sendQueuedRequest();
            emit changed();
            return;
        }
        finishStopPath(pathId);
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

bool Net::PathManager::finishStopPath(const QString &pathId)
{
    auto path = std::ranges::find(m_paths, pathId.toULongLong(),
        [](const ActivePath &entry) { return entry.endpoint.pathId; });
    if ((path == m_paths.end()) || (path->endpoint.port == 0) || path->publicLease)
        return false;
    const quint64 generation = path->endpoint.generation;
    path->endpoint.type = PeerRouteEndpoint::Type::Blocked;
    path->endpoint.port = 0;
    path->endpoint.username.clear();
    path->endpoint.password.clear();
    path->wire = {};
    if ((m_resolution.value(u"state"_s) == u"pending"_s)
        && (m_resolution.value(u"pathId"_s).toString() == pathId))
    {
        finishResolution({}, u"path_stopped"_s);
    }
    if (!applyTrustedInboundRoutes() || !applyRoutes())
    {
        fail(tr("Unable to revoke the selected network path."));
        return false;
    }
    BitTorrent::Session::instance()->invalidateNetworkRoute(path->endpoint.pathId, generation);
    if (path == m_paths.begin())
        ProxyConfigurationManager::instance()->setRuntimeProxy(blockedRuntimeProxy());
    m_status = tr("Path disconnected. Its existing peer connections have been closed.");
    scheduleGatewayRenewal();
    request({{u"method"_s, u"close"_s}, {u"pathId"_s, pathId},
        {u"generation"_s, static_cast<qint64>(generation)}});
    return true;
}

bool Net::PathManager::shutdown()
{
    m_dhtBootstrap.clear();
    m_bootstrapRequestId = 0;
    finishResolution({}, u"path_stopped"_s);
    auto *session = BitTorrent::Session::instance();
    m_gatewayRenewal.stop();
    m_statusRefresh.stop();
    for (ActivePath &path : m_paths)
    {
        clearGatewayLease(path);
        path.endpoint.type = PeerRouteEndpoint::Type::Blocked;
        path.endpoint.port = 0;
        path.endpoint.username.clear();
        path.endpoint.password.clear();
        path.wire = {};
    }
    bool routesRetired = session->setTrustedInboundRoutes({});
    if (ProxyConfigurationManager::instance()->hasRuntimeProxy())
        routesRetired = applyRoutes() && routesRetired;
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
    m_requestQueue.clear();
    m_pendingRequest = {};
    m_pendingId = 0;
    m_pendingStopPath.clear();
    m_pathRollover.clear();
    m_rolloverOpening = false;
    m_rolloverFailed = false;
    return (m_process.state() == QProcess::NotRunning) && routesRetired;
}
