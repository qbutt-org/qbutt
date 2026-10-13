// SPDX-License-Identifier: GPL-2.0-or-later
#include "streamserver.h"

#include <algorithm>

#include <QCoreApplication>
#include <QElapsedTimer>
#include <QMimeDatabase>
#include <QTcpSocket>
#include <QTimer>
#include <QUuid>
#include <QUrlQuery>

#include "base/bittorrent/torrentimpl.h"
#include "base/global.h"

namespace
{
    constexpr qint64 MAX_BUFFERED_BYTES = 256 * 1024;
    constexpr qint64 MAX_PIECE_BYTES = 32 * 1024 * 1024;
    constexpr qint64 READ_AHEAD_BYTES = 4 * 1024 * 1024;
    constexpr int MAX_CONNECTIONS = 8;

    QByteArray streamToken(const QUrl &url)
    {
        const QStringList parts = url.path().split(u'/');
        return ((parts.size() >= 4) && (parts[1] == u"media")) ? parts[2].toLatin1() : QByteArray();
    }
}

namespace Media
{
    class StreamConnection final : public QObject
    {
    public:
        StreamConnection(StreamServer *server, const qintptr descriptor)
            : QObject(server)
            , m_server {server}
            , m_socket {new QTcpSocket(this)}
        {
            m_socket->setReadBufferSize(16 * 1024);
            if (!m_socket->setSocketDescriptor(descriptor))
            {
                deleteLater();
                return;
            }
            connect(m_socket, &QTcpSocket::readyRead, this, [this] { readRequest(); });
            connect(m_socket, &QTcpSocket::bytesWritten, this, [this] { pump(); });
            connect(m_socket, &QTcpSocket::disconnected, this, [this]
            {
                release();
                deleteLater();
            });
            m_progress.start();
            auto *timer = new QTimer(this);
            timer->setInterval(1000);
            connect(timer, &QTimer::timeout, this, [this]
            {
                if (m_progress.hasExpired(60000))
                    m_socket->abort();
            });
            timer->start();
        }

        ~StreamConnection() override
        {
            release();
            m_server->m_connections.remove(this);
        }

        QByteArray token() const { return m_token; }
        void close() { m_socket->abort(); }

    private:
        void release()
        {
            if (m_torrent)
                m_torrent->clearStreamingRange(this);
            m_torrent = nullptr;
            m_pieceData.clear();
        }

        void respond(const int status, const QByteArray &reason, const qint64 length = 0, const QByteArray &extraHeaders = {})
        {
            m_socket->write("HTTP/1.1 " + QByteArray::number(status) + " " + reason + "\r\n"
                "Connection: close\r\nCache-Control: no-store\r\n"
                "X-Content-Type-Options: nosniff\r\nAccept-Ranges: bytes\r\n"
                "Content-Length: " + QByteArray::number(length) + "\r\n" + extraHeaders + "\r\n");
        }

        void fail(const int status, const QByteArray &reason, const QByteArray &headers = {})
        {
            respond(status, reason, 0, headers);
            m_socket->disconnectFromHost();
        }

