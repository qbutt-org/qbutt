/*
 * Copyright (C) 2026 qbutt contributors
 * SPDX-License-Identifier: GPL-2.0-or-later
 */

#pragma once

#include <functional>

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
        NoInkMask
    };

    struct Region
    {
        QString id;
        QString translation;
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
    };

    struct Result
    {
        QImage mask;
        QImage cleaned;
        QImage rendered;
        QList<Diagnostic> diagnostics;
        QString error;
    };

    QFont mangaFont();
    TextShape typesetText(QString text, const QRectF &rect, QFont font, int maximumPixels);
    Result render(const QImage &original, const QList<Region> &regions,
        const std::function<bool()> &cancelled = {});
}
