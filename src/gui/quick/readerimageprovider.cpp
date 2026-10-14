// SPDX-License-Identifier: GPL-2.0-or-later
#include "readerimageprovider.h"

#include <QBuffer>
#include <QImageReader>
#include <QUrl>

#include "qbutt_quick/native.h"

Content::ReaderImageProvider::ReaderImageProvider()
    : QQuickImageProvider(QQuickImageProvider::Image, QQuickImageProvider::ForceAsynchronousImageLoading)
{
}

void Content::ReaderImageProvider::clear(const quint64 generation)
{
    QWriteLocker guard(&m_lock);
    m_generation = generation;
    m_assets.clear();
    m_originalPath.clear();
    m_original = {};
}

void Content::ReaderImageProvider::add(const quint64 generation, const QString &id, const QByteArray &bytes)
{
    QWriteLocker guard(&m_lock);
    if (generation == m_generation && id.size() == 64 && bytes.size() <= 8 * 1024 * 1024)
        m_assets.insert(id, bytes);
}

QImage Content::ReaderImageProvider::requestImage(const QString &id, QSize *size, const QSize &requestedSize)
{
    if (id.startsWith(QLatin1String("tile/")))
        return requestTile(id, size);

    QByteArray data;
    const auto separator = id.indexOf(QLatin1Char('/'));
    if (separator < 0)
        return {};
    {
        QReadLocker guard(&m_lock);
        if (id.left(separator).toULongLong() != m_generation)
            return {};
        data = m_assets.value(id.mid(separator + 1));
    }
    if (data.isEmpty())
        return {};

    QBuffer buffer(&data);
    buffer.open(QIODevice::ReadOnly);
    QImageReader reader(&buffer);
    const QSize original = reader.size();
    if (!original.isValid())
        return {};
    if (size)
        *size = original;
    QSize bounds(4096, 4096);
    if (requestedSize.isValid())
        bounds = QSize(qMin(bounds.width(), requestedSize.width()),
            qMin(bounds.height(), requestedSize.height()));
    const QSize decoded = original.width() > bounds.width() || original.height() > bounds.height()
        ? original.scaled(bounds, Qt::KeepAspectRatio) : original;
    if (decoded != original)
        reader.setScaledSize(decoded);
    return reader.read();
}

QImage Content::ReaderImageProvider::requestTile(const QString &id, QSize *size)
{
    quint64 generation;
    {
        QReadLocker guard(&m_lock);
        generation = m_generation;
    }
    const QStringList parts = id.split(QLatin1Char('/'));
    if (parts.size() != 4)
        return {};

    bool validTop = false;
    bool validHeight = false;
    const int top = parts[2].toInt(&validTop);
    const int height = parts[3].toInt(&validHeight);
    const QUrl url(QUrl::fromPercentEncoding(parts[1].toUtf8()));
    if (!validTop || !validHeight || !url.isLocalFile()
        || (top < 0) || (height <= 0) || (height > 2048))
    {
        return {};
    }
    const QString path = url.toLocalFile();
    QImage original;
    QMutexLocker decodeGuard(&m_decodeLock);
    {
        QReadLocker guard(&m_lock);
        if (generation != m_generation)
            return {};
        if (path == m_originalPath)
            original = m_original;
    }
    if (original.isNull())
    {
        original = Qbutt::readReaderImage(path).convertToFormat(QImage::Format_ARGB32_Premultiplied);
        if (original.isNull())
            return {};
        QWriteLocker guard(&m_lock);
        if (generation != m_generation)
            return {};
        m_originalPath = path;
        m_original = original;
    }
    if ((top >= original.height()) || (height > original.height() - top)
        || (qint64(original.width()) * height > 2'000'000))
    {
        return {};
    }
    const QImage tile = original.copy(0, top, original.width(), height);
    {
        QReadLocker guard(&m_lock);
        if (generation != m_generation)
            return {};
    }
    if (size)
        *size = tile.size();
    return tile;
}
