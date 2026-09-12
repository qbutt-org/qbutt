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

class QNetworkReply;

namespace Net
{
    // Owns only the child and its single pinned proxy. libtorrent remains the
    // owner of the session, peers, pieces and transfer state.
    class PathManager final : public QObject
    {
        Q_OBJECT
        Q_DISABLE_COPY_MOVE(PathManager)

    public:
        explicit PathManager(QObject *parent = nullptr);
        ~PathManager() override;

        static PathManager *instance();
        bool isBusy() const;
        bool isOpen() const;
        QString status() const;
        QJsonObject statusData() const;
        QString subscriptionUrl() const;
        QString configurationPath() const;
        QString proxyName() const;
        QString interfaceName() const;
        void refreshSubscription(const QString &url);
        void inspectConfiguration(const QString &configPath);
        void openPath(const QString &configPath, const QString &proxyName,
            const QString &interfaceName);
        void useNative();
        void stopPath();
        bool shutdown();

    signals:
        void changed();
        void proxiesLoaded(const QJsonArray &proxies);

    private:
        void request(QJsonObject message);
        void send(QJsonObject message);
        void readOutput();
        void handleResponse(const QJsonObject &message);
        void fail(const QString &message);
        void reportError(const QString &message);

        QProcess m_process;
        QNetworkAccessManager m_network;
        QPointer<QNetworkReply> m_subscriptionReply;
        QTimer m_timeout;
        QByteArray m_output;
        QJsonObject m_queuedRequest;
        QJsonObject m_pendingRequest;
        QJsonArray m_proxies;
        int m_nextId = 0;
        int m_pendingId = 0;
        int m_generation = 0;
        bool m_open = false;
        QString m_pathId;
        QString m_status;
        SettingValue<QString> m_storeSubscriptionUrl;
        SettingValue<QString> m_storeConfigurationPath;
        SettingValue<QString> m_storeProxyName;
        SettingValue<QString> m_storeInterfaceName;
    };
}
