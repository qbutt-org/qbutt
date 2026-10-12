// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QHash>
#include <QQuickWidget>
#include <QUrl>
#include <QVector>

namespace Qbutt
{
    class ContentController;
}

namespace Content
{
    class ReaderImageProvider;

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
        void setBrowserChoice(const QString &choice);
        void relayBrowserRequest(quint64 generation, quint64 id, const QByteArray &request);
        void cancelBrowserRequest(quint64 generation, quint64 id);
        void closeBrowserSession(quint64 generation);
        void openSharedLink(const QUrl &url);
        QQuickWidget *createDownloadsView(QWidget *parent);
        QQuickWidget *createSourcesView(QWidget *parent);
        void initDownloads(const QString &destination);
        bool downloadPlayback(quint64 launchToken);
        bool downloadCapture(const QUrl &page, const QString &title, const QString &captureId);
        void cancelCaptureDownload();
        bool watchCapture(const QUrl &page, const QString &title, const QString &captureId);
        void cancelCaptureWatch();

    signals:
        void torrentReady(const QString &magnet);
        void torrentMetadataReady(const QString &identity, const QByteArray &data);
        void videoReady(quint64 launchToken, const QString &identity, const QUrl &url,
            const QUrl &proxyUrl, const QString &title, const QString &contentKind,
            const QUrl &audioUrl, const QString &audioTitle, const QString &audioType,
            const QString &audioLanguage, const QString &audioCodec, int audioChannels,
            const QVector<QUrl> &subtitles);
        void readerActiveChanged();
        void searchFinished(bool failed);
        void browserReply(quint64 generation, quint64 id, const QByteArray &reply);
        void sharedLinkFailed(bool malformed);
        void downloadQueued();
        void capturedDownloadFinished(const QString &captureId, const QString &reason);
        void capturedWatchFinished(const QString &captureId, const QString &reason);

    private slots:
        void openDownloadedFolder(const QString &path);

    private:
        void changeEvent(QEvent *event) override;
        bool eventFilter(QObject *watched, QEvent *event) override;

        Qbutt::ContentController *m_controller = nullptr;
        ReaderImageProvider *m_readerImages = nullptr; // QQmlEngine owns the provider.
        QHash<quint64, quint64> m_mediaRequests;
    };
}
