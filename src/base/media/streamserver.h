// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QHash>
#include <QPointer>
#include <QSet>
#include <QTcpServer>
#include <QUrl>

namespace BitTorrent
{
    class Torrent;
    class TorrentImpl;
}

namespace Media
{
    class StreamConnection;

    // An authenticated, local reader of the existing libtorrent session. It
    // never opens payload files or introduces a second downloader/writer.
    class StreamServer final : public QTcpServer
    {
        Q_OBJECT
        Q_DISABLE_COPY_MOVE(StreamServer)

    public:
        static StreamServer *instance();
        ~StreamServer() override;
        QUrl openTorrent(BitTorrent::Torrent *torrent, int fileIndex);
        void closeStream(const QUrl &url);

    signals:
        void error(const QString &message);

    private:
        friend class StreamConnection;
        explicit StreamServer(QObject *parent);
        void incomingConnection(qintptr socketDescriptor) override;

        struct Stream
        {
            QPointer<BitTorrent::TorrentImpl> torrent;
            int fileIndex = 0;
            QMetaObject::Connection lifetimeConnection;
            QMetaObject::Connection stopConnection;
        };
        QHash<QByteArray, Stream> m_streams;
        QSet<StreamConnection *> m_connections;
    };
}
