/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <QElapsedTimer>
#include <QHostAddress>
#include <QJsonArray>
#include <QJsonObject>
#include <QNetworkAccessManager>
#include <QObject>
#include <QPointer>
#include <QProcess>
#include <QStringList>
#include <QTimer>
#include <QVariantMap>

#include <optional>

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

        friend class PathManagerAcceptance;

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
        QStringList selectedNodes() const;
        QString preferredTransport(const QString &edgeId) const;
        bool setPreferredTransport(const QString &edgeId, const QString &proxyName = {});
        bool managedEnabled() const;
        bool setSelectedNodes(const QStringList &names);
        bool setManagedEnabled(bool enabled, const QString &interfaceName = {});
        QJsonObject dnsPolicy() const;
        QJsonObject gatewayConfiguration(const QString &configuredServerId) const;
        bool setDnsPolicy(const QString &server, const QString &bootstrapServer, const QString &family);
        bool setGatewayConfiguration(const QJsonObject &configuration, const QString &configuredServerId);
        qint64 resolveHost(const QString &pathId, quint64 generation, const QString &host, const QString &family);
        void refreshSubscription(const QString &url);
        void inspectConfiguration(const QString &configPath);
        void openPath(const QString &configPath, const QString &proxyName,
            const QString &interfaceName, const QStringList &reserveNames = {});
        void switchTransport(const QString &pathId, const QString &proxyName);
        bool setPolicy(const QString &mode, const QString &nativeInterface = {});
        bool useNative();
        bool reopenSelectedEdge(const QString &edgeId);
        void stopPath(const QString &pathId = {});

    signals:
        void changed();
        void proxiesLoaded(const QJsonArray &proxies);
        void dnsPolicyChanged();
        void hostResolved(qint64 requestId, quint64 pathId, quint64 generation,
            const QList<QHostAddress> &addresses, const QString &errorCode);

    private:
        static PathManager *m_instance;

        struct ActivePath;
        struct PathRollover;

        void request(QJsonObject message);
        void openIdentifiedPath(const QString &configPath, const QString &proxyName,
            const QString &interfaceName, const QString &configuredServerId, const QStringList &reserveNames,
            const QJsonObject &reserveServerIds);
        bool controlBusy() const;
        void send(QJsonObject message);
        void readOutput();
        void handleResponse(const QJsonObject &message);
        void handleEvent(const QJsonObject &message);
        void fail(const QString &message);
        void reportError(const QString &message);
        bool shutdown();
        bool finishUseNative();
        QList<PeerRouteEndpoint> nativeEndpointsForInterface(const QString &interfaceName) const;
        bool applyPolicy(const QString &mode, const QString &nativeInterface,
            QList<PeerRouteEndpoint> nativeEndpoints);
        bool applyRoutes();
        bool applyTrustedInboundRoutes();
        void finishResolution(const QList<QHostAddress> &addresses, const QString &errorCode = {});
        void sendQueuedRequest();
        void restoreSelectedNodes();
        void openNextSelectedNode();
        bool selectedNodesValid(const QStringList &names);
        bool queueGatewayOpen(const ActivePath &path);
        void queueGatewayClose(const ActivePath &path);
        void scheduleGatewayRenewal();
        void clearGatewayLease(ActivePath &path);
        QList<PathRollover> activePathRollover() const;
        bool beginPathRollover(QList<PathRollover> paths, const QString &status);
        void handleGatewayFailure(const QJsonObject &request);
        void startNextPathRollover();
        bool finishStopPath(const QString &pathId);
        bool revokePath(ActivePath &path);
        const PeerRouteEndpoint *findEndpoint(quint64 pathId, quint64 generation) const;
        void queueDhtBootstrap(quint64 pathId, quint64 generation, bool ipv6);
        void processDhtBootstrap();

        struct DhtBootstrap
        {
            quint64 pathId;
            quint64 generation;
            bool ipv6;
            qsizetype nodeIndex = 0;
            quint16 port = 0;
        };

        struct ActivePath
        {
            struct PublicLease
            {
                TrustedInboundRoute route;
                QString publicEndpoint;
                QString family;
                bool tcp = false;
                bool udp = false;
                qint64 expiresUnixMilli = 0;
            };

            PeerRouteEndpoint endpoint;
            QString configurationPath;
            QString edgeId;
            QString configuredServerId;
            QString proxyName;
            QStringList reserveNames;
            QJsonObject reserveServerIds;
            QJsonObject transport;
            QString interfaceName;
            QJsonObject capabilities;
            QJsonObject dnsPolicy;
            QJsonObject wire;
            QJsonObject health;
            QJsonObject relayRate;
            qint64 wireSampleTimeMs = 0;
            qint64 closedPayloadDownload = 0;
            qint64 closedPayloadUpload = 0;
            std::optional<PublicLease> publicLease;
        };

        struct PathRollover
        {
            quint64 pathId = 0;
            QString configurationPath;
            QString proxyName;
            QStringList reserveNames;
            QJsonObject reserveServerIds;
            QString interfaceName;
            QString configuredServerId;
            QJsonObject dnsPolicy;
        };

        QProcess m_process;
        QNetworkAccessManager m_network;
        QPointer<QNetworkReply> m_subscriptionReply;
        QTimer m_timeout;
        QTimer m_gatewayRenewal;
        QTimer m_statusRefresh;
        QElapsedTimer m_wireClock;
        QByteArray m_output;
        QList<QJsonObject> m_requestQueue;
        QJsonObject m_pendingRequest;
        QJsonObject m_resolution;
        QJsonArray m_proxies;
        QList<ActivePath> m_paths;
        QList<PeerRouteEndpoint> m_nativeEndpoints;
        QList<DhtBootstrap> m_dhtBootstrap;
        qint64 m_bootstrapRequestId = 0;
        qint64 m_nextId = 0;
        qint64 m_pendingId = 0;
        qint64 m_generation = 0;
        quint64 m_nextPathId = 2;
        quint64 m_nativeGeneration = 0;
        QString m_pendingStopPath;
        QList<PathRollover> m_pathRollover;
        QStringList m_pendingNodes;
        QStringList m_failedNodes;
        QString m_openingNode;
        QStringList m_pendingPreferredEdges;
        std::optional<PathRollover> m_pendingEdgeReopen;
        bool m_nativePending = false;
        bool m_nativeWasEnabled = false;
        bool m_ignoreProcessExit = false;
        bool m_restoreStarted = false;
        bool m_rolloverOpening = false;
        bool m_rolloverFailed = false;
        QString m_status;
        SettingValue<QString> m_storeSubscriptionUrl;
        SettingValue<QString> m_storeConfigurationPath;
        SettingValue<QString> m_storeProxyName;
        SettingValue<QString> m_storeInterfaceName;
        SettingValue<QStringList> m_storeSelectedNodes;
        SettingValue<QVariantMap> m_storePreferredTransports;
        SettingValue<bool> m_storeManagedEnabled;
        SettingValue<QString> m_storePolicy;
        SettingValue<QString> m_storeNativeInterface;
        SettingValue<QString> m_storeDnsServer;
        SettingValue<QString> m_storeBootstrapServer;
        SettingValue<QString> m_storeDnsFamily;
        SettingValue<QVariantMap> m_storeGatewayByEdge;
    };
}
