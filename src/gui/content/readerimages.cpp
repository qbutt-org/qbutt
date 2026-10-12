/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#include "readerimages.h"

#include <utility>

#include <QAction>
#include <QContextMenuEvent>
#include <QCoreApplication>
#include <QDir>
#include <QImageReader>
#include <QKeyEvent>
#include <QMenu>
#include <QMouseEvent>
#include <QPainter>
#include <QPainterPath>
#include <QPaintEvent>
#include <QPen>
#include <QResizeEvent>
#include <QScopedValueRollback>
#include <QScrollBar>
#include <QUrl>

#include "base/global.h"
#include "mangarender.h"

Content::ImagePages::ImagePages(QWidget *parent)
    : QAbstractScrollArea(parent)
{
    setFont(MangaRender::mangaFont());
    setFrameShape(QFrame::NoFrame);
    viewport()->setCursor(Qt::PointingHandCursor);
    viewport()->setMouseTracking(true);
}

void Content::ImagePages::setPage(const QJsonObject &page, const int imageIndex, const double offset, const bool original)
{
    const QJsonArray images = page.value(u"images"_s).toArray();
    const int last = qMax(0, static_cast<int>(images.size()) - 1);
    const int selected = (imageIndex < 0) ? last : qBound(0, imageIndex, last);
    QList<PageImage> pages;
    pages.reserve(images.size());
    for (qsizetype index = 0; index < images.size(); ++index)
    {
        const QJsonObject image = images.at(index).toObject();
        const QString cleaned = image.value(u"cleanedUrl"_s).toString();
        const QString source = original || cleaned.isEmpty() ? image.value(u"url"_s).toString() : cleaned;
        const QUrl url(source);
        const QString path = url.isLocalFile() ? url.toLocalFile() : source;
        PageImage entry;
        entry.path = QDir::isAbsolutePath(path) ? path : QString();
        if ((index < m_images.size()) && (entry.path == m_images.at(index).path))
        {
            entry.size = m_images.at(index).size;
            entry.image = m_images.at(index).image;
        }
        else
        {
            entry.size = QImageReader(entry.path).size();
        }
        pages.append(std::move(entry));
    }
    m_imageIndex = selected;
    m_images = std::move(pages);
    m_regions = original ? QJsonArray() : page.value(u"regions"_s).toArray();
    updateLayout(offset);
}

void Content::ImagePages::setSettings(const ReaderSettings &settings)
{
    if (settings == m_settings)
        return;

    const double offset = imageOffset();
    m_settings = settings;
    viewport()->setCursor(settings.mode == ReaderMode::Paged ? Qt::PointingHandCursor : Qt::ArrowCursor);
    updateLayout(offset);
}

int Content::ImagePages::imageIndex() const
{
    return m_imageIndex;
}

QRect Content::ImagePages::imageRect() const
{
    return m_images.isEmpty() ? QRect() : m_images.at(m_imageIndex).rect.translated(0, -verticalScrollBar()->value());
}

double Content::ImagePages::imageOffset() const
{
    if (m_images.isEmpty() || (verticalScrollBar()->maximum() == 0))
        return 0;

    const QRect rect = m_images.at(m_imageIndex).rect;
    return (rect.height() > 0) ? double(verticalScrollBar()->value() + 8 - rect.y()) / rect.height() : 0;
}

