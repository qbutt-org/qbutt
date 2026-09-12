/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <QJsonArray>
#include <QJsonObject>
#include <QNetworkAccessManager>
#include <QObject>
#include <QPointer>
#include <QProcess>
#include <QTimer>

#include "base/settingvalue.h"
#include "peerroute.h"

class QNetworkReply;

namespace Net
{
    // Owns the child and its payload endpoints. libtorrent remains the
    // owner of the session, peers, pieces and transfer state.
    class PathManager final : public QObject
    {
        Q_OBJECT
        Q_DISABLE_COPY_MOVE(PathManager)

        PathManager();
        ~PathManager() override;

    public:
        static void initInstance();
        static void freeInstance();
        static PathManager *instance();
        bool isBusy() const;
        bool isOpen() const;
        QString status() const;
        QJsonObject statusData(bool includePeers = false) const;
        QString subscriptionUrl() const;
        QString configurationPath() const;
        QString proxyName() const;
        QString interfaceName() const;
        void refreshSubscription(const QString &url);
        void inspectConfiguration(const QString &configPath);
        void openPath(const QString &configPath, const QString &proxyName,
            const QString &interfaceName, const QString &edgeId = {});
        void setPolicy(const QString &mode, const QString &nativeInterface = {});
        void useNative();
        void stopPath(const QString &pathId = {});

    signals:
        void changed();
        void proxiesLoaded(const QJsonArray &proxies);

    private:
        static PathManager *m_instance;

        void request(QJsonObject message);
        void send(QJsonObject message);
        void readOutput();
        void handleResponse(const QJsonObject &message);
        void fail(const QString &message);
        void reportError(const QString &message);
        bool shutdown();
        void applyRoutes();

        struct ActivePath
        {
            PeerRouteEndpoint endpoint;
            QString edgeId;
            QString proxyName;
            QString interfaceName;
            QJsonObject capabilities;
            qint64 closedPayloadDownload = 0;
            qint64 closedPayloadUpload = 0;
        };

        QProcess m_process;
        QNetworkAccessManager m_network;
        QPointer<QNetworkReply> m_subscriptionReply;
        QTimer m_timeout;
        QByteArray m_output;
        QJsonObject m_queuedRequest;
        QJsonObject m_pendingRequest;
        QJsonArray m_proxies;
        QList<ActivePath> m_paths;
        QList<PeerRouteEndpoint> m_nativeEndpoints;
        int m_nextId = 0;
        int m_pendingId = 0;
        int m_generation = 0;
        quint64 m_nextPathId = 2;
        quint64 m_nativeGeneration = 0;
        QString m_status;
        SettingValue<QString> m_storeSubscriptionUrl;
        SettingValue<QString> m_storeConfigurationPath;
        SettingValue<QString> m_storeProxyName;
        SettingValue<QString> m_storeInterfaceName;
        SettingValue<bool> m_storeMixed;
        SettingValue<QString> m_storeNativeInterface;
    };
}
