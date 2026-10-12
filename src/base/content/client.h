// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QJsonObject>
#include <QJsonValue>
#include <QMap>
#include <QObject>
#include <QProcess>
#include <QSet>

namespace Content
{
    class Client final : public QObject
    {
        Q_OBJECT

    public:
        static Client *instance();
        static void freeInstance();
        ~Client() override;

        qint64 request(const QString &method, const QJsonObject &params = {});
        void cancel(qint64 id);
        void replyBrowser(quint64 generation, quint64 id, const QByteArray &reply);

    signals:
        void eventReceived(qint64 id, const QString &event, const QJsonValue &data);
        void browserRequested(quint64 generation, quint64 id, const QByteArray &request);
        void browserCanceled(quint64 generation, quint64 id);
        void browserSessionClosed(quint64 generation);

    private:
        explicit Client(QObject *parent);
        void flushRequests();
        void readOutput();
        void failPending(const QString &message);

        QProcess m_process;
        QByteArray m_output;
        QMap<qint64, QByteArray> m_queue;
        QSet<qint64> m_pending;
        QSet<quint64> m_browserPending;
        quint64 m_generation = 0;
        qint64 m_nextId = 0;
        static Client *m_instance;
    };
}