void Content::ImagePages::updateLayout(const double imageOffset)
{
    if (m_layingOut)
        return;

    const QScopedValueRollback layingOut(m_layingOut, true);
    const int availableWidth = qMax(1, (viewport()->width() - 16) * m_settings.contentWidth / 100);
    const int availableHeight = qMax(1, viewport()->height() - 16);
    int top = 8;
    for (qsizetype index = 0; index < m_images.size(); ++index)
    {
        PageImage &image = m_images[index];
        image.rect = {};
        if ((m_settings.mode == ReaderMode::Paged) && (index != m_imageIndex))
            continue;

        QSize size(availableWidth, availableHeight);
        if (image.size.isValid())
        {
            qreal scale = double(availableWidth) / image.size.width();
            if (m_settings.imageFit == ImageFit::Height)
                scale = qMin(scale, double(availableHeight) / image.size.height());
            if (!m_settings.enlargeSmallImages)
                scale = qMin(scale, qreal(1));
            size = QSize(qMax(1, qRound(image.size.width() * scale)), qMax(1, qRound(image.size.height() * scale)));
        }
        if (m_settings.mode == ReaderMode::Paged)
            top = qMax(8, (viewport()->height() - size.height()) / 2);
        image.rect = QRect((viewport()->width() - size.width()) / 2, top, size.width(), size.height());
        top = image.rect.bottom() + 1 + m_settings.pageGap;
    }
    const int contentHeight = m_images.isEmpty() ? 0 : top - m_settings.pageGap + 8;
    verticalScrollBar()->setPageStep(viewport()->height());
    verticalScrollBar()->setSingleStep(40);
    verticalScrollBar()->setRange(0, qMax(0, contentHeight - viewport()->height()));
    if (!m_images.isEmpty())
    {
        const QRect rect = m_images.at(m_imageIndex).rect;
        verticalScrollBar()->setValue(rect.y() - 8 + qRound(imageOffset * rect.height()));
    }
    viewport()->update();
}

QRectF Content::ImagePages::regionRect(const QJsonObject &region, const QRect &imageRect) const
{
    const QJsonObject layout = region.value(u"layout"_s).toObject();
    const QRectF rect(imageRect.x() + (layout.value(u"x"_s).toDouble() * imageRect.width()),
        imageRect.y() + (layout.value(u"y"_s).toDouble() * imageRect.height()),
        layout.value(u"width"_s).toDouble() * imageRect.width(),
        layout.value(u"height"_s).toDouble() * imageRect.height());
    return rect.intersected(imageRect);
}

QJsonObject Content::ImagePages::regionAt(const QPointF &position) const
{
    for (const QJsonValue &value : m_regions)
    {
        const QJsonObject region = value.toObject();
        const int index = region.value(u"imageIndex"_s).toInt();
        if ((index < 0) || (index >= m_images.size()))
            continue;
        const QRect rect = m_images.at(index).rect.translated(0, -verticalScrollBar()->value());
        if (regionRect(region, rect).contains(position))
            return region;
    }
    return {};
}

void Content::ImagePages::paintEvent(QPaintEvent *event)
{
    QPainter painter(viewport());
    painter.fillRect(event->rect(), palette().window());
    painter.setRenderHints(QPainter::SmoothPixmapTransform | QPainter::Antialiasing);
    if (m_images.isEmpty())
    {
        painter.setPen(palette().text().color());
        painter.drawText(viewport()->rect(), Qt::AlignCenter,
            QCoreApplication::translate("Content::ReaderWidget", "Image unavailable"));
        return;
    }
    for (qsizetype index = 0; index < m_images.size(); ++index)
    {
        PageImage &image = m_images[index];
        const QRect rect = image.rect.translated(0, -verticalScrollBar()->value());
        if (!rect.intersects(viewport()->rect()))
        {
            image.image = {};
            continue;
        }
        if (image.image.isNull())
            image.image = QImageReader(image.path).read();
        if (image.image.isNull())
        {
            painter.setPen(palette().text().color());
            painter.drawText(rect, Qt::AlignCenter,
                QCoreApplication::translate("Content::ReaderWidget", "Image unavailable"));
            continue;
        }
        painter.drawImage(rect, image.image);
        const qreal scale = double(rect.width()) / image.image.width();
        for (const QJsonValue &value : m_regions)
        {
            const QJsonObject region = value.toObject();
            if (region.value(u"imageIndex"_s).toInt() != index)
                continue;
            const QRectF regionArea = regionRect(region, rect);
            const QString text = region.value(u"translation"_s).toString();
            if (text.isEmpty() || regionArea.isEmpty() || !regionArea.intersects(event->rect()))
                continue;

            const qreal margin = qMin(qMax(qreal(1), 3 * scale), qMin(regionArea.width(), regionArea.height()) / 4);
            const QRectF textRect = regionArea.adjusted(margin, margin, -margin, -margin);
            const int maximumPixels = qMax(1, qRound(qMin(textRect.height(), 28 * scale)));
            const QPainterPath path = MangaRender::typesetText(text, textRect, font(), maximumPixels).path;
            const bool light = region.value(u"ink"_s).toString() == u"light";
            painter.strokePath(path, QPen(light ? Qt::black : Qt::white, qMax(qreal(0.75), 1.5 * scale),
                Qt::SolidLine, Qt::RoundCap, Qt::RoundJoin));
            painter.fillPath(path, light ? Qt::white : Qt::black);
        }
        if (m_settings.brightness < 100)
            painter.fillRect(rect, QColor(0, 0, 0, 255 * (100 - m_settings.brightness) / 100));
    }
}

