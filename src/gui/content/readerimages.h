/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <QAbstractScrollArea>
#include <QImage>
#include <QJsonArray>
#include <QJsonObject>
#include <QList>
#include <QRect>
#include <QString>

#include "base/content/readersettings.h"

class QContextMenuEvent;
class QKeyEvent;
class QMouseEvent;
class QPaintEvent;
class QResizeEvent;

namespace Content
{
    class ImagePages final : public QAbstractScrollArea
    {
        Q_OBJECT
        Q_PROPERTY(int imageIndex READ imageIndex)
        Q_PROPERTY(QRect imageRect READ imageRect)

    public:
        explicit ImagePages(QWidget *parent);

        void setPage(const QJsonObject &page, int imageIndex, bool original);
        void setSettings(const ReaderSettings &settings);
        int imageIndex() const;
        QRect imageRect() const;

    signals:
        void pageTurnRequested(int delta);
        void imageIndexChanged(int imageIndex);
        void correctionRequested(const QJsonObject &region);

    private:
        struct PageImage
        {
            QString path;
            QSize size;
            QRect rect;
            QImage image;
        };

        void updateLayout(double imageOffset = 0);
        double imageOffset() const;
        QRectF regionRect(const QJsonObject &region, const QRect &imageRect) const;
        QJsonObject regionAt(const QPointF &position) const;

        void paintEvent(QPaintEvent *event) override;
        void resizeEvent(QResizeEvent *event) override;
        void mouseReleaseEvent(QMouseEvent *event) override;
        void mouseMoveEvent(QMouseEvent *event) override;
        void keyPressEvent(QKeyEvent *event) override;
        void contextMenuEvent(QContextMenuEvent *event) override;
        void scrollContentsBy(int dx, int dy) override;

        int m_imageIndex = 0;
        QString m_pageId;
        QList<PageImage> m_images;
        QJsonArray m_regions;
        ReaderSettings m_settings;
        bool m_layingOut = false;
    };
}
