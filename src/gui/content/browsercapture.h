// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QJsonObject>
#include <QObject>
#include <QUrl>

class QJsonValue;

namespace Content
{
    class BrowserCapture final : public QObject
    {
        Q_OBJECT

    public:
        explicit BrowserCapture(QObject *parent = nullptr);
        ~BrowserCapture() override;

        void open(const QUrl &url);
        void retry();
        void cancel();
        QJsonObject downloadParameters(const QJsonObject &params) const;

    signals:
        void preparationChanged(const QString &message);
        void browserRequired();
        void error(const QString &message);
        void downloadRequested(const QJsonObject &params);
        void completed();

    private:
        void handleEvent(qint64 id, const QString &event, const QJsonValue &data);
        void play(const QJsonObject &resolved);

        qint64 m_request = 0;
        QJsonObject m_work;
        QJsonObject m_variant;
        QJsonObject m_playback;
    };
}