void Content::ImagePages::resizeEvent(QResizeEvent *event)
{
    const double offset = imageOffset();
    QAbstractScrollArea::resizeEvent(event);
    updateLayout(offset);
}

void Content::ImagePages::mouseReleaseEvent(QMouseEvent *event)
{
    if (event->button() == Qt::LeftButton)
    {
        setFocus(Qt::MouseFocusReason);
        if (m_settings.mode == ReaderMode::Paged)
            emit pageTurnRequested(event->position().x() < (viewport()->width() / 2.0) ? -1 : 1);
        event->accept();
        return;
    }
    QAbstractScrollArea::mouseReleaseEvent(event);
}

void Content::ImagePages::mouseMoveEvent(QMouseEvent *event)
{
    viewport()->setToolTip(regionAt(event->position()).isEmpty() ? QString()
        : QCoreApplication::translate("Content::ReaderWidget", "Right-click to correct the translation"));
    QAbstractScrollArea::mouseMoveEvent(event);
}

void Content::ImagePages::keyPressEvent(QKeyEvent *event)
{
    const bool paged = m_settings.mode == ReaderMode::Paged;
    if ((event->key() == Qt::Key_Left) || (paged && (event->key() == Qt::Key_PageUp)))
        emit pageTurnRequested(-1);
    else if ((event->key() == Qt::Key_Right) || (paged && (event->key() == Qt::Key_PageDown)))
        emit pageTurnRequested(1);
    else
    {
        QAbstractScrollArea::keyPressEvent(event);
        return;
    }
    event->accept();
}

void Content::ImagePages::contextMenuEvent(QContextMenuEvent *event)
{
    const QPoint position = event->pos();
    const QJsonObject region = regionAt(position);
    if (region.isEmpty())
        return;

    QMenu menu(this);
    QAction *correct = menu.addAction(QCoreApplication::translate("Content::ReaderWidget", "Correct translation"));
    correct->setObjectName(u"readerCorrectTranslation"_s);
    if (menu.exec(viewport()->mapToGlobal(position)) == correct)
        emit correctionRequested(region);
}

void Content::ImagePages::scrollContentsBy(int, int)
{
    viewport()->update();
    if (m_layingOut || m_images.isEmpty())
        return;

    if (m_settings.mode == ReaderMode::Continuous)
    {
        const int top = verticalScrollBar()->value() + 8;
        m_imageIndex = static_cast<int>(m_images.size()) - 1;
        if (verticalScrollBar()->value() < verticalScrollBar()->maximum())
        {
            for (int candidate = 0; candidate < m_images.size(); ++candidate)
            {
                if (m_images.at(candidate).rect.bottom() >= top)
                {
                    m_imageIndex = candidate;
                    break;
                }
            }
        }
    }
    emit positionChanged();
}
