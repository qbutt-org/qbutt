// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QObject>
#include <QUrl>

namespace Content
{
    class BrowserCapture final : public QObject
    {
        Q_OBJECT

    public:
        explicit BrowserCapture(QObject *parent = nullptr);

        void open(const QUrl &url);
        void retry();
        void cancel();
        void finishDownload(const QString &captureId, const QString &reason);
        void finishWatch(const QString &captureId, const QString &reason);

    signals:
        void preparationChanged(const QString &message);
        void browserRequired();
        void error(const QString &message);
        void downloadRequested(const QUrl &page, const QString &title, const QString &captureId);
        void watchRequested(const QUrl &page, const QString &title, const QString &captureId);
        void downloadCancelled();
        void watchCancelled();
        void completed();

    private:
        enum class Action { None, Watch, Download };

        void dispatch();
        void finish(const QString &captureId, const QString &reason, Action action);

        QUrl m_page;
        QString m_title;
        QString m_captureId;
        Action m_action = Action::None;
        bool m_pending = false;

    };
}
