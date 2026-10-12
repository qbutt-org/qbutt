// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QQuickWidget>
#include <QUrl>

namespace Qbutt
{
    class ContentController;
}

namespace Content
{
    class ContentView final : public QQuickWidget
    {
        Q_OBJECT

    public:
        explicit ContentView(const QString &profileDir, QWidget *parent = nullptr);
        ~ContentView() override;
        void giveFocusToSearchInput();
        bool isReading() const;
        bool confirmPlayback(quint64 launchToken);
        bool finishPlayback(quint64 launchToken);
        void initializeBrowser();
        void relayBrowserRequest(quint64 generation, quint64 id, const QByteArray &request);
        void cancelBrowserRequest(quint64 generation, quint64 id);
        void closeBrowserSession(quint64 generation);

    signals:
        void torrentReady(const QString &magnet);
        void torrentMetadataReady(const QString &identity, const QByteArray &data);
        void videoReady(quint64 launchToken, const QString &identity, const QUrl &url,
            const QUrl &proxyUrl, const QString &title, const QString &contentKind);
        void readerActiveChanged();
        void searchFinished(bool failed);
        void browserReply(quint64 generation, quint64 id, const QByteArray &reply);

    private:
        void changeEvent(QEvent *event) override;

        Qbutt::ContentController *m_controller = nullptr;
    };
}