        void readRequest()
        {
            if (m_started)
            {
                m_socket->readAll();
                return;
            }
            m_request += m_socket->readAll();
            if (m_request.size() > 16 * 1024)
            {
                fail(431, "Request Header Fields Too Large");
                return;
            }
            const qsizetype headerEnd = m_request.indexOf("\r\n\r\n");
            if (headerEnd < 0)
                return;
            m_started = true;
            const auto lines = m_request.left(headerEnd).split('\n');
            const auto first = lines.first().trimmed().split(' ');
            if ((first.size() != 3) || ((first[0] != "GET") && (first[0] != "HEAD")))
            {
                fail(405, "Method Not Allowed", "Allow: GET, HEAD\r\n");
                return;
            }
            QHash<QByteArray, QByteArray> headers;
            for (qsizetype i = 1; i < lines.size(); ++i)
            {
                const auto separator = lines[i].indexOf(':');
                if (separator <= 0)
                {
                    fail(400, "Bad Request");
                    return;
                }
                const auto name = lines[i].left(separator).trimmed().toLower();
                if (headers.contains(name))
                {
                    fail(400, "Bad Request");
                    return;
                }
                headers.insert(name, lines[i].mid(separator + 1).trimmed());
            }
            const QByteArray authority = "127.0.0.1:" + QByteArray::number(m_server->serverPort());
            if ((headers.value("host") != authority) || headers.contains("origin")
                || headers.contains("content-length") || headers.contains("transfer-encoding"))
            {
                fail(403, "Forbidden");
                return;
            }
            const QUrl url = QUrl::fromEncoded(first[1]);
            m_token = streamToken(url);
            const auto stream = m_server->m_streams.constFind(m_token);
            if ((stream == m_server->m_streams.cend()) || !stream->torrent)
            {
                fail(404, "Not Found");
                return;
            }
            m_torrent = stream->torrent;
            m_fileIndex = stream->fileIndex;
            const qint64 size = m_torrent->fileSize(m_fileIndex);
            m_position = 0;
            m_end = size - 1;
            if (m_torrent->hasExclusiveFileOperation() || m_torrent->isChecking())
            {
                fail(409, "Conflict");
                return;
            }

            // The verified file is immutable, even when its playback token changes.
            const BitTorrent::InfoHash infoHash = m_torrent->infoHash();
            const QString torrentHash = infoHash.v2().isValid() ? infoHash.v2().toString() : infoHash.v1().toString();
            const QByteArray entityTag = "\"torrent-" + torrentHash.toLatin1()
                + '-' + QByteArray::number(m_fileIndex) + '-' + QByteArray::number(size) + '"';
            if (headers.contains("if-match"))
            {
                const QByteArray condition = headers.value("if-match");
                const auto candidates = condition.split(',');
                if ((condition != "*") && std::none_of(candidates.cbegin(), candidates.cend()
                    , [&entityTag](const QByteArray &candidate) { return candidate.trimmed() == entityTag; }))
                {
                    fail(412, "Precondition Failed");
                    return;
                }
            }

            bool partial = false;
            if (headers.contains("range") && (first[0] != "HEAD"))
            {
                const QByteArray range = headers.value("range");
                if (!range.startsWith("bytes=") || range.contains(','))
                {
                    fail(400, "Bad Request");
                    return;
                }
                const auto bounds = range.mid(6).split('-');
                bool firstOk = false;
                bool lastOk = false;
                if (bounds.size() == 2)
                {
                    if (bounds[0].isEmpty())
                    {
                        const qint64 suffix = bounds[1].toLongLong(&lastOk);
                        firstOk = lastOk && (suffix > 0);
                        m_position = std::max<qint64>(0, size - suffix);
                    }
                    else
                    {
                        m_position = bounds[0].toLongLong(&firstOk);
                        m_end = bounds[1].isEmpty() ? m_end : bounds[1].toLongLong(&lastOk);
                        lastOk = bounds[1].isEmpty() || lastOk;
                    }
                }
                if (!firstOk || !lastOk || (m_position < 0) || (m_position >= size) || (m_end < m_position))
                {
                    fail(416, "Range Not Satisfiable", "Content-Range: bytes */" + QByteArray::number(size) + "\r\n");
                    return;
                }
                m_end = std::min(m_end, size - 1);
                partial = true;
            }
            QByteArray extra = "Content-Type: " + QMimeDatabase().mimeTypeForFile(m_torrent->filePath(m_fileIndex).toString()
                , QMimeDatabase::MatchExtension).name().toLatin1() + "\r\n";
            extra += "ETag: " + entityTag + "\r\n";
            if (partial)
            {
                extra += "Content-Range: bytes " + QByteArray::number(m_position) + "-"
                    + QByteArray::number(m_end) + "/" + QByteArray::number(size) + "\r\n";
            }
            respond(partial ? 206 : 200, partial ? "Partial Content" : "OK", m_end - m_position + 1, extra);
            m_request.clear();
            if ((first[0] == "HEAD") || (size == 0))
            {
                m_socket->disconnectFromHost();
                return;
            }
            m_urgent = QUrlQuery(url).queryItemValue(u"role"_s) != u"probe";
            connect(m_torrent, &BitTorrent::TorrentImpl::streamingPieceReady, this
                , [this](const int piece, const QByteArray &data, const QString &error)
            {
                if (piece != m_waitingPiece)
                    return;
                m_waitingPiece = -1;
                if (!error.isEmpty() || data.isEmpty())
                {
                    emit m_server->error(error.isEmpty() ? tr("The requested video data could not be read.") : error);
                    m_socket->abort();
                    return;
                }
                m_piece = piece;
                m_pieceData = data;
                m_progress.restart();
                pump();
            });
            connect(m_torrent, &QObject::destroyed, this, [this] { m_socket->abort(); });
            connect(m_torrent, &BitTorrent::TorrentImpl::streamingStopped, this, [this] { m_socket->abort(); });
            m_body = true;
            pump();
        }

