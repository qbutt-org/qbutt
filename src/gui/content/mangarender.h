/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <functional>

#include <QColor>
#include <QFont>
#include <QImage>
#include <QList>
#include <QPainterPath>
#include <QRectF>
#include <QSize>
#include <QString>

namespace Content::MangaRender
{
    enum class Status
    {
        Rendered,
        EmptyTranslation,
        InvalidGeometry,
        Overflow,
        MissingGlyph
    };

    enum class FontFamily
    {
        Comic,
        Serif,
        Condensed
    };

    struct TextRun
    {
        QString text;
        bool bold = false;
        bool italic = false;
    };

    struct Region
    {
        QString id;
        QList<TextRun> translation;
        FontFamily family = FontFamily::Comic;
        qreal angle = 0;
        QColor textColor;
        QColor outlineColor;
        qreal outlineWidth = 0;
        QRectF bounds;
        QRectF layout;
        QList<QRectF> cleanupRects;
    };

    struct Diagnostic
    {
        QString regionId;
        Status status = Status::Rendered;
        int fontPixels = 0;
        int layoutLines = 0;
        int cleanupPixels = 0;
    };

    struct TextShape
    {
        QPainterPath path;
        int fontPixels = 0;
        int lines = 0;
        Status status = Status::Rendered;
    };

    struct Typography
    {
        QList<TextShape> shapes;
        QList<Diagnostic> diagnostics;
        QString error;
    };

    struct Result
    {
        QImage cleaned;
        QImage rendered;
        QList<Diagnostic> diagnostics;
        QString error;
    };

    struct NormalizedPage
    {
        QImage image;
        QString error;
    };

    bool supportsImageSize(const QSize &size);
    NormalizedPage normalizeGeneratedPage(
        const QSize &originalSize, const QImage &generated, const std::function<bool()> &cancelled = { });
    QFont mangaFont(FontFamily family);
    TextShape typesetText(
        const QList<TextRun> &runs, const QRectF &rect, QFont font, int maximumPixels, qreal angle = 0);
    Typography prepareTypography(
        const QSize &size, const QList<Region> &regions, const std::function<bool()> &cancelled = { });
    Result renderCleaned(const QImage &original, const QImage &generated, const QList<Region> &regions,
        const std::function<bool()> &cancelled = { });
}
