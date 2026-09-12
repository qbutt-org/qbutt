/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "pathmanager.h"

#include <QCoreApplication>
#include <QDir>
#include <QFileInfo>
#include <QJsonDocument>
#include <QNetworkProxy>
#include <QNetworkReply>
#include <QNetworkRequest>
#include <QSaveFile>
#include <QSignalBlocker>
#include <QUrl>
#include <QUuid>

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

Net::PathManager::PathManager(QObject *parent)
    : QObject(parent)
    , m_pathId {QUuid::createUuid().toString(QUuid::WithoutBraces)}
    , m_status {ProxyConfigurationManager::instance()->hasRuntimeProxy()
        ? tr("Pinned path unavailable. Start a path to reconnect; automatic Native fallback is disabled.")
        : tr("Native / saved connection settings. No qbutt-net path is active.")}
    , m_storeSubscriptionUrl {u"Network/Paths/SubscriptionUrl"_s}
    , m_storeConfigurationPath {u"Network/Paths/ConfigurationPath"_s}
    , m_storeProxyName {u"Network/Paths/ProxyName"_s}
    , m_storeInterfaceName {u"Network/Paths/InterfaceName"_s}
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
}

Net::PathManager::~PathManager()
{
    shutdown();
}

Net::PathManager *Net::PathManager::instance()
{
    static auto *manager = new PathManager(QCoreApplication::instance());
    return manager;
}

bool Net::PathManager::isBusy() const
{
    return (m_pendingId != 0) || !m_queuedRequest.isEmpty() || m_subscriptionReply;
}

bool Net::PathManager::isOpen() const
{
    return m_open;
}

QString Net::PathManager::status() const
{
    return m_status;
}

QJsonObject Net::PathManager::statusData() const
{
    return {{u"v"_s, 1}, {u"busy"_s, isBusy()}, {u"open"_s, m_open},
        {u"pinned"_s, ProxyConfigurationManager::instance()->hasRuntimeProxy()},
        {u"status"_s, m_status}, {u"processId"_s, m_process.processId()},
        {u"pathId"_s, m_pathId}, {u"generation"_s, m_generation}, {u"nodes"_s, m_proxies},
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
    const QString &interfaceName)
{
    if (isBusy() || m_open)
    {
        reportError(tr("A path operation is already active. Stop the current path before changing it."));
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
    if (!proxyManager->setRuntimeProxy(blockedRuntimeProxy()))
    {
        reportError(tr("Unable to save the pinned startup policy. The path was not started."));
        return;
    }

    ++m_generation;
    m_status = tr("Starting pinned path. Egress and network capabilities have not been probed.");
    request({{u"method"_s, u"open"_s}, {u"configPath"_s, QFileInfo(configPath).absoluteFilePath()},
        {u"proxyName"_s, proxyName}, {u"pathId"_s, m_pathId}, {u"generation"_s, m_generation},
        {u"interfaceName"_s, interfaceName}});
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
        ProxyConfiguration proxy = blockedRuntimeProxy();
        proxy.port = static_cast<ushort>(port);
        proxy.username = username;
        proxy.password = password;
        if (!ProxyConfigurationManager::instance()->setRuntimeProxy(proxy))
        {
            fail(tr("Unable to save the pinned startup policy. The path remains blocked."));
            return;
        }
        m_open = true;
        m_storeConfigurationPath = request.value(u"configPath"_s).toString();
        m_storeProxyName = request.value(u"proxyName"_s).toString();
        m_storeInterfaceName = request.value(u"interfaceName"_s).toString();
        m_status = tr("Pinned TCP endpoint ready: %1\n"
            "Egress, UDP, public inbound and throughput: unknown (not probed).")
            .arg(request.value(u"proxyName"_s).toString());
    }
    emit changed();
}

void Net::PathManager::fail(const QString &message)
{
    shutdown();
    // A dead authenticated endpoint cannot become a direct connection. Keep
    // the effective proxy and its persisted startup requirement in place.
    reportError(message);
}

void Net::PathManager::reportError(const QString &message)
{
    m_status = message;
    emit changed();
}

void Net::PathManager::stopPath()
{
    if (isBusy())
        return;
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
    m_open = false;
    return m_process.state() == QProcess::NotRunning;
}