        void pump()
        {
            if (!m_body || !m_torrent || (m_waitingPiece >= 0)
                || (m_socket->state() != QAbstractSocket::ConnectedState))
                return;
            while ((m_position <= m_end) && (m_socket->bytesToWrite() < MAX_BUFFERED_BYTES))
            {
                if (m_torrent->hasExclusiveFileOperation() || m_torrent->isChecking())
                {
                    m_socket->abort();
                    break;
                }
                const qint64 absolute = m_torrent->info().fileOffset(m_fileIndex) + m_position;
                const int piece = absolute / m_torrent->pieceLength();
                if (piece != m_piece)
                {
                    m_pieceData.clear();
                    m_waitingPiece = piece;
                    m_torrent->setStreamingRange(this, m_fileIndex, m_position
                        , std::min(m_end - m_position + 1, std::max<qint64>(READ_AHEAD_BYTES, 4 * m_torrent->pieceLength())), m_urgent);
                    m_torrent->requestStreamingPiece(piece);
                    break;
                }
                const qint64 pieceOffset = absolute % m_torrent->pieceLength();
                const qint64 count = std::min<qint64>({64 * 1024, m_end - m_position + 1, m_pieceData.size() - pieceOffset});
                if (count <= 0)
                {
                    m_socket->abort();
                    break;
                }
                const qint64 written = m_socket->write(m_pieceData.constData() + pieceOffset, count);
                if (written <= 0)
                    break;
                m_position += written;
                m_progress.restart();
            }
            if (m_position > m_end)
            {
                m_body = false;
                release();
                m_socket->disconnectFromHost();
            }
        }

        StreamServer *m_server = nullptr;
        QTcpSocket *m_socket = nullptr;
        QPointer<BitTorrent::TorrentImpl> m_torrent;
        QByteArray m_token;
        QByteArray m_request;
        QByteArray m_pieceData;
        QElapsedTimer m_progress;
        qint64 m_position = 0;
        qint64 m_end = -1;
        int m_fileIndex = 0;
        int m_piece = -1;
        int m_waitingPiece = -1;
        bool m_started = false;
        bool m_body = false;
        bool m_urgent = true;
    };

    StreamServer *StreamServer::instance()
    {
        static QPointer<StreamServer> server;
        if (!server)
            server = new StreamServer(QCoreApplication::instance());
        return server;
    }

    StreamServer::StreamServer(QObject *parent)
        : QTcpServer(parent)
    {
    }

    StreamServer::~StreamServer()
    {
        const auto tokens = m_streams.keys();
        for (const auto &token : tokens)
        {
            QUrl url;
            url.setPath(u"/media/"_s + QString::fromLatin1(token) + u"/file"_s);
            closeStream(url);
        }
        const auto connections = m_connections.values();
        qDeleteAll(connections);
    }

    QUrl StreamServer::openTorrent(BitTorrent::Torrent *torrent, const int fileIndex)
    {
        auto *implementation = qobject_cast<BitTorrent::TorrentImpl *>(torrent);
        if (m_streams.size() >= MAX_CONNECTIONS)
        {
            emit error(tr("Close an existing video before opening another one."));
            return {};
        }
        if (!implementation || !implementation->hasMetadata() || (fileIndex < 0) || (fileIndex >= implementation->filesCount())
            || implementation->hasExclusiveFileOperation() || implementation->isChecking() || implementation->hasError()
            || (implementation->fileSize(fileIndex) <= 0) || (implementation->pieceLength() > MAX_PIECE_BYTES))
        {
            emit error(tr("This file is not ready for playback. Wait for the torrent's metadata and file operations to finish."));
            return {};
        }
        if (!isListening() && !listen(QHostAddress::LocalHost, 0))
        {
            emit error(tr("Could not open the local video stream: %1").arg(errorString()));
            return {};
        }
        const QByteArray token = QUuid::createUuid().toByteArray(QUuid::Id128);
        const auto revoke = [this, token]
        {
            QUrl url;
            url.setPath(u"/media/"_s + QString::fromLatin1(token) + u"/file"_s);
            closeStream(url);
        };
        const auto lifetime = connect(implementation, &QObject::destroyed, this, revoke);
        const auto stop = connect(implementation, &BitTorrent::TorrentImpl::streamingStopped, this, revoke);
        implementation->acquireStreamingLease();
        m_streams.insert(token, {implementation, fileIndex, lifetime, stop});
        if (implementation->isStopped())
            implementation->start();
        QUrl url;
        url.setScheme(u"http"_s);
        url.setHost(u"127.0.0.1"_s);
        url.setPort(serverPort());
        url.setPath(u"/media/"_s + QString::fromLatin1(token) + u'/' + implementation->filePath(fileIndex).filename());
        return url;
    }

    void StreamServer::closeStream(const QUrl &url)
    {
        const auto token = streamToken(url);
        const auto stream = m_streams.find(token);
        if (stream == m_streams.end())
            return;
        disconnect(stream->lifetimeConnection);
        disconnect(stream->stopConnection);
        if (stream->torrent)
            stream->torrent->releaseStreamingLease();
        m_streams.erase(stream);
        const auto connections = m_connections.values();
        for (StreamConnection *connection : connections)
        {
            if (connection->token() == token)
                connection->close();
        }
        if (m_streams.isEmpty())
            close();
    }

    void StreamServer::incomingConnection(const qintptr socketDescriptor)
    {
        auto *connection = new StreamConnection(this, socketDescriptor);
        m_connections.insert(connection);
        if (m_connections.size() > MAX_CONNECTIONS)
            connection->close();
    }
}
