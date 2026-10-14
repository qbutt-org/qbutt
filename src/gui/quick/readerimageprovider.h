// SPDX-License-Identifier: GPL-2.0-or-later
#pragma once

#include <QHash>
#include <QMutex>
#include <QQuickImageProvider>
#include <QReadWriteLock>

namespace Content
{
    class ReaderImageProvider final : public QQuickImageProvider
    {
    public:
        ReaderImageProvider();
        void clear(quint64 generation);
        void add(quint64 generation, const QString &id, const QByteArray &bytes);
        QImage requestImage(const QString &id, QSize *size, const QSize &requestedSize) override;

    private:
        QImage requestTile(const QString &id, QSize *size);

        QReadWriteLock m_lock;
        quint64 m_generation = 0;
        QHash<QString, QByteArray> m_assets;
        QMutex m_decodeLock;
        QString m_originalPath;
        QImage m_original;
    };
}
