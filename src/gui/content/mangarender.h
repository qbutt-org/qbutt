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
#include <QString>

namespace Content::MangaRender
{
    enum class Ink
    {
        Dark,
        Light
    };

    enum class Status
    {
        Rendered,
        EmptyTranslation,
        InvalidGeometry,
        Overflow,
        NoInkMask,
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
        QRectF bounds;
        QRectF layout;
        QList<QRectF> inkBoxes;
        Ink ink = Ink::Dark;
    };

    struct Diagnostic
    {
        QString regionId;
        Status status = Status::Rendered;
        int fontPixels = 0;
        int layoutLines = 0;
        int maskedPixels = 0;
    };

    struct TextShape
    {
        QPainterPath path;
        int fontPixels = 0;
        int lines = 0;
        Status status = Status::Rendered;
    };

    struct Result
    {
        QImage mask;
        QImage cleaned;
        QImage rendered;
        QList<Diagnostic> diagnostics;
        QString error;
    };

    struct Crop
    {
        QImage image;
        QRectF bounds;
        QString error;
    };

    struct NormalizedPage
    {
        QImage image;
        QString error;
    };

    NormalizedPage normalizeGeneratedPage(const QSize &originalSize, const QImage &generated,
        const std::function<bool()> &cancelled = {});
    Crop cropPage(const QImage &image, const QRectF &bounds,
        const std::function<bool()> &cancelled = {});
    QFont mangaFont(FontFamily family);
    TextShape typesetText(const QList<TextRun> &runs, const QRectF &rect, QFont font,
        int maximumPixels, qreal angle = 0);
    Result render(const QImage &original, const QList<Region> &regions,
        const std::function<bool()> &cancelled = {});
}
